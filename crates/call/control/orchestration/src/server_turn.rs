use async_trait::async_trait;
use call_control_bot_speech::{
    BotSpeechItem, BotSpeechPriority, BotSpeechSourceType, BotSpeechTriggerType,
};
use call_runtime_control::RuntimeFactKind;
use shared_kernel::AppResult;

use crate::request::{ServerInitiatedTurnRequest, ServerInitiatedTurnTrigger};

#[async_trait]
pub trait ServerInitiatedTurnPort: Send + Sync {
    async fn generate_turn(&self, request: &ServerInitiatedTurnRequest) -> AppResult<String>;
}

pub fn item_id_for_trigger(trigger: &ServerInitiatedTurnTrigger, session_id: i64) -> String {
    format!("server-turn-{}-{session_id}", trigger.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInitiatedTurnIntent {
    pub trigger: ServerInitiatedTurnTrigger,
    pub item_id: String,
}

pub fn server_initiated_turn_intent_for_runtime_fact(
    session_id: i64,
    kind: &RuntimeFactKind,
) -> Option<ServerInitiatedTurnIntent> {
    let trigger = match kind {
        RuntimeFactKind::WorkerRuntimeReady => ServerInitiatedTurnTrigger::CallStarted,
        _ => return None,
    };
    Some(ServerInitiatedTurnIntent {
        item_id: item_id_for_trigger(&trigger, session_id),
        trigger,
    })
}

pub fn build_server_initiated_bot_speech_item(
    session_id: i64,
    trigger: ServerInitiatedTurnTrigger,
    text: impl Into<String>,
) -> BotSpeechItem {
    BotSpeechItem::text(
        session_id,
        item_id_for_trigger(&trigger, session_id),
        BotSpeechSourceType::ReplyToServer,
        match trigger {
            ServerInitiatedTurnTrigger::CallStarted => BotSpeechTriggerType::CallStarted,
        },
        BotSpeechPriority::ReplyToServer,
        text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use call_control_bot_speech::{BotSpeechPayload, BotSpeechPriority, BotSpeechSourceType};

    #[test]
    fn worker_runtime_ready_maps_to_call_started_turn_intent() {
        assert_eq!(
            server_initiated_turn_intent_for_runtime_fact(42, &RuntimeFactKind::WorkerRuntimeReady),
            Some(ServerInitiatedTurnIntent {
                trigger: ServerInitiatedTurnTrigger::CallStarted,
                item_id: "server-turn-call_started-42".to_owned(),
            })
        );
    }

    #[test]
    fn non_server_trigger_runtime_fact_has_no_turn_intent() {
        assert_eq!(
            server_initiated_turn_intent_for_runtime_fact(
                42,
                &RuntimeFactKind::RuntimeReplyFinished
            ),
            None
        );
    }

    #[test]
    fn call_started_server_turn_builds_text_item() {
        let item = build_server_initiated_bot_speech_item(
            42,
            ServerInitiatedTurnTrigger::CallStarted,
            "hi",
        );

        assert_eq!(item.item_id, "server-turn-call_started-42");
        assert_eq!(item.source_type, BotSpeechSourceType::ReplyToServer);
        assert_eq!(item.priority, BotSpeechPriority::ReplyToServer);
        assert_eq!(
            item.payload,
            BotSpeechPayload::Text {
                text: "hi".to_owned()
            }
        );
    }
}
