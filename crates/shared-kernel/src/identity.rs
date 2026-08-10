use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallerIdentity {
    PlatformUser { user_id: i64 },
}

impl CallerIdentity {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::PlatformUser { .. } => "platform_user",
        }
    }

    pub fn platform_user_id(&self) -> Option<i64> {
        match self {
            Self::PlatformUser { user_id } => Some(*user_id),
        }
    }

    pub fn realtime_participant_identity(&self) -> String {
        match self {
            Self::PlatformUser { user_id } => format!("platform_user:{user_id}"),
        }
    }
}
