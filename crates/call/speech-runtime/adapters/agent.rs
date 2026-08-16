use std::sync::Arc;

use async_trait::async_trait;
use shared_kernel::AppResult;

use crate::{AgentTurnPort, AgentTurnRequest, AgentTurnResult};

pub struct AgentRuntimeAdapter {
    inner: Arc<dyn agent::AgentRuntimeUseCases>,
}

impl AgentRuntimeAdapter {
    pub fn new(inner: Arc<dyn agent::AgentRuntimeUseCases>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl AgentTurnPort for AgentRuntimeAdapter {
    async fn chat_once(&self, request: AgentTurnRequest) -> AppResult<AgentTurnResult> {
        let outcome = self
            .inner
            .chat_once(agent::ChatCommand {
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
