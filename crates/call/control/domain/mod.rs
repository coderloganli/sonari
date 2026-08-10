pub mod bot_speech;

pub use crate::contract::CallStatus;
pub use call_log_contract::CallEvent;
pub use call_runtime_control::RuntimeWorkStatus;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub use shared_kernel::CallerIdentity as CallCallerIdentity;

pub use bot_speech::{
    BotSpeechDispatchDecision, BotSpeechInterruptionDecision, BotSpeechState, BotSpeechStateEvent,
    BotSpeechStateTransitionResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallType {
    Realtime,
}

impl CallType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "realtime" => Some(Self::Realtime),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeechInputLanguage {
    Zh,
    En,
}

impl SpeechInputLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSession {
    pub id: i64,
    pub caller: CallCallerIdentity,
    pub realtime_participant_identity: String,
    pub character_id: i64,
    pub character_name: String,
    pub scene_id: Option<i64>,
    pub scene_name: Option<String>,
    pub voice: String,
    pub agent_session_id: String,
    pub asr_language: SpeechInputLanguage,
    pub call_type: CallType,
    pub status: CallStatus,
    pub owner_instance: String,
    pub runtime_owner_id: Option<String>,
    pub runtime_status: RuntimeWorkStatus,
    pub runtime_failure_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallTimelineEvent {
    pub id: i64,
    pub session_id: i64,
    pub round_id: Option<String>,
    pub source: String,
    pub event: String,
    pub ts_ms: i64,
    pub fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallActivityEvent {
    pub name: String,
    pub timestamp: i64,
    pub offset_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallActivityRound {
    pub round_id: String,
    pub events: Vec<CallActivityEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallActivityLog {
    pub session_id: i64,
    pub started_at_ms: i64,
    pub rounds: Vec<CallActivityRound>,
}
