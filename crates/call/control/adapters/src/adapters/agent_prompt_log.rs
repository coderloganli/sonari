use async_trait::async_trait;
use call::CallEvent;
use call::ports::EventSinkPort;
use call_log_contract::{CallPromptLogSinkPort, CallPromptLogSnapshot};
use shared_kernel::AppResult;

#[derive(Clone)]
pub struct AgentPromptLogEventAdapter<S> {
    sink: S,
}

impl<S> AgentPromptLogEventAdapter<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl<S> CallPromptLogSinkPort for AgentPromptLogEventAdapter<S>
where
    S: EventSinkPort + Send + Sync,
{
    async fn publish_prompt_log(&self, snapshot: CallPromptLogSnapshot) -> AppResult<()> {
        self.sink
            .publish(CallEvent {
                session_id: snapshot.session_id,
                round_id: snapshot.round_id.clone(),
                source: snapshot.source.clone(),
                event: "agent_prompt_logged".to_owned(),
                ts_ms: snapshot.created_at.timestamp_millis(),
                fields: serde_json::json!({
                    "agent_session_id": snapshot.agent_session_id,
                    "model_name": snapshot.model_name,
                    "request_messages": snapshot.request_messages,
                    "response_text": snapshot.response_text,
                }),
            })
            .await
    }
}
