//! Local inference interfaces.
//!
//! Every implementation runs inside this process against a model loaded from
//! disk. No credential, endpoint or supplier appears in any signature here — if
//! one ever needs to, the component belongs behind a different boundary.
//!
//! The asymmetry between ASR and TTS is deliberate. ASR is fed at frame rate and
//! produces results on its own schedule, so it is push and poll. TTS is driven
//! once per synthesis unit and streams its output back, so it is one call
//! returning a stream.

use async_trait::async_trait;
use shared_kernel::AppResult;

use crate::domain::AsrLanguage;
use crate::ports::TtsAudioStream;

/// Whether the detector currently believes speech is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Silence,
    Speech,
}

/// Frame-rate speech detection. One instance per call: it owns its own state,
/// which is why this takes `&mut self` rather than carrying state in the caller.
pub trait Vad: Send {
    fn push(&mut self, frame: &[i16]) -> AppResult<VadState>;
    fn reset(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrStreamConfig {
    pub sample_rate_hz: u32,
    pub num_channels: u16,
    pub language: AsrLanguage,
}

/// A loaded recognition model. Shared across calls — loading is expensive and
/// the model itself is immutable.
pub trait AsrEngine: Send + Sync {
    fn open(&self, config: &AsrStreamConfig) -> AppResult<Box<dyn AsrStream>>;
}

/// One call's recognition state.
pub trait AsrStream: Send {
    /// Feed one frame. Returns as soon as the frame is accepted; recognition
    /// results arrive through `poll`.
    fn push(&mut self, frame: &[i16]) -> AppResult<()>;

    /// Take the next result, if one is ready. Never blocks.
    fn poll(&mut self) -> Option<AsrEvent>;

    /// Declare the end of the current utterance. Any remaining audio is flushed
    /// and the final result becomes available through `poll`.
    fn finish(&mut self) -> AppResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrEvent {
    Partial { transcript: String },
    Final { transcript: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRequest {
    pub text: String,
    /// A speaker within the loaded model, named in configuration.
    pub voice: String,
}

/// A loaded synthesis model. Shared across calls for the same reason as
/// `AsrEngine`.
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Begins synthesis and returns audio as it is produced, so playback can
    /// start before the utterance is complete.
    async fn synthesize(&self, request: TtsRequest) -> AppResult<TtsAudioStream>;
}
