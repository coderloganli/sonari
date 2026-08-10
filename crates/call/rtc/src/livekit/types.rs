use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveKitRoomName(String);

impl LiveKitRoomName {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveKitParticipantIdentity(String);

impl LiveKitParticipantIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveKitUserJoinArtifacts {
    pub livekit_url: String,
    pub room_name: LiveKitRoomName,
    pub user_identity: LiveKitParticipantIdentity,
    pub bot_identity: LiveKitParticipantIdentity,
    pub user_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveKitBotJoinArtifacts {
    pub livekit_url: String,
    pub room_name: LiveKitRoomName,
    pub bot_identity: LiveKitParticipantIdentity,
    pub bot_token: String,
}
