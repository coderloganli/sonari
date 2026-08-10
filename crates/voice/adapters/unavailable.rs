//! The voice runtime used when no models are configured.
//!
//! Every call fails as unavailable rather than panicking or hanging, so the
//! process still serves its HTTP surface and reports why voice does not work.

use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use crate::ports::{
    CloseRuntimeAsrSessionRequest, CommitRuntimeAsrSessionRequest, OpenRuntimeAsrSessionRequest,
    OpenRuntimeAsrSessionResult, PollRuntimeAsrEventsRequest, PollRuntimeAsrEventsResult,
    PushRuntimeAsrAudioRequest, RuntimeTtsExecutionRequest, RuntimeTtsExecutionStream,
    TtsAudioStream, TtsEngine, TtsRequest, VoiceRuntimeExecutionPort,
};

pub struct UnavailableVoiceRuntime;

fn unavailable<T>() -> AppResult<T> {
    Err(AppError::unavailable(
        "no speech models are configured; voice is unavailable",
    ))
}

#[async_trait]
impl VoiceRuntimeExecutionPort for UnavailableVoiceRuntime {
    async fn open_asr_session_for_runtime(
        &self,
        _request: OpenRuntimeAsrSessionRequest,
    ) -> AppResult<OpenRuntimeAsrSessionResult> {
        unavailable()
    }

    async fn push_asr_audio_for_runtime(
        &self,
        _request: PushRuntimeAsrAudioRequest,
    ) -> AppResult<()> {
        unavailable()
    }

    async fn commit_asr_session_for_runtime(
        &self,
        _request: CommitRuntimeAsrSessionRequest,
    ) -> AppResult<()> {
        unavailable()
    }

    async fn poll_asr_events_for_runtime(
        &self,
        _request: PollRuntimeAsrEventsRequest,
    ) -> AppResult<PollRuntimeAsrEventsResult> {
        unavailable()
    }

    async fn close_asr_session_for_runtime(
        &self,
        _request: CloseRuntimeAsrSessionRequest,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn stream_tts_for_runtime(
        &self,
        _request: RuntimeTtsExecutionRequest,
    ) -> AppResult<RuntimeTtsExecutionStream> {
        unavailable()
    }
}

/// Synthesis when no model is loaded. Recognition can still run, so the failure
/// is reported per request rather than by refusing the whole voice path.
pub struct UnavailableTtsEngine;

#[async_trait]
impl TtsEngine for UnavailableTtsEngine {
    async fn synthesize(&self, _request: TtsRequest) -> AppResult<TtsAudioStream> {
        Err(AppError::unavailable(
            "no synthesis model is configured; the agent cannot speak",
        ))
    }
}
