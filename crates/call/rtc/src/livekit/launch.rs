use async_trait::async_trait;
use rtc_control_contract::{RuntimeLaunchArtifacts, RuntimeLaunchArtifactsPort};
use shared_kernel::AppResult;

use super::token::LiveKitTokenIssuer;

#[derive(Debug, Clone)]
pub struct LiveKitRuntimeLaunchAdapter {
    issuer: LiveKitTokenIssuer,
}

impl LiveKitRuntimeLaunchAdapter {
    pub fn new(issuer: LiveKitTokenIssuer) -> Self {
        Self { issuer }
    }
}

#[async_trait]
impl RuntimeLaunchArtifactsPort for LiveKitRuntimeLaunchAdapter {
    async fn issue_runtime_launch_artifacts(
        &self,
        session_id: i64,
        expected_remote_participant_identity: &str,
    ) -> AppResult<RuntimeLaunchArtifacts> {
        let artifacts = self.issuer.issue_bot_join_artifacts(session_id)?;
        Ok(RuntimeLaunchArtifacts {
            endpoint: artifacts.livekit_url,
            room_name: artifacts.room_name.as_str().to_owned(),
            access_token: artifacts.bot_token,
            local_participant_identity: artifacts.bot_identity.as_str().to_owned(),
            expected_remote_participant_identity: expected_remote_participant_identity.to_owned(),
        })
    }
}
