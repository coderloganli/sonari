use call_control_bot_speech::BotSpeechItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BotSpeechState {
    pub initialized: bool,
    pub active_item: Option<BotSpeechItem>,
    pub pending_items: Vec<BotSpeechItem>,
    #[serde(default)]
    pub handled_item_ids: Vec<String>,
    pub started_item_ids: Vec<String>,
    pub completed_item_ids: Vec<String>,
}

impl BotSpeechState {
    pub fn initialized_with(items: Vec<BotSpeechItem>) -> Self {
        Self {
            initialized: true,
            active_item: None,
            pending_items: items,
            handled_item_ids: Vec::new(),
            started_item_ids: Vec::new(),
            completed_item_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotSpeechStateEvent {
    Initialized,
    Enqueued { item: BotSpeechItem },
    Started { item: BotSpeechItem },
    Completed { item: BotSpeechItem },
    Interrupted { item: BotSpeechItem },
    Cancelled { item: BotSpeechItem },
    QueueFlushed { item_ids: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotSpeechDispatchDecision {
    None,
    Dispatch(BotSpeechItem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotSpeechStateTransitionResult {
    pub state: BotSpeechState,
    pub dispatch: BotSpeechDispatchDecision,
    pub events: Vec<BotSpeechStateEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BotSpeechInterruptionDecision {
    pub interrupted_active: Option<BotSpeechItem>,
    pub cancelled_pending: Vec<BotSpeechItem>,
    pub flushed_item_ids: Vec<String>,
}
