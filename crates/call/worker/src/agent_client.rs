//! `AgentTurnPort` backed by a direct call into the agent.
//!
//! The agent owns history, prompt assembly and usage accounting, all of which
//! are database-bound. It lives in the same process, so a turn is a function
//! call rather than a round trip.

use std::sync::Arc;

use agent::{AgentRuntimeUseCases, ChatCommand};
use async_trait::async_trait;
use shared_kernel::AppResult;
use speech_runtime::{AgentTurnPort, AgentTurnRequest, AgentTurnResult};

pub struct LocalAgentTurnAdapter {
    agent: Arc<dyn AgentRuntimeUseCases>,
}

impl LocalAgentTurnAdapter {
    pub fn new(agent: Arc<dyn AgentRuntimeUseCases>) -> Self {
        Self { agent }
    }

    /// The opening line of a server-initiated turn, fetched once the session exists.
    pub async fn generate_welcome(&self, agent_session_id: &str) -> AppResult<String> {
        self.agent.generate_welcome_message(agent_session_id).await
    }
}

#[async_trait]
impl AgentTurnPort for LocalAgentTurnAdapter {
    async fn chat_once(&self, request: AgentTurnRequest) -> AppResult<AgentTurnResult> {
        let outcome = self
            .agent
            .chat_once(ChatCommand {
                session_id: request.session_id,
                user_message: request.user_message,
            })
            .await?;
        Ok(AgentTurnResult {
            reply_text: outcome.reply_text,
            first_token_at_ms: outcome.first_token_at_ms,
            first_sentence_at_ms: outcome.first_sentence_at_ms,
        })
    }
}
