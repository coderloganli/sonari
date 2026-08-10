use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ObservabilityResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientDebugEvent {
    pub ts_ms: i64,
    pub name: String,
    pub level: String,
    pub flow_id: Option<String>,
    pub event_class: Option<String>,
    pub session_id: Option<String>,
    pub character_id: Option<String>,
    pub device_name: Option<String>,
    pub fields: Value,
    pub merged_count: i32,
}

#[async_trait]
pub trait ClientDebugEventSinkPort: Send + Sync {
    async fn publish_client_event(&self, event: ClientDebugEvent) -> ObservabilityResult<()>;
}
