use async_trait::async_trait;
use rtc_control_contract::{RealtimeJoinArtifacts, RealtimeJoinArtifactsPort};
use shared_kernel::AppResult;

use super::naming::bot_identity_for_session;
use super::token::LiveKitTokenIssuer;

#[derive(Debug, Clone)]
pub struct LiveKitRealtimeJoinAdapter {
    issuer: LiveKitTokenIssuer,
}

impl LiveKitRealtimeJoinAdapter {
    pub fn new(issuer: LiveKitTokenIssuer) -> Self {
        Self { issuer }
    }
}

#[async_trait]
impl RealtimeJoinArtifactsPort for LiveKitRealtimeJoinAdapter {
    async fn issue_user_join_artifacts(
        &self,
        session_id: i64,
        participant_identity: &str,
    ) -> AppResult<RealtimeJoinArtifacts> {
        let artifacts = self
            .issuer
            .issue_user_join_artifacts(session_id, participant_identity)?;
        Ok(RealtimeJoinArtifacts {
            provider: "livekit".to_owned(),
            endpoint: artifacts.livekit_url,
            room_name: artifacts.room_name.as_str().to_owned(),
            access_token: artifacts.user_token,
            participant_identity: artifacts.user_identity.as_str().to_owned(),
            bot_participant_identity: bot_identity_for_session(session_id).as_str().to_owned(),
        })
    }
}
