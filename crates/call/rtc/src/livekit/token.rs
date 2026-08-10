use livekit_api::{access_token, access_token::VideoGrants};
use shared_kernel::{AppError, AppResult};

use super::naming::{bot_identity_for_session, room_name_for_session};
use super::types::LiveKitParticipantIdentity;
use super::types::{LiveKitBotJoinArtifacts, LiveKitUserJoinArtifacts};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitTokenConfig {
    pub internal_livekit_url: String,
    pub public_livekit_url: String,
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone)]
pub struct LiveKitTokenIssuer {
    config: LiveKitTokenConfig,
}

impl LiveKitTokenIssuer {
    pub fn new(config: LiveKitTokenConfig) -> Self {
        Self { config }
    }

    pub fn issue_user_join_artifacts(
        &self,
        session_id: i64,
        participant_identity: &str,
    ) -> AppResult<LiveKitUserJoinArtifacts> {
        let room_name = room_name_for_session(session_id);
        let user_identity = LiveKitParticipantIdentity::new(participant_identity.to_owned());
        let bot_identity = bot_identity_for_session(session_id);
        let user_token = self.issue_token(user_identity.as_str(), room_name.as_str())?;

        Ok(LiveKitUserJoinArtifacts {
            livekit_url: self.config.public_livekit_url.clone(),
            room_name,
            user_identity,
            bot_identity,
            user_token,
        })
    }

    pub fn issue_bot_join_artifacts(&self, session_id: i64) -> AppResult<LiveKitBotJoinArtifacts> {
        let room_name = room_name_for_session(session_id);
        let bot_identity = bot_identity_for_session(session_id);
        let bot_token = self.issue_token(bot_identity.as_str(), room_name.as_str())?;

        Ok(LiveKitBotJoinArtifacts {
            livekit_url: self.config.internal_livekit_url.clone(),
            room_name,
            bot_identity,
            bot_token,
        })
    }

    fn issue_token(&self, identity: &str, room_name: &str) -> AppResult<String> {
        access_token::AccessToken::with_api_key(&self.config.api_key, &self.config.api_secret)
            .with_identity(identity)
            .with_grants(VideoGrants {
                room_join: true,
                room: room_name.to_owned(),
                ..Default::default()
            })
            .to_jwt()
            .map_err(|error| AppError::internal(format!("failed to issue livekit token: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveKitTokenConfig, LiveKitTokenIssuer};

    fn issuer() -> LiveKitTokenIssuer {
        LiveKitTokenIssuer::new(LiveKitTokenConfig {
            internal_livekit_url: "ws://livekit.internal:7880".to_owned(),
            public_livekit_url: "wss://rtc.example.com".to_owned(),
            api_key: "devkey".to_owned(),
            api_secret: "devsecret".to_owned(),
        })
    }

    #[test]
    fn issue_user_join_artifacts_uses_room_and_identity_convention() {
        let artifacts_result = issuer().issue_user_join_artifacts(7, "platform_user:21");
        assert!(artifacts_result.is_ok());
        let artifacts = match artifacts_result {
            Ok(artifacts) => artifacts,
            Err(_) => return,
        };

        assert_eq!(artifacts.room_name.as_str(), "call-7");
        assert_eq!(artifacts.user_identity.as_str(), "platform_user:21");
        assert!(!artifacts.user_token.is_empty());
    }

    #[test]
    fn issue_bot_join_artifacts_uses_bot_identity() {
        let artifacts_result = issuer().issue_bot_join_artifacts(7);
        assert!(artifacts_result.is_ok());
        let artifacts = match artifacts_result {
            Ok(artifacts) => artifacts,
            Err(_) => return,
        };

        assert_eq!(artifacts.room_name.as_str(), "call-7");
        assert_eq!(artifacts.bot_identity.as_str(), "bot-7");
        assert!(!artifacts.bot_token.is_empty());
    }
}
