use async_trait::async_trait;
use call_log_contract::{CallEvent, CallEventSinkPort};
use chrono::Utc;
use shared_kernel::AppResult;

use crate::{SpeechLogEvent, SpeechRuntimeEventPort};

#[derive(Clone)]
pub struct SpeechRuntimeEventAdapter<S> {
    sink: S,
}

impl<S> SpeechRuntimeEventAdapter<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl<S> SpeechRuntimeEventPort for SpeechRuntimeEventAdapter<S>
where
    S: CallEventSinkPort + Send + Sync,
{
    async fn publish(&self, event: SpeechLogEvent) -> AppResult<()> {
        self.sink
            .publish(CallEvent {
                session_id: event.session_id,
                round_id: event.round_id,
                source: "speech_runtime".to_owned(),
                event: event.event,
                ts_ms: Utc::now().timestamp_millis(),
                fields: event.fields,
            })
            .await
    }
}
