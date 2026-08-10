//! Forwards the speech runtime's stage events — ASR, agent and TTS timings —
//! into the same call event sink the rest of the system writes to.
//!
//! `ts_ms` is stamped when the event occurs, so a delayed write does not distort
//! stage timing. Publishing is fire-and-forget: observability is best-effort and
//! must never block a turn.

use std::sync::Arc;

use async_trait::async_trait;
use call_log_contract::{CallEvent, CallEventSinkPort};
use shared_kernel::AppResult;
use speech_runtime::{SpeechLogEvent, SpeechRuntimeEventPort};

#[derive(Clone)]
pub struct LocalSpeechEventsAdapter {
    sink: Arc<dyn CallEventSinkPort>,
}

impl LocalSpeechEventsAdapter {
    pub fn new(sink: Arc<dyn CallEventSinkPort>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl SpeechRuntimeEventPort for LocalSpeechEventsAdapter {
    async fn publish(&self, event: SpeechLogEvent) -> AppResult<()> {
        let call_event = CallEvent {
            session_id: event.session_id,
            round_id: event.round_id,
            source: "speech_runtime".to_owned(),
            event: event.event,
            ts_ms: chrono::Utc::now().timestamp_millis(),
            fields: event.fields,
        };
        let sink = self.sink.clone();
        tokio::spawn(async move {
            let event_name = call_event.event.clone();
            if let Err(error) = sink.publish(call_event).await {
                tracing::warn!(
                    reason = %error,
                    event = %event_name,
                    "failed to record speech stage event"
                );
            }
        });
        Ok(())
    }
}
