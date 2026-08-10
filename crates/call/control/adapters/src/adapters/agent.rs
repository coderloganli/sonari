use std::sync::Arc;

use async_trait::async_trait;
use call::{
    CallCallerIdentity,
    ports::{AgentSessionHandle, AgentSessionPort},
};

#[derive(Clone)]
pub struct AgentSessionAdapter {
    service: Arc<dyn agent::AgentCallControlPort>,
}

impl AgentSessionAdapter {
    pub fn new(service: Arc<dyn agent::AgentCallControlPort>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AgentSessionPort for AgentSessionAdapter {
    async fn create_session(
        &self,
        caller: CallCallerIdentity,
        character_id: i64,
        scene_id: Option<i64>,
        timezone: &str,
    ) -> shared_kernel::AppResult<AgentSessionHandle> {
        let session = self
            .service
            .create_call_session(agent::CreateAgentSessionRequest {
                caller,
                character_id,
                timezone: timezone.to_owned(),
                scene_id,
            })
            .await?;
        Ok(AgentSessionHandle {
            session_id: session.session_id,
        })
    }
}
