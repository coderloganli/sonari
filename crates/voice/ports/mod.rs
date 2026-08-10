use async_trait::async_trait;
use futures::Stream;
use shared_kernel::AppResult;
use std::{pin::Pin, sync::Arc};

use crate::domain::AsrLanguage;

#[path = "engine.rs"]
mod engine;
pub use engine::{
    AsrEngine, AsrEvent, AsrStream, AsrStreamConfig, TtsEngine, TtsRequest, Vad, VadState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRuntimeAsrSessionRequest {
    pub speech_session_id: String,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
    pub language: AsrLanguage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRuntimeAsrSessionResult {
    pub asr_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRuntimeAsrAudioRequest {
    pub asr_session_id: String,
    pub pcm_s16le: Vec<i16>,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRuntimeAsrSessionRequest {
    pub asr_session_id: String,
    pub round_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollRuntimeAsrEventsRequest {
    pub asr_session_id: String,
    pub max_events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAsrEvent {
    PartialTranscript {
        round_id: String,
        transcript: String,
    },
    FinalTranscript {
        round_id: String,
        transcript: String,
    },
    Warning {
        message: String,
    },
    RoundFailed {
        round_id: String,
        message: String,
    },
    SessionFailed {
        message: String,
    },
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollRuntimeAsrEventsResult {
    pub events: Vec<RuntimeAsrEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseRuntimeAsrSessionRequest {
    pub asr_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsSynthesisRequest {
    pub text: String,
    pub voice_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsAudioChunk {
    pub pcm_s16le: Vec<i16>,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

pub type TtsAudioStream = Pin<Box<dyn Stream<Item = AppResult<TtsAudioChunk>> + Send>>;

pub struct TtsSynthesisStream {
    pub chunks: TtsAudioStream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTtsExecutionRequest {
    pub text: String,
    pub voice: String,
}

pub struct RuntimeTtsExecutionStream {
    pub chunks: TtsAudioStream,
}

#[async_trait]
pub trait VoiceConfigRepository: Send + Sync {
    async fn get_asr_input_language(&self) -> AppResult<AsrLanguage>;
    async fn set_asr_input_language(&self, language: AsrLanguage) -> AppResult<()>;
}

#[async_trait]
impl<T> VoiceConfigRepository for Arc<T>
where
    T: VoiceConfigRepository + ?Sized,
{
    async fn get_asr_input_language(&self) -> AppResult<AsrLanguage> {
        (**self).get_asr_input_language().await
    }
    async fn set_asr_input_language(&self, language: AsrLanguage) -> AppResult<()> {
        (**self).set_asr_input_language(language).await
    }
}

#[async_trait]
pub trait VoiceRuntimeExecutionPort: Send + Sync {
    async fn open_asr_session_for_runtime(
        &self,
        request: OpenRuntimeAsrSessionRequest,
    ) -> AppResult<OpenRuntimeAsrSessionResult>;
    async fn push_asr_audio_for_runtime(
        &self,
        request: PushRuntimeAsrAudioRequest,
    ) -> AppResult<()>;
    async fn commit_asr_session_for_runtime(
        &self,
        request: CommitRuntimeAsrSessionRequest,
    ) -> AppResult<()>;
    async fn poll_asr_events_for_runtime(
        &self,
        request: PollRuntimeAsrEventsRequest,
    ) -> AppResult<PollRuntimeAsrEventsResult>;
    async fn close_asr_session_for_runtime(
        &self,
        request: CloseRuntimeAsrSessionRequest,
    ) -> AppResult<()>;
    async fn stream_tts_for_runtime(
        &self,
        request: RuntimeTtsExecutionRequest,
    ) -> AppResult<RuntimeTtsExecutionStream>;
}
