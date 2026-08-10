use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallEvent {
    pub session_id: i64,
    pub round_id: Option<String>,
    pub source: String,
    pub event: String,
    pub ts_ms: i64,
    pub fields: serde_json::Value,
}

#[async_trait]
pub trait CallEventSinkPort: Send + Sync {
    async fn publish(&self, event: CallEvent) -> AppResult<()>;
}

#[async_trait]
impl<T> CallEventSinkPort for Arc<T>
where
    T: CallEventSinkPort + Send + Sync + ?Sized,
{
    async fn publish(&self, event: CallEvent) -> AppResult<()> {
        (**self).publish(event).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallPromptLogMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallPromptLogSnapshot {
    pub session_id: i64,
    pub agent_session_id: String,
    pub round_id: Option<String>,
    pub source: String,
    pub model_name: String,
    pub request_messages: Vec<CallPromptLogMessage>,
    pub response_text: String,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait CallPromptLogSinkPort: Send + Sync {
    async fn publish_prompt_log(&self, snapshot: CallPromptLogSnapshot) -> AppResult<()>;
}

#[async_trait]
pub trait CallPromptLogContextPort: Send + Sync {
    async fn resolve_call_session_id(&self, agent_session_id: &str) -> AppResult<i64>;
}
