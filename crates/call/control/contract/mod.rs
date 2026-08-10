//! Types the control plane exposes to the rest of the system.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallStatus {
    Starting,
    Active,
    Ending,
    Ended,
    Failed,
}

impl CallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Active => "active",
            Self::Ending => "ending",
            Self::Ended => "ended",
            Self::Failed => "failed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "starting" => Some(Self::Starting),
            "active" => Some(Self::Active),
            "ending" => Some(Self::Ending),
            "ended" => Some(Self::Ended),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}
