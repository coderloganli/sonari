//! `VoiceRuntimeExecutionPort` over locally loaded models.
//!
//! Engines are process-wide: a model is loaded once and shared. Per-call state
//! lives in the streams this opens, keyed by the session id the caller supplies.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use crate::ports::{
    AsrEngine, AsrEvent, AsrStream, AsrStreamConfig, CloseRuntimeAsrSessionRequest,
    CommitRuntimeAsrSessionRequest, OpenRuntimeAsrSessionRequest, OpenRuntimeAsrSessionResult,
    PollRuntimeAsrEventsRequest, PollRuntimeAsrEventsResult, PushRuntimeAsrAudioRequest,
    RuntimeAsrEvent, RuntimeTtsExecutionRequest, RuntimeTtsExecutionStream, TtsEngine, TtsRequest,
    VoiceRuntimeExecutionPort,
};

struct AsrSession {
    stream: Box<dyn AsrStream>,
    /// The round a transcript belongs to. Set when the round is committed; a
    /// result arriving before then has nothing to attach to and is dropped.
    round_id: Option<String>,
}

/// Turns the call path's requests into calls on the configured engines.
///
/// It holds no voice of its own: which voice to speak with arrives with each
/// request, from the persona that owns the session.
pub struct LocalVoiceRuntime {
    asr: Arc<dyn AsrEngine>,
    tts: Arc<dyn TtsEngine>,
    sessions: Mutex<HashMap<String, AsrSession>>,
}

impl LocalVoiceRuntime {
    pub fn new(asr: Arc<dyn AsrEngine>, tts: Arc<dyn TtsEngine>) -> Self {
        Self {
            asr,
            tts,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn lock_sessions(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, AsrSession>>> {
        self.sessions
            .lock()
            .map_err(|_| AppError::internal("voice runtime session map is poisoned"))
    }

    /// Runs an operation against one session's recognition stream.
    ///
    /// This used to move the stream into the blocking pool, because recognition
    /// was ONNX inference on this thread. It is now a message to a socket task,
    /// so the hop cost a task switch per frame — fifty a second — and bought
    /// nothing.
    fn on_stream<T>(
        &self,
        asr_session_id: &str,
        operation: impl FnOnce(&mut Box<dyn AsrStream>) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut sessions = self.lock_sessions()?;
        let session = sessions
            .get_mut(asr_session_id)
            .ok_or_else(|| AppError::not_found(format!("unknown asr session: {asr_session_id}")))?;
        operation(&mut session.stream)
    }
}

#[async_trait]
impl VoiceRuntimeExecutionPort for LocalVoiceRuntime {
    async fn open_asr_session_for_runtime(
        &self,
        request: OpenRuntimeAsrSessionRequest,
    ) -> AppResult<OpenRuntimeAsrSessionResult> {
        let stream = self.asr.open(&AsrStreamConfig {
            sample_rate_hz: request.sample_rate_hz,
            num_channels: request.num_channels,
            language: request.language,
        })?;
        let asr_session_id = request.speech_session_id;
        let mut sessions = self.lock_sessions()?;
        sessions.insert(
            asr_session_id.clone(),
            AsrSession {
                stream,
                round_id: None,
            },
        );
        Ok(OpenRuntimeAsrSessionResult { asr_session_id })
    }

    async fn push_asr_audio_for_runtime(
        &self,
        request: PushRuntimeAsrAudioRequest,
    ) -> AppResult<()> {
        let frame = request.pcm_s16le;
        self.on_stream(&request.asr_session_id, move |stream| stream.push(&frame))
    }

    async fn commit_asr_session_for_runtime(
        &self,
        request: CommitRuntimeAsrSessionRequest,
    ) -> AppResult<()> {
        {
            let mut sessions = self.lock_sessions()?;
            let session = sessions.get_mut(&request.asr_session_id).ok_or_else(|| {
                AppError::not_found(format!("unknown asr session: {}", request.asr_session_id))
            })?;
            session.round_id = Some(request.round_id);
        }
        self.on_stream(&request.asr_session_id, |stream| stream.finish())
    }

    async fn poll_asr_events_for_runtime(
        &self,
        request: PollRuntimeAsrEventsRequest,
    ) -> AppResult<PollRuntimeAsrEventsResult> {
        let round_id = {
            let sessions = self.lock_sessions()?;
            let session = sessions.get(&request.asr_session_id).ok_or_else(|| {
                AppError::not_found(format!("unknown asr session: {}", request.asr_session_id))
            })?;
            session.round_id.clone()
        };
        // Polling only drains an already-decoded queue, so it stays on the async
        // runtime.
        let max_events = request.max_events;
        let events = self.on_stream(&request.asr_session_id, move |stream| {
            let mut events = Vec::new();
            while events.len() < max_events {
                let Some(event) = stream.poll() else {
                    break;
                };
                events.push(event);
            }
            Ok(events)
        })?;

        // A transcript that arrives before the round is committed has nothing to
        // attach to.
        let Some(round_id) = round_id else {
            return Ok(PollRuntimeAsrEventsResult { events: Vec::new() });
        };
        let events = events
            .into_iter()
            .map(|event| match event {
                AsrEvent::Partial { transcript } => RuntimeAsrEvent::PartialTranscript {
                    round_id: round_id.clone(),
                    transcript,
                },
                AsrEvent::Final { transcript } => RuntimeAsrEvent::FinalTranscript {
                    round_id: round_id.clone(),
                    transcript,
                },
            })
            .collect();
        Ok(PollRuntimeAsrEventsResult { events })
    }

    async fn close_asr_session_for_runtime(
        &self,
        request: CloseRuntimeAsrSessionRequest,
    ) -> AppResult<()> {
        let mut sessions = self.lock_sessions()?;
        sessions.remove(&request.asr_session_id);
        Ok(())
    }

    async fn stream_tts_for_runtime(
        &self,
        request: RuntimeTtsExecutionRequest,
    ) -> AppResult<RuntimeTtsExecutionStream> {
        let chunks = self
            .tts
            .synthesize(TtsRequest {
                text: request.text,
                voice: request.voice,
            })
            .await?;
        Ok(RuntimeTtsExecutionStream { chunks })
    }
}
