use std::sync::Arc;

use async_trait::async_trait;
use call::ports::{
    BuildServerInitiatedTurnCommand, RuntimeSessionStatePort, ServerInitiatedBotSpeechPort,
};
use call_control_bot_speech::BotSpeechItem;
use call_control_orchestration::{
    ServerInitiatedTurnPort, ServerInitiatedTurnRequest, ServerInitiatedTurnTrigger,
    build_server_initiated_bot_speech_item,
};
use shared_kernel::AppError;

#[derive(Clone)]
pub struct ServerInitiatedTurnTextAdapter {
    agent: Arc<dyn agent::AgentCallControlPort>,
}

impl ServerInitiatedTurnTextAdapter {
    pub fn new(agent: Arc<dyn agent::AgentCallControlPort>) -> Self {
        Self { agent }
    }
}

#[async_trait]
impl ServerInitiatedTurnPort for ServerInitiatedTurnTextAdapter {
    async fn generate_turn(
        &self,
        request: &ServerInitiatedTurnRequest,
    ) -> shared_kernel::AppResult<String> {
        match request.trigger {
            ServerInitiatedTurnTrigger::CallStarted => {
                self.agent
                    .generate_welcome_message(&request.agent_session_id)
                    .await
            }
        }
    }
}

#[derive(Clone)]
pub struct ServerInitiatedBotSpeechAdapter<R, S> {
    runtime_sessions: R,
    server_turns: S,
}

impl<R, S> ServerInitiatedBotSpeechAdapter<R, S> {
    pub fn new(runtime_sessions: R, server_turns: S) -> Self {
        Self {
            runtime_sessions,
            server_turns,
        }
    }
}

#[async_trait]
impl<R, S> ServerInitiatedBotSpeechPort for ServerInitiatedBotSpeechAdapter<R, S>
where
    R: RuntimeSessionStatePort + Send + Sync,
    S: ServerInitiatedTurnPort + Send + Sync,
{
    async fn build_server_initiated_turn(
        &self,
        command: BuildServerInitiatedTurnCommand,
    ) -> shared_kernel::AppResult<BotSpeechItem> {
        let state = self
            .runtime_sessions
            .get_runtime_session_state(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("runtime session state not found"))?;
        if state.runtime_owner_id.as_deref() != Some(command.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }

        let request = ServerInitiatedTurnRequest {
            session_id: command.session_id,
            agent_session_id: state.agent_session_id,
            trigger: command.trigger,
        };
        let text = self.server_turns.generate_turn(&request).await?;
        Ok(build_server_initiated_bot_speech_item(
            request.session_id,
            request.trigger,
            text,
        ))
    }
}
