//! Defines business-owned bot speech queue semantics for call audio orchestration.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechSourceType {
    ReplyToUser,
    ReplyToServer,
    PreRecorded,
}

impl BotSpeechSourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReplyToUser => "reply_to_user",
            Self::ReplyToServer => "reply_to_server",
            Self::PreRecorded => "pre_recorded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechTriggerType {
    UserTurnReply,
    CallStarted,
    ServerRequest,
    FixedAsset,
}

impl BotSpeechTriggerType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserTurnReply => "user_turn_reply",
            Self::CallStarted => "call_started",
            Self::ServerRequest => "server_request",
            Self::FixedAsset => "fixed_asset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechPriority {
    ReplyToUser,
    ReplyToServer,
    PreRecorded,
}

impl BotSpeechPriority {
    pub fn rank(self) -> u8 {
        match self {
            Self::ReplyToUser => 3,
            Self::ReplyToServer => 2,
            Self::PreRecorded => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReplyToUser => "reply_to_user",
            Self::ReplyToServer => "reply_to_server",
            Self::PreRecorded => "pre_recorded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechInterruptibility {
    Interruptible,
    NonInterruptible,
}

impl BotSpeechInterruptibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interruptible => "interruptible",
            Self::NonInterruptible => "non_interruptible",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechCancellationReason {
    UserBargeIn,
    QueueFlushed,
}

impl BotSpeechCancellationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserBargeIn => "user_barge_in",
            Self::QueueFlushed => "queue_flushed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BotSpeechQueueFlushScope {
    CancelablePendingItems,
}

impl BotSpeechQueueFlushScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CancelablePendingItems => "cancelable_pending_items",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotSpeechQueuePolicy {
    pub drop_pending_pre_recorded_on_speech_arrival: bool,
    pub clear_queued_audio_on_interrupt: bool,
    pub user_barge_in_flush_scope: BotSpeechQueueFlushScope,
}

impl Default for BotSpeechQueuePolicy {
    fn default() -> Self {
        Self {
            drop_pending_pre_recorded_on_speech_arrival: true,
            clear_queued_audio_on_interrupt: true,
            user_barge_in_flush_scope: BotSpeechQueueFlushScope::CancelablePendingItems,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BotSpeechPayload {
    Text { text: String },
    PreRecorded { audio_url: String, band: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotSpeechItem {
    pub session_id: i64,
    pub item_id: String,
    pub source_type: BotSpeechSourceType,
    pub trigger_type: BotSpeechTriggerType,
    pub priority: BotSpeechPriority,
    pub interruptibility: BotSpeechInterruptibility,
    pub cancel_on_user_barge_in: bool,
    pub payload: BotSpeechPayload,
}

impl BotSpeechItem {
    pub fn is_cancelable_on_user_barge_in(&self) -> bool {
        self.cancel_on_user_barge_in
            && matches!(
                self.interruptibility,
                BotSpeechInterruptibility::Interruptible
            )
    }
}

impl BotSpeechItem {
    pub fn text(
        session_id: i64,
        item_id: impl Into<String>,
        source_type: BotSpeechSourceType,
        trigger_type: BotSpeechTriggerType,
        priority: BotSpeechPriority,
        text: impl Into<String>,
    ) -> Self {
        Self {
            session_id,
            item_id: item_id.into(),
            source_type,
            trigger_type,
            priority,
            interruptibility: BotSpeechInterruptibility::Interruptible,
            cancel_on_user_barge_in: true,
            payload: BotSpeechPayload::Text { text: text.into() },
        }
    }

    pub fn pre_recorded(
        session_id: i64,
        item_id: impl Into<String>,
        audio_url: impl Into<String>,
        band: impl Into<String>,
    ) -> Self {
        Self {
            session_id,
            item_id: item_id.into(),
            source_type: BotSpeechSourceType::PreRecorded,
            trigger_type: BotSpeechTriggerType::FixedAsset,
            priority: BotSpeechPriority::PreRecorded,
            interruptibility: BotSpeechInterruptibility::Interruptible,
            cancel_on_user_barge_in: true,
            payload: BotSpeechPayload::PreRecorded {
                audio_url: audio_url.into(),
                band: band.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallAudioStateEventKind {
    BotSpeechEnqueued,
    BotSpeechStarted,
    BotSpeechCompleted,
    BotSpeechInterrupted,
    BotSpeechCancelled,
    QueueFlushed,
}

impl CallAudioStateEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BotSpeechEnqueued => "bot_speech_enqueued",
            Self::BotSpeechStarted => "bot_speech_started",
            Self::BotSpeechCompleted => "bot_speech_completed",
            Self::BotSpeechInterrupted => "bot_speech_interrupted",
            Self::BotSpeechCancelled => "bot_speech_cancelled",
            Self::QueueFlushed => "queue_flushed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBargeInDecision {
    pub interrupt_active_item: bool,
    pub flushed_pending_item_ids: Vec<String>,
    pub flush_scope: BotSpeechQueueFlushScope,
    pub reason: BotSpeechCancellationReason,
}

#[derive(Debug, Clone, Default)]
pub struct BotSpeechArbiter {
    queue_policy: BotSpeechQueuePolicy,
}

impl BotSpeechArbiter {
    pub fn new(queue_policy: BotSpeechQueuePolicy) -> Self {
        Self { queue_policy }
    }

    pub fn queue_policy(&self) -> BotSpeechQueuePolicy {
        self.queue_policy
    }

    pub fn order_items_for_playback(&self, mut items: Vec<BotSpeechItem>) -> Vec<BotSpeechItem> {
        items.sort_by_key(|item| Reverse(item.priority.rank()));
        items
    }

    pub fn decide_user_barge_in(
        &self,
        active_item: Option<&BotSpeechItem>,
        pending_items: &[BotSpeechItem],
    ) -> UserBargeInDecision {
        let interrupt_active_item = match active_item {
            Some(item) => item.is_cancelable_on_user_barge_in(),
            None => true,
        };
        let flushed_pending_item_ids = pending_items
            .iter()
            .filter(|item| item.is_cancelable_on_user_barge_in())
            .map(|item| item.item_id.clone())
            .collect();

        UserBargeInDecision {
            interrupt_active_item,
            flushed_pending_item_ids,
            flush_scope: self.queue_policy.user_barge_in_flush_scope,
            reason: BotSpeechCancellationReason::UserBargeIn,
        }
    }
}
