use async_trait::async_trait;
use call_control_bot_speech::{
    BotSpeechArbiter, BotSpeechItem, BotSpeechPayload, CallAudioStateEventKind,
};
use call_runtime_control::{RuntimeEventFact, RuntimeFactKind};
use shared_kernel::{AppError, AppResult};

use crate::domain::{
    BotSpeechDispatchDecision, BotSpeechInterruptionDecision, BotSpeechState, BotSpeechStateEvent,
    BotSpeechStateTransitionResult, CallEvent, CallStatus,
};
use crate::ports::{
    BotSpeechRuntimeTriggerPort, BotSpeechStateStorePort, BuildServerInitiatedTurnCommand,
    EventSinkPort, RuntimeSessionStatePort, ServerInitiatedBotSpeechPort,
};
use call_control_orchestration::{
    ServerInitiatedTurnIntent, server_initiated_turn_intent_for_runtime_fact,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ConsumeBotSpeechRuntimeFactCommand {
    pub fact: RuntimeEventFact,
}

pub struct BotSpeechDependencies<R, S, E, T, B> {
    pub runtime_sessions: R,
    pub state_store: S,
    pub event_sink: E,
    pub runtime_trigger: T,
    pub server_initiated_turn: B,
}

pub struct BotSpeechService<R, S, E, T, B> {
    runtime_sessions: R,
    state_store: S,
    event_sink: E,
    runtime_trigger: T,
    server_initiated_turn: B,
    arbiter: BotSpeechArbiter,
}

impl<R, S, E, T, B> BotSpeechService<R, S, E, T, B> {
    pub fn new(deps: BotSpeechDependencies<R, S, E, T, B>) -> Self {
        Self {
            runtime_sessions: deps.runtime_sessions,
            state_store: deps.state_store,
            event_sink: deps.event_sink,
            runtime_trigger: deps.runtime_trigger,
            server_initiated_turn: deps.server_initiated_turn,
            arbiter: BotSpeechArbiter::default(),
        }
    }
}

enum RuntimeFactDisposition {
    Apply,
    AbsorbStale,
}

#[async_trait]
pub trait ConsumeBotSpeechRuntimeFactUseCases: Send + Sync {
    async fn consume_bot_speech_runtime_fact(
        &self,
        command: ConsumeBotSpeechRuntimeFactCommand,
    ) -> AppResult<()>;
}

#[async_trait]
impl<R, S, E, T, B> ConsumeBotSpeechRuntimeFactUseCases for BotSpeechService<R, S, E, T, B>
where
    R: RuntimeSessionStatePort + Send + Sync,
    S: BotSpeechStateStorePort + Send + Sync,
    E: EventSinkPort + Send + Sync,
    T: BotSpeechRuntimeTriggerPort + Send + Sync,
    B: ServerInitiatedBotSpeechPort + Send + Sync,
{
    async fn consume_bot_speech_runtime_fact(
        &self,
        command: ConsumeBotSpeechRuntimeFactCommand,
    ) -> AppResult<()> {
        let fact = command.fact;
        match self
            .runtime_fact_disposition(fact.session_id, &fact.runtime_owner_id, &fact.kind)
            .await?
        {
            RuntimeFactDisposition::Apply => {}
            RuntimeFactDisposition::AbsorbStale => return Ok(()),
        }

        let server_turn_intent =
            server_initiated_turn_intent_for_runtime_fact(fact.session_id, &fact.kind);

        match fact.kind {
            RuntimeFactKind::WorkerRuntimeReady => {
                let mut state = self.require_initialized_state(fact.session_id).await?;
                if let Some(intent) = server_turn_intent {
                    self.ensure_server_initiated_turn(
                        fact.session_id,
                        &fact.runtime_owner_id,
                        &mut state,
                        intent,
                    )
                    .await?;
                }
                self.dispatch_next_item(fact.session_id, &fact.runtime_owner_id, &mut state)
                    .await
            }
            RuntimeFactKind::WorkerRuntimeStopped
            | RuntimeFactKind::WorkerRuntimeFailed { .. }
            | RuntimeFactKind::WorkerRuntimeMissing => {
                self.handle_runtime_reset(fact.session_id).await
            }
            RuntimeFactKind::RuntimeReplyStarted { .. }
            | RuntimeFactKind::ExternalAudioStarted { .. } => {
                let Some(round_id) = fact.round_id.as_deref() else {
                    return Err(AppError::internal("runtime start fact missing round_id"));
                };
                self.handle_started(fact.session_id, round_id).await
            }
            RuntimeFactKind::RuntimePlaybackCompleted => {
                let Some(round_id) = fact.round_id.as_deref() else {
                    return Err(AppError::internal(
                        "runtime completion fact missing round_id",
                    ));
                };
                self.handle_completed(fact.session_id, &fact.runtime_owner_id, round_id)
                    .await
            }
            RuntimeFactKind::ExternalAudioFinished { .. } => {
                let Some(round_id) = fact.round_id.as_deref() else {
                    return Err(AppError::internal(
                        "runtime completion fact missing round_id",
                    ));
                };
                self.handle_completed(fact.session_id, &fact.runtime_owner_id, round_id)
                    .await
            }
            RuntimeFactKind::WorkerBargeInDetected { .. } => {
                self.handle_barge_in(fact.session_id, &fact.runtime_owner_id)
                    .await
            }
            RuntimeFactKind::WorkerRuntimeStarted
            | RuntimeFactKind::RuntimeReplyFinished
            | RuntimeFactKind::RuntimeInputRoundFailed { .. } => Ok(()),
        }
    }
}

impl<R, S, E, T, B> BotSpeechService<R, S, E, T, B>
where
    R: RuntimeSessionStatePort + Send + Sync,
    S: BotSpeechStateStorePort + Send + Sync,
    E: EventSinkPort + Send + Sync,
    T: BotSpeechRuntimeTriggerPort + Send + Sync,
    B: ServerInitiatedBotSpeechPort + Send + Sync,
{
    async fn runtime_fact_disposition(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        kind: &RuntimeFactKind,
    ) -> AppResult<RuntimeFactDisposition> {
        let Some(state) = self
            .runtime_sessions
            .get_runtime_session_state(session_id)
            .await?
        else {
            return Err(AppError::not_found("runtime session state not found"));
        };
        if state.runtime_owner_id.as_deref() != Some(runtime_owner_id) {
            tracing::debug!(
                session_id,
                runtime_owner_id,
                current_runtime_owner_id = state.runtime_owner_id.as_deref().unwrap_or(""),
                fact_kind = kind.as_str(),
                "absorbing stale bot speech runtime fact for non-current owner"
            );
            return Ok(RuntimeFactDisposition::AbsorbStale);
        }
        if matches!(
            state.status,
            CallStatus::Ending | CallStatus::Ended | CallStatus::Failed
        ) {
            tracing::debug!(
                session_id,
                runtime_owner_id,
                call_status = state.status.as_str(),
                runtime_status = state.runtime_status.as_str(),
                fact_kind = kind.as_str(),
                "absorbing stale bot speech runtime fact for terminal call session"
            );
            return Ok(RuntimeFactDisposition::AbsorbStale);
        }
        Ok(RuntimeFactDisposition::Apply)
    }

    async fn require_initialized_state(&self, session_id: i64) -> AppResult<BotSpeechState> {
        let state = self
            .state_store
            .load_state(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("bot speech state not found"))?;
        if !state.initialized {
            return Err(AppError::conflict("bot speech state is not initialized"));
        }
        Ok(state)
    }

    async fn dispatch_next_item(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        state: &mut BotSpeechState,
    ) -> AppResult<()> {
        let decision = dispatch_decision(state);
        let BotSpeechDispatchDecision::Dispatch(item) = decision else {
            return Ok(());
        };

        self.dispatch_item(session_id, runtime_owner_id, &item)
            .await?;

        let transition = apply_dispatch(state, item);
        *state = transition.state;
        self.state_store.save_state(session_id, state).await?;
        self.publish_state_events(session_id, &transition.events)
            .await
    }

    async fn ensure_server_initiated_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        state: &mut BotSpeechState,
        intent: ServerInitiatedTurnIntent,
    ) -> AppResult<()> {
        if bot_speech_state_contains_item(state, &intent.item_id) {
            return Ok(());
        }

        let item = self
            .server_initiated_turn
            .build_server_initiated_turn(BuildServerInitiatedTurnCommand {
                session_id,
                runtime_owner_id: runtime_owner_id.to_owned(),
                trigger: intent.trigger,
            })
            .await?;
        if item.item_id != intent.item_id {
            return Err(AppError::internal("server initiated turn item_id mismatch"));
        }
        if bot_speech_state_contains_item(state, &intent.item_id) {
            return Ok(());
        }

        remember_handled_item(state, &intent.item_id);
        state.pending_items.push(item.clone());
        state.pending_items = self
            .arbiter
            .order_items_for_playback(std::mem::take(&mut state.pending_items));
        self.publish_state_events(session_id, &[BotSpeechStateEvent::Enqueued { item }])
            .await?;
        self.state_store.save_state(session_id, state).await
    }

    async fn dispatch_item(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        item: &BotSpeechItem,
    ) -> AppResult<()> {
        match &item.payload {
            BotSpeechPayload::Text { text } => {
                self.runtime_trigger
                    .submit_server_turn(
                        session_id,
                        runtime_owner_id,
                        &item.item_id,
                        text,
                        item.is_cancelable_on_user_barge_in(),
                    )
                    .await
            }
            BotSpeechPayload::PreRecorded { audio_url, band } => {
                self.runtime_trigger
                    .submit_pre_recorded_turn(
                        session_id,
                        runtime_owner_id,
                        &item.item_id,
                        audio_url,
                        band,
                        item.is_cancelable_on_user_barge_in(),
                    )
                    .await
            }
        }
    }

    async fn handle_started(&self, session_id: i64, round_id: &str) -> AppResult<()> {
        let mut state = self.require_bot_speech_state(session_id).await?;
        let transition = mark_started(state, round_id);
        if transition.events.is_empty() {
            return Ok(());
        }
        self.publish_state_events(session_id, &transition.events)
            .await?;
        state = transition.state;
        self.state_store.save_state(session_id, &state).await
    }

    async fn handle_completed(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
    ) -> AppResult<()> {
        let mut state = self.require_bot_speech_state(session_id).await?;
        let transition = mark_completed(state, round_id);
        if transition.events.is_empty() {
            return Ok(());
        }
        self.publish_state_events(session_id, &transition.events)
            .await?;
        state = transition.state;
        self.state_store.save_state(session_id, &state).await?;
        self.dispatch_next_item(session_id, runtime_owner_id, &mut state)
            .await
    }

    async fn handle_barge_in(&self, session_id: i64, runtime_owner_id: &str) -> AppResult<()> {
        let mut state = self.require_bot_speech_state(session_id).await?;
        let transition = flush_on_barge_in(&self.arbiter, state);
        if transition.events.is_empty() {
            return Ok(());
        }
        self.publish_state_events(session_id, &transition.events)
            .await?;
        state = transition.state;
        self.state_store.save_state(session_id, &state).await?;
        self.dispatch_next_item(session_id, runtime_owner_id, &mut state)
            .await
    }

    async fn handle_runtime_reset(&self, session_id: i64) -> AppResult<()> {
        let runtime_state = self
            .runtime_sessions
            .get_runtime_session_state(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("runtime session state not found"))?;
        let Some(mut state) = self.state_store.load_state(session_id).await? else {
            if matches!(
                runtime_state.status,
                CallStatus::Ending | CallStatus::Ended | CallStatus::Failed
            ) {
                return Ok(());
            }
            return Err(AppError::not_found("bot speech state not found"));
        };

        let Some(active_item) = state.active_item.take() else {
            return Ok(());
        };
        state
            .started_item_ids
            .retain(|item_id| item_id != &active_item.item_id);
        state
            .completed_item_ids
            .retain(|item_id| item_id != &active_item.item_id);
        if !state
            .pending_items
            .iter()
            .any(|item| item.item_id == active_item.item_id)
        {
            state.pending_items.insert(0, active_item);
        }
        self.state_store.save_state(session_id, &state).await
    }

    async fn require_bot_speech_state(&self, session_id: i64) -> AppResult<BotSpeechState> {
        self.state_store
            .load_state(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("bot speech state not found"))
    }

    async fn publish_state_events(
        &self,
        session_id: i64,
        events: &[BotSpeechStateEvent],
    ) -> AppResult<()> {
        for event in events {
            match event {
                BotSpeechStateEvent::Initialized => {}
                BotSpeechStateEvent::Enqueued { item } => {
                    self.publish_item_event(
                        session_id,
                        item,
                        CallAudioStateEventKind::BotSpeechEnqueued,
                    )
                    .await?;
                }
                BotSpeechStateEvent::Started { item } => {
                    self.publish_item_event(
                        session_id,
                        item,
                        CallAudioStateEventKind::BotSpeechStarted,
                    )
                    .await?;
                }
                BotSpeechStateEvent::Completed { item } => {
                    self.publish_item_event(
                        session_id,
                        item,
                        CallAudioStateEventKind::BotSpeechCompleted,
                    )
                    .await?;
                }
                BotSpeechStateEvent::Interrupted { item } => {
                    self.publish_item_event(
                        session_id,
                        item,
                        CallAudioStateEventKind::BotSpeechInterrupted,
                    )
                    .await?;
                }
                BotSpeechStateEvent::Cancelled { item } => {
                    self.publish_item_event(
                        session_id,
                        item,
                        CallAudioStateEventKind::BotSpeechCancelled,
                    )
                    .await?;
                }
                BotSpeechStateEvent::QueueFlushed { item_ids } => {
                    self.event_sink
                        .publish(CallEvent {
                            session_id,
                            round_id: None,
                            source: "call_control".to_owned(),
                            event: CallAudioStateEventKind::QueueFlushed.as_str().to_owned(),
                            ts_ms: chrono::Utc::now().timestamp_millis(),
                            fields: serde_json::json!({
                                "flushed_item_ids": item_ids,
                            }),
                        })
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn publish_item_event(
        &self,
        session_id: i64,
        item: &BotSpeechItem,
        event: CallAudioStateEventKind,
    ) -> AppResult<()> {
        self.event_sink
            .publish(CallEvent {
                session_id,
                round_id: Some(item.item_id.clone()),
                source: "call_control".to_owned(),
                event: event.as_str().to_owned(),
                ts_ms: chrono::Utc::now().timestamp_millis(),
                fields: serde_json::json!({
                    "source_type": item.source_type.as_str(),
                    "trigger_type": item.trigger_type.as_str(),
                    "priority": item.priority.as_str(),
                    "interruptibility": item.interruptibility.as_str(),
                }),
            })
            .await
    }
}

fn dispatch_decision(state: &BotSpeechState) -> BotSpeechDispatchDecision {
    if state.active_item.is_some() || state.pending_items.is_empty() {
        return BotSpeechDispatchDecision::None;
    }
    BotSpeechDispatchDecision::Dispatch(state.pending_items[0].clone())
}

fn apply_dispatch(state: &BotSpeechState, item: BotSpeechItem) -> BotSpeechStateTransitionResult {
    let mut next = state.clone();
    next.pending_items.remove(0);
    next.active_item = Some(item.clone());
    BotSpeechStateTransitionResult {
        state: next,
        dispatch: BotSpeechDispatchDecision::Dispatch(item.clone()),
        events: Vec::new(),
    }
}

fn mark_started(state: BotSpeechState, round_id: &str) -> BotSpeechStateTransitionResult {
    let Some(active_item) = state
        .active_item
        .as_ref()
        .filter(|item| item.item_id == round_id)
        .cloned()
    else {
        return BotSpeechStateTransitionResult {
            state,
            dispatch: BotSpeechDispatchDecision::None,
            events: Vec::new(),
        };
    };
    if state
        .started_item_ids
        .iter()
        .any(|item_id| item_id == round_id)
    {
        return BotSpeechStateTransitionResult {
            state,
            dispatch: BotSpeechDispatchDecision::None,
            events: Vec::new(),
        };
    }

    let mut next = state;
    next.started_item_ids.push(round_id.to_owned());
    BotSpeechStateTransitionResult {
        state: next,
        dispatch: BotSpeechDispatchDecision::None,
        events: vec![BotSpeechStateEvent::Started { item: active_item }],
    }
}

fn mark_completed(state: BotSpeechState, round_id: &str) -> BotSpeechStateTransitionResult {
    let Some(active_item) = state
        .active_item
        .as_ref()
        .filter(|item| item.item_id == round_id)
        .cloned()
    else {
        return BotSpeechStateTransitionResult {
            state,
            dispatch: BotSpeechDispatchDecision::None,
            events: Vec::new(),
        };
    };

    let mut next = state;
    next.active_item = None;
    remember_handled_item(&mut next, &active_item.item_id);
    if !next
        .completed_item_ids
        .iter()
        .any(|item_id| item_id == &active_item.item_id)
    {
        next.completed_item_ids.push(active_item.item_id.clone());
    }
    BotSpeechStateTransitionResult {
        state: next,
        dispatch: BotSpeechDispatchDecision::None,
        events: vec![BotSpeechStateEvent::Completed { item: active_item }],
    }
}

fn flush_on_barge_in(
    arbiter: &BotSpeechArbiter,
    state: BotSpeechState,
) -> BotSpeechStateTransitionResult {
    let decision = interruption_decision(arbiter, &state);
    if decision.flushed_item_ids.is_empty() {
        return BotSpeechStateTransitionResult {
            state,
            dispatch: BotSpeechDispatchDecision::None,
            events: Vec::new(),
        };
    }

    let mut next = state;
    if decision.interrupted_active.is_some() {
        next.active_item = None;
    }
    next.pending_items.retain(|item| {
        !decision
            .cancelled_pending
            .iter()
            .any(|cancelled| cancelled.item_id == item.item_id)
    });

    let mut events = Vec::new();
    if let Some(active_item) = decision.interrupted_active {
        remember_handled_item(&mut next, &active_item.item_id);
        events.push(BotSpeechStateEvent::Interrupted { item: active_item });
    }
    for item in decision.cancelled_pending {
        remember_handled_item(&mut next, &item.item_id);
        events.push(BotSpeechStateEvent::Cancelled { item });
    }
    for item_id in &decision.flushed_item_ids {
        remember_handled_item(&mut next, item_id);
    }
    events.push(BotSpeechStateEvent::QueueFlushed {
        item_ids: decision.flushed_item_ids,
    });

    BotSpeechStateTransitionResult {
        state: next,
        dispatch: BotSpeechDispatchDecision::None,
        events,
    }
}

fn interruption_decision(
    arbiter: &BotSpeechArbiter,
    state: &BotSpeechState,
) -> BotSpeechInterruptionDecision {
    let decision = arbiter.decide_user_barge_in(state.active_item.as_ref(), &state.pending_items);
    let interrupted_active = if decision.interrupt_active_item {
        state.active_item.clone()
    } else {
        None
    };
    let cancelled_pending = state
        .pending_items
        .iter()
        .filter(|item| {
            decision
                .flushed_pending_item_ids
                .iter()
                .any(|item_id| item_id == &item.item_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    BotSpeechInterruptionDecision {
        interrupted_active,
        cancelled_pending,
        flushed_item_ids: decision
            .flushed_pending_item_ids
            .into_iter()
            .chain(
                state
                    .active_item
                    .as_ref()
                    .filter(|_| decision.interrupt_active_item)
                    .map(|item| item.item_id.clone()),
            )
            .collect(),
    }
}

fn bot_speech_state_contains_item(state: &BotSpeechState, item_id: &str) -> bool {
    state
        .active_item
        .as_ref()
        .is_some_and(|item| item.item_id == item_id)
        || state
            .handled_item_ids
            .iter()
            .any(|handled_item_id| handled_item_id == item_id)
        || state
            .pending_items
            .iter()
            .any(|item| item.item_id == item_id)
        || state
            .started_item_ids
            .iter()
            .any(|started_item_id| started_item_id == item_id)
        || state
            .completed_item_ids
            .iter()
            .any(|completed_item_id| completed_item_id == item_id)
}

fn remember_handled_item(state: &mut BotSpeechState, item_id: &str) {
    if !state
        .handled_item_ids
        .iter()
        .any(|handled_item_id| handled_item_id == item_id)
    {
        state.handled_item_ids.push(item_id.to_owned());
    }
}

#[async_trait]
impl<T> ConsumeBotSpeechRuntimeFactUseCases for std::sync::Arc<T>
where
    T: ConsumeBotSpeechRuntimeFactUseCases + Send + Sync + ?Sized,
{
    async fn consume_bot_speech_runtime_fact(
        &self,
        command: ConsumeBotSpeechRuntimeFactCommand,
    ) -> AppResult<()> {
        (**self).consume_bot_speech_runtime_fact(command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RuntimeWorkStatus, SpeechInputLanguage};
    use crate::ports::RuntimeSessionState;
    use call_control_bot_speech::{
        BotSpeechInterruptibility, BotSpeechPayload, BotSpeechPriority, BotSpeechSourceType,
        BotSpeechTriggerType,
    };
    use call_control_orchestration::build_server_initiated_bot_speech_item;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard};

    #[derive(Default)]
    struct StubRuntimeSessions {
        states: Mutex<HashMap<i64, RuntimeSessionState>>,
    }

    #[derive(Default)]
    struct StubBotSpeechStateStore {
        states: Mutex<HashMap<i64, BotSpeechState>>,
    }

    #[derive(Default)]
    struct StubEventSink {
        events: Mutex<Vec<CallEvent>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SubmittedTextTurn {
        session_id: i64,
        runtime_owner_id: String,
        round_id: String,
        reply_text: String,
        interruptible: bool,
    }

    #[derive(Default)]
    struct StubRuntimeTrigger {
        text_turns: Mutex<Vec<SubmittedTextTurn>>,
        text_turn_error: Mutex<Option<String>>,
    }
    #[derive(Default)]
    struct StubServerInitiatedTurn;

    type TestBotSpeechService = BotSpeechService<
        Arc<StubRuntimeSessions>,
        Arc<StubBotSpeechStateStore>,
        Arc<StubEventSink>,
        Arc<StubRuntimeTrigger>,
        Arc<StubServerInitiatedTurn>,
    >;

    type TestServiceParts = (
        TestBotSpeechService,
        Arc<StubBotSpeechStateStore>,
        Arc<StubEventSink>,
        Arc<StubRuntimeTrigger>,
        Arc<StubServerInitiatedTurn>,
    );

    fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn require_some<T>(value: Option<T>) -> AppResult<T> {
        value.ok_or_else(|| AppError::internal("expected Some(..), got None"))
    }

    #[async_trait]
    impl RuntimeSessionStatePort for StubRuntimeSessions {
        async fn get_runtime_session_state(
            &self,
            session_id: i64,
        ) -> AppResult<Option<RuntimeSessionState>> {
            Ok(lock_or_recover(&self.states).get(&session_id).cloned())
        }
    }

    #[async_trait]
    impl BotSpeechStateStorePort for StubBotSpeechStateStore {
        async fn load_state(&self, session_id: i64) -> AppResult<Option<BotSpeechState>> {
            Ok(lock_or_recover(&self.states).get(&session_id).cloned())
        }

        async fn save_state(&self, session_id: i64, state: &BotSpeechState) -> AppResult<()> {
            lock_or_recover(&self.states).insert(session_id, state.clone());
            Ok(())
        }
    }

    #[async_trait]
    impl EventSinkPort for StubEventSink {
        async fn publish(&self, event: CallEvent) -> AppResult<()> {
            lock_or_recover(&self.events).push(event);
            Ok(())
        }
    }

    #[async_trait]
    impl BotSpeechRuntimeTriggerPort for StubRuntimeTrigger {
        async fn submit_server_turn(
            &self,
            session_id: i64,
            runtime_owner_id: &str,
            round_id: &str,
            reply_text: &str,
            interruptible: bool,
        ) -> AppResult<()> {
            if let Some(message) = lock_or_recover(&self.text_turn_error).clone() {
                return Err(AppError::unavailable(message));
            }
            lock_or_recover(&self.text_turns).push(SubmittedTextTurn {
                session_id,
                runtime_owner_id: runtime_owner_id.to_owned(),
                round_id: round_id.to_owned(),
                reply_text: reply_text.to_owned(),
                interruptible,
            });
            Ok(())
        }

        async fn submit_pre_recorded_turn(
            &self,
            _session_id: i64,
            _runtime_owner_id: &str,
            _round_id: &str,
            _audio_url: &str,
            _band: &str,
            _interruptible: bool,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ServerInitiatedBotSpeechPort for StubServerInitiatedTurn {
        async fn build_server_initiated_turn(
            &self,
            command: BuildServerInitiatedTurnCommand,
        ) -> AppResult<BotSpeechItem> {
            Ok(build_server_initiated_bot_speech_item(
                command.session_id,
                command.trigger,
                "欢迎",
            ))
        }
    }

    #[tokio::test]
    async fn terminal_session_runtime_ready_is_absorbed_without_dispatch() -> AppResult<()> {
        for status in [CallStatus::Ending, CallStatus::Ended, CallStatus::Failed] {
            let (service, state_store, event_sink, runtime_trigger, _) =
                service_with_session(status);
            service
                .consume_bot_speech_runtime_fact(ConsumeBotSpeechRuntimeFactCommand {
                    fact: runtime_fact(1, RuntimeFactKind::WorkerRuntimeReady),
                })
                .await?;

            assert!(lock_or_recover(&runtime_trigger.text_turns).is_empty());
            assert!(lock_or_recover(&event_sink.events).is_empty());
            let state = require_some(lock_or_recover(&state_store.states).get(&1).cloned())?;
            assert!(state.active_item.is_none());
            assert_eq!(state.pending_items.len(), 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn active_session_runtime_ready_dispatches_next_bot_speech_item() -> AppResult<()> {
        let (service, state_store, event_sink, runtime_trigger, _) =
            service_with_session(CallStatus::Active);
        service
            .consume_bot_speech_runtime_fact(ConsumeBotSpeechRuntimeFactCommand {
                fact: runtime_fact(1, RuntimeFactKind::WorkerRuntimeReady),
            })
            .await?;

        assert_eq!(
            lock_or_recover(&runtime_trigger.text_turns).as_slice(),
            &[SubmittedTextTurn {
                session_id: 1,
                runtime_owner_id: "worker-a".to_owned(),
                round_id: "item-1".to_owned(),
                reply_text: "欢迎".to_owned(),
                interruptible: true,
            }]
        );
        assert_eq!(lock_or_recover(&event_sink.events).len(), 1);
        let state = require_some(lock_or_recover(&state_store.states).get(&1).cloned())?;
        assert_eq!(
            state.active_item.as_ref().map(|item| item.item_id.as_str()),
            Some("item-1")
        );
        assert_eq!(state.pending_items.len(), 1);
        assert_eq!(state.pending_items[0].item_id, "server-turn-call_started-1");
        Ok(())
    }

    #[tokio::test]
    async fn failed_runtime_dispatch_does_not_persist_fake_active_item() -> AppResult<()> {
        let (service, state_store, event_sink, runtime_trigger, _) =
            service_with_session(CallStatus::Active);
        *lock_or_recover(&runtime_trigger.text_turn_error) =
            Some("speech runtime unavailable".to_owned());

        let result = service
            .consume_bot_speech_runtime_fact(ConsumeBotSpeechRuntimeFactCommand {
                fact: runtime_fact(1, RuntimeFactKind::WorkerRuntimeReady),
            })
            .await;

        assert!(result.is_err());
        assert!(lock_or_recover(&runtime_trigger.text_turns).is_empty());
        assert_eq!(lock_or_recover(&event_sink.events).len(), 1);
        let state = require_some(lock_or_recover(&state_store.states).get(&1).cloned())?;
        assert!(state.active_item.is_none());
        assert_eq!(
            state
                .pending_items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>(),
            vec!["item-1", "server-turn-call_started-1"]
        );
        Ok(())
    }

    fn service_with_session(status: CallStatus) -> TestServiceParts {
        let runtime_sessions = Arc::new(StubRuntimeSessions::default());
        lock_or_recover(&runtime_sessions.states).insert(
            1,
            RuntimeSessionState {
                session_id: 1,
                agent_session_id: "agent-session-1".to_owned(),
                voice: "voice-1".to_owned(),
                asr_language: SpeechInputLanguage::Zh,
                runtime_owner_id: Some("worker-a".to_owned()),
                status,
                runtime_status: match status {
                    CallStatus::Ending => RuntimeWorkStatus::StopRequested,
                    CallStatus::Ended => RuntimeWorkStatus::Stopped,
                    CallStatus::Failed => RuntimeWorkStatus::Failed,
                    _ => RuntimeWorkStatus::Ready,
                },
            },
        );

        let state_store = Arc::new(StubBotSpeechStateStore::default());
        lock_or_recover(&state_store.states).insert(
            1,
            BotSpeechState::initialized_with(vec![text_item("item-1")]),
        );
        let event_sink = Arc::new(StubEventSink::default());
        let runtime_trigger = Arc::new(StubRuntimeTrigger::default());
        let server_initiated_turn = Arc::new(StubServerInitiatedTurn);
        let service = BotSpeechService::new(BotSpeechDependencies {
            runtime_sessions,
            state_store: state_store.clone(),
            event_sink: event_sink.clone(),
            runtime_trigger: runtime_trigger.clone(),
            server_initiated_turn: server_initiated_turn.clone(),
        });
        (
            service,
            state_store,
            event_sink,
            runtime_trigger,
            server_initiated_turn,
        )
    }

    fn text_item(item_id: &str) -> BotSpeechItem {
        BotSpeechItem {
            session_id: 1,
            item_id: item_id.to_owned(),
            source_type: BotSpeechSourceType::ReplyToUser,
            trigger_type: BotSpeechTriggerType::UserTurnReply,
            priority: BotSpeechPriority::ReplyToUser,
            interruptibility: BotSpeechInterruptibility::Interruptible,
            cancel_on_user_barge_in: true,
            payload: BotSpeechPayload::Text {
                text: "欢迎".to_owned(),
            },
        }
    }

    fn runtime_fact(session_id: i64, kind: RuntimeFactKind) -> RuntimeEventFact {
        RuntimeEventFact {
            session_id,
            runtime_owner_id: "worker-a".to_owned(),
            round_id: None,
            source: "worker".to_owned(),
            ts_ms: 1,
            kind,
        }
    }
}
