use serde::{Deserialize, Serialize};

pub const RUNTIME_FACT_STREAM_PREFIX: &str = "call:runtime-facts";

pub fn runtime_fact_stream_key(runtime_owner_id: &str) -> String {
    format!("{RUNTIME_FACT_STREAM_PREFIX}:{runtime_owner_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechInputMediaState {
    Open,
    OutputTurnPending,
    BotPlayback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeWorkKind {
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeWorkStatus {
    None,
    PendingStart,
    StartClaimed,
    Ready,
    StopRequested,
    StopClaimed,
    Stopped,
    Failed,
}

impl RuntimeWorkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PendingStart => "pending_start",
            Self::StartClaimed => "start_claimed",
            Self::Ready => "ready",
            Self::StopRequested => "stop_requested",
            Self::StopClaimed => "stop_claimed",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "pending_start" => Some(Self::PendingStart),
            "start_claimed" => Some(Self::StartClaimed),
            "ready" => Some(Self::Ready),
            "stop_requested" => Some(Self::StopRequested),
            "stop_claimed" => Some(Self::StopClaimed),
            "stopped" => Some(Self::Stopped),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl RuntimeWorkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeLaunchSpec {
    pub endpoint: String,
    pub room_name: String,
    pub access_token: String,
    pub local_participant_identity: String,
    pub expected_remote_participant_identity: String,
    /// Per-session runtime configuration for the co-located media plane.
    #[serde(default)]
    pub speech: Option<SpeechRuntimeBootstrap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeWorkItem {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub kind: RuntimeWorkKind,
    pub launch: Option<RuntimeLaunchSpec>,
}

/// Per-session configuration handed to the media plane with a launch spec.
///
/// Speech models are loaded by the process itself, so nothing about providers,
/// endpoints or credentials travels here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechRuntimeBootstrap {
    pub agent_session_id: String,
    pub voice: String,
    /// Language code, matching `voice::AsrLanguage`.
    pub language: String,
    pub segmentation: SegmentationConfigSpec,
    pub llm: LlmSelectionSpec,
}

/// Endpointing and VAD parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentationConfigSpec {
    pub min_utterance_ms: u32,
    pub silence_flush_ms: u32,
    pub silence_force_agent_ms: u32,
    pub voice_activity_threshold: i16,
    pub min_speech_confirm_ms: u32,
}

/// Which LLM to call. The endpoint is reached over HTTP; its key comes from the
/// environment, never from a session payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmSelectionSpec {
    pub provider_key: String,
    pub endpoint: String,
    pub model: String,
    pub temperature: f64,
    pub frequency_penalty: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RuntimeFactKind {
    WorkerRuntimeStarted,
    WorkerRuntimeReady,
    WorkerRuntimeStopped,
    WorkerRuntimeFailed { reason: String },
    WorkerRuntimeMissing,
    RuntimeReplyStarted { reply_text: String },
    RuntimeReplyFinished,
    RuntimePlaybackCompleted,
    ExternalAudioStarted { audio_url: String, band: String },
    ExternalAudioFinished { audio_url: String, band: String },
    WorkerBargeInDetected { rms: f64 },
    RuntimeInputRoundFailed { reason: String },
}

impl RuntimeFactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkerRuntimeStarted => "worker_runtime_started",
            Self::WorkerRuntimeReady => "worker_runtime_ready",
            Self::WorkerRuntimeStopped => "worker_runtime_stopped",
            Self::WorkerRuntimeFailed { .. } => "worker_runtime_failed",
            Self::WorkerRuntimeMissing => "worker_runtime_missing",
            Self::RuntimeReplyStarted { .. } => "runtime_reply_started",
            Self::RuntimeReplyFinished => "runtime_reply_finished",
            Self::RuntimePlaybackCompleted => "runtime_playback_completed",
            Self::ExternalAudioStarted { .. } => "external_audio_started",
            Self::ExternalAudioFinished { .. } => "external_audio_finished",
            Self::WorkerBargeInDetected { .. } => "worker_barge_in_detected",
            Self::RuntimeInputRoundFailed { .. } => "runtime_input_round_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEventFact {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub round_id: Option<String>,
    pub source: String,
    pub ts_ms: i64,
    pub kind: RuntimeFactKind,
}
