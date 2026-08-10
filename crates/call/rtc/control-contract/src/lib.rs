use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeJoinArtifacts {
    pub provider: String,
    pub endpoint: String,
    pub room_name: String,
    pub access_token: String,
    pub participant_identity: String,
    pub bot_participant_identity: String,
}

#[async_trait]
pub trait RealtimeJoinArtifactsPort: Send + Sync {
    async fn issue_user_join_artifacts(
        &self,
        session_id: i64,
        participant_identity: &str,
    ) -> AppResult<RealtimeJoinArtifacts>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLaunchArtifacts {
    pub endpoint: String,
    pub room_name: String,
    pub access_token: String,
    pub local_participant_identity: String,
    pub expected_remote_participant_identity: String,
}

#[async_trait]
pub trait RuntimeLaunchArtifactsPort: Send + Sync {
    async fn issue_runtime_launch_artifacts(
        &self,
        session_id: i64,
        expected_remote_participant_identity: &str,
    ) -> AppResult<RuntimeLaunchArtifacts>;
}
