use async_trait::async_trait;
use call_log_contract::{CallEvent, CallEventSinkPort};
use shared_kernel::AppResult;

use crate::application::{CallExecutionEvent, CallExecutionEventPort};

pub struct CallExecutionEventAdapter<S> {
    sink: S,
}

impl<S> CallExecutionEventAdapter<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl<S> CallExecutionEventPort for CallExecutionEventAdapter<S>
where
    S: CallEventSinkPort + Send + Sync,
{
    async fn publish(&self, event: CallExecutionEvent) -> AppResult<()> {
        self.sink
            .publish(CallEvent {
                session_id: event.session_id,
                round_id: event.round_id,
                source: event.source,
                event: event.event,
                ts_ms: event.ts_ms,
                fields: event.fields,
            })
            .await
    }
}
