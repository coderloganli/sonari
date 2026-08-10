pub mod bot_speech;

use async_trait::async_trait;
use character_context::CharacterCallContextReadPort;
use chrono::Utc;
use rtc_control_contract::RealtimeJoinArtifactsPort;
use serde::Serialize;
use shared_kernel::{AppError, AppResult};

use crate::domain::{
    BotSpeechState, CallCallerIdentity, CallEvent, CallSession, CallStatus, CallType,
    RuntimeWorkStatus,
};
use crate::ports::{
    AgentSessionPort, BotSpeechStateStorePort, CallExecutionControlUseCases, CallInputLanguagePort,
    CallSessionRepository, CallTransactionRunner, CallUserContextPort, ClaimRuntimeStartCommand,
    ClaimRuntimeStopCommand, InstanceIdentityPort, MarkRuntimeFailedCommand,
    MarkRuntimeMissingCommand, MarkRuntimeReadyCommand, MarkRuntimeStoppedCommand,
    RuntimeOwnerPort, RuntimeWorkDescriptor, RuntimeWorkDescriptorKind,
};

pub use bot_speech::{
    BotSpeechDependencies, BotSpeechService, ConsumeBotSpeechRuntimeFactCommand,
    ConsumeBotSpeechRuntimeFactUseCases,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartCallCommand {
    pub user_id: i64,
    pub character_id: i64,
    pub scene_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndCallCommand {
    pub user_id: i64,
    pub session_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceIdentity {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartCallResponse {
    pub session_id: i64,
    pub owner_instance: String,
    pub realtime: rtc_control_contract::RealtimeJoinArtifacts,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallHistoryItemView {
    pub session_id: i64,
    pub character_id: i64,
    pub status: String,
    pub owner_instance: String,
    pub started_at: chrono::DateTime<Utc>,
    pub ended_at: Option<chrono::DateTime<Utc>>,
    pub duration_seconds: i32,
}

pub struct CallDependencies<S, T, C, A, L, U, I, O, R, St> {
    pub sessions: S,
    pub transactions: T,
    pub characters: C,
    pub agent: A,
    pub input_language_port: L,
    pub user_context: U,
    pub instance_identity: I,
    pub runtime_owner: O,
    pub realtime_join: R,
    pub bot_speech_state_store: St,
}

pub struct CallService<S, T, C, A, L, U, I, O, R, St> {
    sessions: S,
    transactions: T,
    characters: C,
    agent: A,
    input_language_port: L,
    user_context: U,
    instance_identity: I,
    runtime_owner: O,
    realtime_join: R,
    bot_speech_state_store: St,
}

impl<S, T, C, A, L, U, I, O, R, St> CallService<S, T, C, A, L, U, I, O, R, St> {
    pub fn new(deps: CallDependencies<S, T, C, A, L, U, I, O, R, St>) -> Self {
        Self {
            sessions: deps.sessions,
            transactions: deps.transactions,
            characters: deps.characters,
            agent: deps.agent,
            input_language_port: deps.input_language_port,
            user_context: deps.user_context,
            instance_identity: deps.instance_identity,
            runtime_owner: deps.runtime_owner,
            realtime_join: deps.realtime_join,
            bot_speech_state_store: deps.bot_speech_state_store,
        }
    }
}

#[async_trait]
pub trait CallUseCases: Send + Sync {
    async fn start_call(&self, command: StartCallCommand) -> AppResult<StartCallResponse>;
    async fn end_call(&self, command: EndCallCommand) -> AppResult<()>;
    async fn list_call_history(&self, user_id: i64) -> AppResult<Vec<CallHistoryItemView>>;
}

#[async_trait]
impl<S, T, C, A, L, U, I, O, R, St> CallUseCases for CallService<S, T, C, A, L, U, I, O, R, St>
where
    S: CallSessionRepository + Send + Sync,
    T: CallTransactionRunner + Send + Sync,
    C: CharacterCallContextReadPort + Send + Sync,
    A: AgentSessionPort + Send + Sync,
    L: CallInputLanguagePort + Send + Sync,
    U: CallUserContextPort + Send + Sync,
    I: InstanceIdentityPort + Send + Sync,
    O: RuntimeOwnerPort + Send + Sync,
    R: RealtimeJoinArtifactsPort + Send + Sync,
    St: BotSpeechStateStorePort + Send + Sync,
{
    async fn start_call(&self, command: StartCallCommand) -> AppResult<StartCallResponse> {
        if command.user_id <= 0 || command.character_id <= 0 {
            return Err(AppError::invalid_input(
                "user_id and character_id are required",
            ));
        }

        let user_context = self.user_context.get_user_context(command.user_id).await?;
        if user_context.needs_profile_completion {
            return Err(AppError::invalid_input("user profile is incomplete"));
        }
        let character = self
            .characters
            .get_visible_call_context(command.user_id, command.character_id, command.scene_id)
            .await?;
        let asr_language = self.input_language_port.get_input_language().await?;
        let instance = self.instance_identity.current_instance();
        let runtime_owner_id = self.runtime_owner.current_runtime_owner();
        let agent_session = self
            .agent
            .create_session(
                CallCallerIdentity::PlatformUser {
                    user_id: command.user_id,
                },
                character.character_id,
                Some(character.scene_id),
                &user_context.timezone,
            )
            .await?;
        let started_at = Utc::now();

        let caller = CallCallerIdentity::PlatformUser {
            user_id: command.user_id,
        };
        let pending = CallSession {
            id: 0,
            realtime_participant_identity: caller.realtime_participant_identity(),
            caller,
            character_id: character.character_id,
            character_name: character.character_name,
            scene_id: Some(character.scene_id),
            scene_name: Some(character.scene_name),
            voice: character.voice,
            agent_session_id: agent_session.session_id.clone(),
            asr_language,
            call_type: CallType::Realtime,
            status: CallStatus::Starting,
            owner_instance: instance.id.clone(),
            runtime_owner_id: Some(runtime_owner_id),
            runtime_status: RuntimeWorkStatus::PendingStart,
            runtime_failure_reason: None,
            started_at,
            ended_at: None,
            duration_seconds: 0,
        };
        let session = self
            .write_starting_session(
                &pending,
                lifecycle_event(
                    0,
                    "call_start_requested",
                    started_at,
                    serde_json::json!({
                        "owner_instance": instance.id.clone(),
                    }),
                ),
            )
            .await?;
        let session_runtime_owner_id = session.runtime_owner_id.clone().ok_or_else(|| {
            AppError::internal("starting call session is missing runtime_owner_id")
        })?;
        if let Err(error) = self
            .initialize_bot_speech_state(session.id, &session_runtime_owner_id)
            .await
        {
            self.write_failed_session(
                session.id,
                Utc::now(),
                Some("failed to initialize bot speech state".to_owned()),
                lifecycle_event(
                    session.id,
                    "call_start_failed",
                    Utc::now(),
                    serde_json::json!({
                        "owner_instance": session.owner_instance.clone(),
                        "reason": "failed to initialize bot speech state",
                    }),
                ),
            )
            .await?;
            return Err(error);
        }
        let realtime = match self
            .realtime_join
            .issue_user_join_artifacts(session.id, &session.realtime_participant_identity)
            .await
        {
            Ok(realtime) => realtime,
            Err(error) => {
                self.write_failed_session(
                    session.id,
                    Utc::now(),
                    Some("failed to issue realtime join artifacts".to_owned()),
                    lifecycle_event(
                        session.id,
                        "call_start_failed",
                        Utc::now(),
                        serde_json::json!({
                            "owner_instance": session.owner_instance.clone(),
                            "reason": "failed to issue realtime join artifacts",
                        }),
                    ),
                )
                .await?;
                return Err(error);
            }
        };

        Ok(StartCallResponse {
            session_id: session.id,
            owner_instance: session.owner_instance,
            realtime,
        })
    }

    async fn end_call(&self, command: EndCallCommand) -> AppResult<()> {
        if command.user_id <= 0 || command.session_id <= 0 {
            return Err(AppError::invalid_input(
                "user_id and session_id are required",
            ));
        }

        let session = self
            .sessions
            .get_by_id(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if session.caller.platform_user_id() != Some(command.user_id) {
            return Err(AppError::forbidden("forbidden"));
        }
        if matches!(session.status, CallStatus::Ended) {
            return Ok(());
        }

        let now = Utc::now();
        let session = self
            .write_ending_session(
                session.id,
                lifecycle_event(
                    session.id,
                    "call_end_requested",
                    now,
                    serde_json::json!({
                        "owner_instance": session.owner_instance.clone(),
                    }),
                ),
            )
            .await?;

        self.write_stop_requested_session(
            session.id,
            lifecycle_event(
                session.id,
                "runtime_stop_requested",
                now,
                serde_json::json!({
                    "owner_instance": session.owner_instance.clone(),
                }),
            ),
        )
        .await?;

        Ok(())
    }

    async fn list_call_history(&self, user_id: i64) -> AppResult<Vec<CallHistoryItemView>> {
        if user_id <= 0 {
            return Err(AppError::invalid_input("user_id is required"));
        }

        let sessions = self.sessions.list_for_user(user_id).await?;
        Ok(sessions
            .into_iter()
            .map(|session| CallHistoryItemView {
                session_id: session.id,
                character_id: session.character_id,
                status: session.status.as_str().to_owned(),
                owner_instance: session.owner_instance,
                started_at: session.started_at,
                ended_at: session.ended_at,
                duration_seconds: session.duration_seconds,
            })
            .collect())
    }
}

impl<S, T, C, A, L, U, I, O, R, St> CallService<S, T, C, A, L, U, I, O, R, St>
where
    S: CallSessionRepository + Send + Sync,
    T: CallTransactionRunner + Send + Sync,
    St: BotSpeechStateStorePort + Send + Sync,
{
}

#[async_trait]
#[async_trait]
impl<S, T, C, A, L, U, I, O, R, St> CallExecutionControlUseCases
    for CallService<S, T, C, A, L, U, I, O, R, St>
where
    S: CallSessionRepository + Send + Sync,
    T: CallTransactionRunner + Send + Sync,
    C: CharacterCallContextReadPort + Send + Sync,
    A: AgentSessionPort + Send + Sync,
    L: CallInputLanguagePort + Send + Sync,
    U: CallUserContextPort + Send + Sync,
    I: InstanceIdentityPort + Send + Sync,
    O: RuntimeOwnerPort + Send + Sync,
    R: RealtimeJoinArtifactsPort + Send + Sync,
    St: BotSpeechStateStorePort + Send + Sync,
{
    async fn claim_runtime_start_work(
        &self,
        command: ClaimRuntimeStartCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        if let Some(session) = self
            .sessions
            .claim_runtime_start_work(&command.runtime_owner_id)
            .await?
        {
            let runtime_owner_id = session.runtime_owner_id.clone().ok_or_else(|| {
                AppError::internal("claimed runtime start session is missing runtime_owner_id")
            })?;
            return Ok(Some(RuntimeWorkDescriptor {
                session_id: session.id,
                remote_participant_identity: session.realtime_participant_identity.clone(),
                voice: session.voice.clone(),
                runtime_owner_id,
                kind: RuntimeWorkDescriptorKind::Start,
            }));
        }

        Ok(None)
    }

    async fn claim_runtime_stop_work(
        &self,
        command: ClaimRuntimeStopCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        if let Some(session) = self
            .sessions
            .claim_runtime_stop_work(&command.runtime_owner_id)
            .await?
        {
            let runtime_owner_id = session.runtime_owner_id.clone().ok_or_else(|| {
                AppError::internal("claimed runtime stop session is missing runtime_owner_id")
            })?;
            return Ok(Some(RuntimeWorkDescriptor {
                session_id: session.id,
                remote_participant_identity: session.realtime_participant_identity.clone(),
                voice: session.voice.clone(),
                runtime_owner_id,
                kind: RuntimeWorkDescriptorKind::Stop,
            }));
        }

        Ok(None)
    }

    async fn mark_runtime_ready(&self, command: MarkRuntimeReadyCommand) -> AppResult<()> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        let session = self
            .sessions
            .get_by_id(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if session.runtime_owner_id.as_deref() != Some(command.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }

        if matches!(
            (session.status, session.runtime_status),
            (CallStatus::Active, RuntimeWorkStatus::Ready)
        ) {
            return Ok(());
        }

        if !matches!(
            (session.status, session.runtime_status),
            (CallStatus::Starting, RuntimeWorkStatus::StartClaimed)
        ) {
            return Err(AppError::invalid_input(
                "runtime ready is only valid for claimed starting sessions",
            ));
        }

        self.write_active_runtime_ready_session(
            session.id,
            lifecycle_event(
                session.id,
                "call_started",
                Utc::now(),
                serde_json::json!({
                    "owner_instance": session.owner_instance,
                    "runtime_owner_id": command.runtime_owner_id,
                }),
            ),
        )
        .await?;

        Ok(())
    }

    async fn mark_runtime_failed(&self, command: MarkRuntimeFailedCommand) -> AppResult<()> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        let session = self
            .sessions
            .get_by_id(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if session.runtime_owner_id.as_deref() != Some(command.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }
        let failed_at = Utc::now();
        if matches!(
            (session.status, session.runtime_status),
            (CallStatus::Failed, RuntimeWorkStatus::Failed)
        ) {
            return Ok(());
        }
        if !matches!(
            session.status,
            CallStatus::Starting | CallStatus::Active | CallStatus::Ending
        ) {
            return Err(AppError::invalid_input(
                "runtime failed is not valid for the current call status",
            ));
        }
        self.write_failed_session(
            session.id,
            failed_at,
            command.failure_reason.clone(),
            lifecycle_event(
                session.id,
                if matches!(session.status, CallStatus::Starting) {
                    "call_start_failed"
                } else {
                    "runtime_failed"
                },
                failed_at,
                serde_json::json!({
                    "owner_instance": session.owner_instance,
                    "reason": command.failure_reason,
                }),
            ),
        )
        .await?;

        Ok(())
    }

    async fn mark_runtime_stopped(&self, command: MarkRuntimeStoppedCommand) -> AppResult<()> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        let session = self
            .sessions
            .get_by_id(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if session.runtime_owner_id.as_deref() != Some(command.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }
        if matches!(
            (session.status, session.runtime_status),
            (CallStatus::Ended, RuntimeWorkStatus::Stopped)
        ) {
            return Ok(());
        }
        if !matches!(
            (session.status, session.runtime_status),
            (CallStatus::Ending, RuntimeWorkStatus::StopClaimed)
        ) {
            return Err(AppError::invalid_input(
                "runtime stopped is only valid for claimed stopping sessions",
            ));
        }
        let ended_at = Utc::now();
        let duration_seconds = ended_at
            .signed_duration_since(session.started_at)
            .num_seconds()
            .max(0) as i32;
        self.write_ended_session(
            session.id,
            ended_at,
            duration_seconds,
            lifecycle_event(
                session.id,
                "call_ended",
                ended_at,
                serde_json::json!({
                    "owner_instance": session.owner_instance,
                    "duration_seconds": duration_seconds,
                }),
            ),
        )
        .await?;

        Ok(())
    }

    async fn mark_runtime_missing(&self, command: MarkRuntimeMissingCommand) -> AppResult<()> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        let session = self
            .sessions
            .get_by_id(command.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if session.runtime_owner_id.as_deref() != Some(command.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }

        match (session.status, session.runtime_status) {
            (CallStatus::Ended, RuntimeWorkStatus::Stopped) => Ok(()),
            (CallStatus::Ending, RuntimeWorkStatus::StopClaimed) => {
                self.mark_runtime_stopped(MarkRuntimeStoppedCommand {
                    session_id: command.session_id,
                    runtime_owner_id: command.runtime_owner_id,
                })
                .await
            }
            (CallStatus::Starting | CallStatus::Active, _) => {
                self.mark_runtime_failed(MarkRuntimeFailedCommand {
                    session_id: command.session_id,
                    runtime_owner_id: command.runtime_owner_id,
                    failure_reason: Some("worker reported missing local runtime".to_owned()),
                })
                .await
            }
            _ => Err(AppError::invalid_input(
                "runtime missing is not valid for the current call status",
            )),
        }
    }
}

impl<S, T, C, A, L, U, I, O, R, St> CallService<S, T, C, A, L, U, I, O, R, St>
where
    T: CallTransactionRunner + Send + Sync,
    St: BotSpeechStateStorePort + Send + Sync,
{
    async fn initialize_bot_speech_state(
        &self,
        session_id: i64,
        _runtime_owner_id: &str,
    ) -> AppResult<()> {
        let state = BotSpeechState::initialized_with(Vec::new());
        self.bot_speech_state_store
            .save_state(session_id, &state)
            .await
    }

    async fn write_starting_session(
        &self,
        session: &CallSession,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        let session = session.clone();
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let created = ctx.store.create(&session).await?;
                    let mut event = event;
                    event.session_id = created.id;
                    ctx.store.insert(event).await?;
                    Ok(created)
                })
            })
            .await
    }

    async fn write_active_runtime_ready_session(
        &self,
        session_id: i64,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let session = ctx
                        .store
                        .mark_status_and_runtime(
                            session_id,
                            CallStatus::Active,
                            RuntimeWorkStatus::Ready,
                            None,
                        )
                        .await?;
                    ctx.store.insert(event).await?;
                    Ok(session)
                })
            })
            .await
    }

    async fn write_ending_session(
        &self,
        session_id: i64,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let session = ctx
                        .store
                        .mark_status(session_id, CallStatus::Ending)
                        .await?;
                    ctx.store.insert(event).await?;
                    Ok(session)
                })
            })
            .await
    }

    async fn write_failed_session(
        &self,
        session_id: i64,
        failed_at: chrono::DateTime<Utc>,
        runtime_failure_reason: Option<String>,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let session = ctx
                        .store
                        .mark_ended(
                            session_id,
                            CallStatus::Failed,
                            RuntimeWorkStatus::Failed,
                            runtime_failure_reason,
                            failed_at,
                            0,
                        )
                        .await?;
                    ctx.store.insert(event).await?;
                    Ok(session)
                })
            })
            .await
    }

    async fn write_ended_session(
        &self,
        session_id: i64,
        ended_at: chrono::DateTime<Utc>,
        duration_seconds: i32,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let session = ctx
                        .store
                        .mark_ended(
                            session_id,
                            CallStatus::Ended,
                            RuntimeWorkStatus::Stopped,
                            None,
                            ended_at,
                            duration_seconds,
                        )
                        .await?;
                    ctx.store.insert(event).await?;
                    Ok(session)
                })
            })
            .await
    }

    async fn write_stop_requested_session(
        &self,
        session_id: i64,
        event: CallEvent,
    ) -> AppResult<CallSession> {
        self.transactions
            .run(|ctx| {
                Box::pin(async move {
                    let session = ctx
                        .store
                        .mark_status_and_runtime(
                            session_id,
                            CallStatus::Ending,
                            RuntimeWorkStatus::StopRequested,
                            None,
                        )
                        .await?;
                    ctx.store.insert(event).await?;
                    Ok(session)
                })
            })
            .await
    }
}

fn lifecycle_event(
    session_id: i64,
    event: &str,
    at: chrono::DateTime<Utc>,
    fields: serde_json::Value,
) -> CallEvent {
    CallEvent {
        session_id,
        round_id: None,
        source: "call_core".to_owned(),
        event: event.to_owned(),
        ts_ms: at.timestamp_millis(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{CallTransactionContext, CallTransactionStore};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard};

    #[derive(Default)]
    struct StubSessions {
        saved: Mutex<Vec<CallSession>>,
        events: Mutex<Vec<CallEvent>>,
    }

    #[derive(Clone, Default)]
    struct StubTransactionRunner {
        sessions: Arc<StubSessions>,
        fail_on_begin: bool,
        fail_on_commit: bool,
        fail_on_event_insert: bool,
    }

    struct StubSessionWriter<'a> {
        sessions: &'a Arc<StubSessions>,
        fail_on_event_insert: bool,
    }

    struct StubCharacters;
    struct StubAgent;
    struct StubInputLanguage;
    struct StubUserContext;
    struct StubIdentity;
    #[derive(Clone, Default)]
    struct StubBotSpeechStateStore {
        states: Arc<Mutex<HashMap<i64, BotSpeechState>>>,
    }

    type TestCallService = CallService<
        Arc<StubSessions>,
        StubTransactionRunner,
        StubCharacters,
        StubAgent,
        StubInputLanguage,
        StubUserContext,
        StubIdentity,
        StubRuntimeOwner,
        StubRealtimeJoin,
        StubBotSpeechStateStore,
    >;

    fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[async_trait]
    impl CallSessionRepository for Arc<StubSessions> {
        async fn create(&self, session: &CallSession) -> AppResult<CallSession> {
            let mut saved = lock_or_recover(&self.saved);
            let mut cloned = session.clone();
            cloned.id = 42;
            saved.push(cloned.clone());
            Ok(cloned)
        }

        async fn get_by_id(&self, session_id: i64) -> AppResult<Option<CallSession>> {
            Ok(lock_or_recover(&self.saved)
                .iter()
                .find(|item| item.id == session_id)
                .cloned())
        }

        async fn list_for_user(&self, user_id: i64) -> AppResult<Vec<CallSession>> {
            Ok(lock_or_recover(&self.saved)
                .iter()
                .filter(|item| item.caller.platform_user_id() == Some(user_id))
                .cloned()
                .collect())
        }

        async fn claim_runtime_start_work(
            &self,
            runtime_owner_id: &str,
        ) -> AppResult<Option<CallSession>> {
            let mut saved = lock_or_recover(&self.saved);
            let candidate = saved
                .iter_mut()
                .filter(|item| {
                    item.runtime_owner_id.as_deref() == Some(runtime_owner_id)
                        && item.status == CallStatus::Starting
                        && item.runtime_status == RuntimeWorkStatus::PendingStart
                })
                .min_by_key(|item| item.started_at);
            if let Some(session) = candidate {
                session.runtime_status = RuntimeWorkStatus::StartClaimed;
                session.runtime_failure_reason = None;
                return Ok(Some(session.clone()));
            }
            Ok(None)
        }

        async fn claim_runtime_stop_work(
            &self,
            runtime_owner_id: &str,
        ) -> AppResult<Option<CallSession>> {
            let mut saved = lock_or_recover(&self.saved);
            let candidate = saved
                .iter_mut()
                .filter(|item| {
                    item.runtime_owner_id.as_deref() == Some(runtime_owner_id)
                        && item.status == CallStatus::Ending
                        && item.runtime_status == RuntimeWorkStatus::StopRequested
                })
                .min_by_key(|item| item.started_at);
            if let Some(session) = candidate {
                session.runtime_status = RuntimeWorkStatus::StopClaimed;
                session.runtime_failure_reason = None;
                return Ok(Some(session.clone()));
            }
            Ok(None)
        }

        async fn mark_status(&self, session_id: i64, status: CallStatus) -> AppResult<CallSession> {
            let mut saved = lock_or_recover(&self.saved);
            let session = saved
                .iter_mut()
                .find(|item| item.id == session_id)
                .ok_or_else(|| AppError::not_found("call session not found"))?;
            session.status = status;
            Ok(session.clone())
        }

        async fn mark_runtime_state(
            &self,
            session_id: i64,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
        ) -> AppResult<CallSession> {
            let mut saved = lock_or_recover(&self.saved);
            let session = saved
                .iter_mut()
                .find(|item| item.id == session_id)
                .ok_or_else(|| AppError::not_found("call session not found"))?;
            session.runtime_status = runtime_status;
            session.runtime_failure_reason = runtime_failure_reason;
            Ok(session.clone())
        }

        async fn mark_status_and_runtime(
            &self,
            session_id: i64,
            status: CallStatus,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
        ) -> AppResult<CallSession> {
            let mut saved = lock_or_recover(&self.saved);
            let session = saved
                .iter_mut()
                .find(|item| item.id == session_id)
                .ok_or_else(|| AppError::not_found("call session not found"))?;
            session.status = status;
            session.runtime_status = runtime_status;
            session.runtime_failure_reason = runtime_failure_reason;
            Ok(session.clone())
        }

        async fn mark_ended(
            &self,
            session_id: i64,
            status: CallStatus,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
            ended_at: chrono::DateTime<Utc>,
            duration_seconds: i32,
        ) -> AppResult<CallSession> {
            let mut saved = lock_or_recover(&self.saved);
            let session = saved
                .iter_mut()
                .find(|item| item.id == session_id)
                .ok_or_else(|| AppError::not_found("call session not found"))?;
            session.status = status;
            session.runtime_status = runtime_status;
            session.runtime_failure_reason = runtime_failure_reason;
            session.ended_at = Some(ended_at);
            session.duration_seconds = duration_seconds;
            Ok(session.clone())
        }
    }

    #[async_trait]
    impl CharacterCallContextReadPort for StubCharacters {
        async fn get_visible_call_context(
            &self,
            _user_id: i64,
            character_id: i64,
            selected_scene_id: Option<i64>,
        ) -> AppResult<character_context::CharacterCallContext> {
            Ok(character_context::CharacterCallContext {
                character_id,
                character_name: "角色".to_owned(),
                scene_id: selected_scene_id.unwrap_or(7),
                scene_name: "Scene".to_owned(),
                voice: "voice-9".to_owned(),
            })
        }
    }

    #[async_trait]
    impl AgentSessionPort for StubAgent {
        async fn create_session(
            &self,
            _caller: CallCallerIdentity,
            _character_id: i64,
            _scene_id: Option<i64>,
            _timezone: &str,
        ) -> AppResult<crate::ports::AgentSessionHandle> {
            Ok(crate::ports::AgentSessionHandle {
                session_id: "agent-session-1".to_owned(),
            })
        }
    }

    #[async_trait]
    impl CallInputLanguagePort for StubInputLanguage {
        async fn get_input_language(&self) -> AppResult<crate::domain::SpeechInputLanguage> {
            Ok(crate::domain::SpeechInputLanguage::Zh)
        }
    }

    #[async_trait]
    impl CallUserContextPort for StubUserContext {
        async fn get_user_context(&self, user_id: i64) -> AppResult<crate::ports::CallUserContext> {
            Ok(crate::ports::CallUserContext {
                user_id,
                timezone: "Asia/Shanghai".to_owned(),
                needs_profile_completion: false,
            })
        }
    }

    impl InstanceIdentityPort for StubIdentity {
        fn current_instance(&self) -> InstanceIdentity {
            InstanceIdentity {
                id: "instance-a".to_owned(),
            }
        }
    }

    struct StubRuntimeOwner;
    struct StubRealtimeJoin;

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

    impl RuntimeOwnerPort for StubRuntimeOwner {
        fn current_runtime_owner(&self) -> String {
            "worker-a".to_owned()
        }
    }

    #[async_trait]
    impl RealtimeJoinArtifactsPort for StubRealtimeJoin {
        async fn issue_user_join_artifacts(
            &self,
            session_id: i64,
            participant_identity: &str,
        ) -> AppResult<rtc_control_contract::RealtimeJoinArtifacts> {
            Ok(rtc_control_contract::RealtimeJoinArtifacts {
                provider: "livekit".to_owned(),
                endpoint: "ws://127.0.0.1:7880".to_owned(),
                room_name: format!("call-{session_id}"),
                access_token: format!("token-{session_id}"),
                participant_identity: participant_identity.to_owned(),
                bot_participant_identity: format!("bot-{session_id}"),
            })
        }
    }

    #[async_trait]
    impl CallTransactionStore for StubSessionWriter<'_> {
        async fn create(&mut self, session: &CallSession) -> AppResult<CallSession> {
            self.sessions.create(session).await
        }

        async fn mark_status(
            &mut self,
            session_id: i64,
            status: CallStatus,
        ) -> AppResult<CallSession> {
            self.sessions.mark_status(session_id, status).await
        }

        async fn mark_runtime_state(
            &mut self,
            session_id: i64,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
        ) -> AppResult<CallSession> {
            self.sessions
                .mark_runtime_state(session_id, runtime_status, runtime_failure_reason)
                .await
        }

        async fn mark_status_and_runtime(
            &mut self,
            session_id: i64,
            status: CallStatus,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
        ) -> AppResult<CallSession> {
            self.sessions
                .mark_status_and_runtime(session_id, status, runtime_status, runtime_failure_reason)
                .await
        }

        async fn mark_ended(
            &mut self,
            session_id: i64,
            status: CallStatus,
            runtime_status: RuntimeWorkStatus,
            runtime_failure_reason: Option<String>,
            ended_at: chrono::DateTime<Utc>,
            duration_seconds: i32,
        ) -> AppResult<CallSession> {
            self.sessions
                .mark_ended(
                    session_id,
                    status,
                    runtime_status,
                    runtime_failure_reason,
                    ended_at,
                    duration_seconds,
                )
                .await
        }

        async fn insert(&mut self, event: CallEvent) -> AppResult<()> {
            if self.fail_on_event_insert {
                return Err(AppError::internal("event insert failed"));
            }
            lock_or_recover(&self.sessions.events).push(event);
            Ok(())
        }
    }

    impl CallTransactionRunner for StubTransactionRunner {
        fn run<'a, T, F>(&'a self, op: F) -> crate::ports::CallTransactionFuture<'a, T>
        where
            T: Send + 'a,
            F: for<'tx> FnOnce(
                    CallTransactionContext<'tx>,
                ) -> crate::ports::CallTransactionFuture<'tx, T>
                + Send
                + 'a,
        {
            Box::pin(async move {
                if self.fail_on_begin {
                    return Err(AppError::internal("transaction begin failed"));
                }
                let mut sessions = StubSessionWriter {
                    sessions: &self.sessions,
                    fail_on_event_insert: self.fail_on_event_insert,
                };
                let result = op(CallTransactionContext {
                    store: &mut sessions,
                })
                .await?;
                if self.fail_on_commit {
                    return Err(AppError::internal("transaction commit failed"));
                }
                Ok(result)
            })
        }
    }

    fn build_service(
        sessions: Arc<StubSessions>,
        tx_runner: StubTransactionRunner,
    ) -> TestCallService {
        CallService::new(CallDependencies {
            sessions,
            transactions: tx_runner,
            characters: StubCharacters,
            agent: StubAgent,
            input_language_port: StubInputLanguage,
            user_context: StubUserContext,
            instance_identity: StubIdentity,
            runtime_owner: StubRuntimeOwner,
            realtime_join: StubRealtimeJoin,
            bot_speech_state_store: StubBotSpeechStateStore::default(),
        })
    }

    #[tokio::test]
    async fn start_call_creates_starting_session_with_pending_runtime_work() {
        let sessions = Arc::new(StubSessions::default());
        let service = build_service(
            sessions.clone(),
            StubTransactionRunner {
                sessions: sessions.clone(),
                ..Default::default()
            },
        );

        let response_result = service
            .start_call(StartCallCommand {
                user_id: 1,
                character_id: 2,
                scene_id: None,
            })
            .await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };

        assert_eq!(response.owner_instance, "instance-a");
        assert_eq!(response.realtime.provider, "livekit");
        assert_eq!(response.realtime.room_name, "call-42");

        let history_result = service.list_call_history(1).await;
        assert!(history_result.is_ok());
        let history = match history_result {
            Ok(history) => history,
            Err(_) => return,
        };
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "starting");

        {
            let saved = lock_or_recover(&sessions.saved);
            let session = match saved.first() {
                Some(session) => session,
                None => return,
            };
            assert_eq!(session.runtime_owner_id.as_deref(), Some("worker-a"));
            assert_eq!(session.runtime_status, RuntimeWorkStatus::PendingStart);
        }

        let state_result = service
            .bot_speech_state_store
            .load_state(response.session_id)
            .await;
        assert!(state_result.is_ok());
        let state = match state_result {
            Ok(Some(state)) => state,
            _ => return,
        };
        assert!(state.initialized);
        assert!(state.active_item.is_none());
        assert!(state.pending_items.is_empty());
    }

    #[tokio::test]
    async fn poll_runtime_start_work_claims_pending_start() {
        let sessions = Arc::new(StubSessions::default());
        let service = build_service(
            sessions.clone(),
            StubTransactionRunner {
                sessions: sessions.clone(),
                ..Default::default()
            },
        );

        let started_result = service
            .start_call(StartCallCommand {
                user_id: 1,
                character_id: 2,
                scene_id: None,
            })
            .await;
        assert!(started_result.is_ok());
        let started = match started_result {
            Ok(started) => started,
            Err(_) => return,
        };

        let claim_result = service
            .claim_runtime_start_work(ClaimRuntimeStartCommand {
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(claim_result.is_ok());
        let work = match claim_result {
            Ok(Some(work)) => work,
            _ => return,
        };

        assert_eq!(work.session_id, started.session_id);
        assert_eq!(work.kind, RuntimeWorkDescriptorKind::Start);

        let saved = lock_or_recover(&sessions.saved);
        let session = match saved.first() {
            Some(session) => session,
            None => return,
        };
        assert_eq!(session.runtime_status, RuntimeWorkStatus::StartClaimed);
    }

    #[tokio::test]
    async fn report_runtime_ready_moves_call_to_active() {
        let sessions = Arc::new(StubSessions::default());
        let service = build_service(
            sessions.clone(),
            StubTransactionRunner {
                sessions: sessions.clone(),
                ..Default::default()
            },
        );

        let started_result = service
            .start_call(StartCallCommand {
                user_id: 1,
                character_id: 2,
                scene_id: None,
            })
            .await;
        assert!(started_result.is_ok());
        let started = match started_result {
            Ok(started) => started,
            Err(_) => return,
        };
        let poll_result = service
            .claim_runtime_start_work(ClaimRuntimeStartCommand {
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(poll_result.is_ok());

        let report_result = service
            .mark_runtime_ready(MarkRuntimeReadyCommand {
                session_id: started.session_id,
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(report_result.is_ok());

        {
            let saved = lock_or_recover(&sessions.saved);
            let session = match saved.first() {
                Some(session) => session,
                None => return,
            };
            assert_eq!(session.status, CallStatus::Active);
            assert_eq!(session.runtime_status, RuntimeWorkStatus::Ready);
        }

        let state_result = service
            .bot_speech_state_store
            .load_state(started.session_id)
            .await;
        assert!(state_result.is_ok());
        let state = match state_result {
            Ok(Some(state)) => state,
            _ => return,
        };
        assert!(state.pending_items.is_empty());
    }

    #[tokio::test]
    async fn end_call_marks_stop_requested_until_worker_reports_stopped() {
        let sessions = Arc::new(StubSessions::default());
        let service = build_service(
            sessions.clone(),
            StubTransactionRunner {
                sessions: sessions.clone(),
                ..Default::default()
            },
        );

        let started_result = service
            .start_call(StartCallCommand {
                user_id: 1,
                character_id: 2,
                scene_id: None,
            })
            .await;
        assert!(started_result.is_ok());
        let started = match started_result {
            Ok(started) => started,
            Err(_) => return,
        };
        let poll_result = service
            .claim_runtime_start_work(ClaimRuntimeStartCommand {
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(poll_result.is_ok());
        let report_ready_result = service
            .mark_runtime_ready(MarkRuntimeReadyCommand {
                session_id: started.session_id,
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(report_ready_result.is_ok());

        let end_result = service
            .end_call(EndCallCommand {
                user_id: 1,
                session_id: started.session_id,
            })
            .await;
        assert!(end_result.is_ok());

        {
            let saved = lock_or_recover(&sessions.saved);
            let session = match saved.first() {
                Some(session) => session,
                None => return,
            };
            assert_eq!(session.status, CallStatus::Ending);
            assert_eq!(session.runtime_status, RuntimeWorkStatus::StopRequested);
        }

        let stop_work_result = service
            .claim_runtime_stop_work(ClaimRuntimeStopCommand {
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(stop_work_result.is_ok());
        let stop_work = match stop_work_result {
            Ok(Some(work)) => work,
            _ => return,
        };
        assert_eq!(stop_work.kind, RuntimeWorkDescriptorKind::Stop);

        let report_stopped_result = service
            .mark_runtime_stopped(MarkRuntimeStoppedCommand {
                session_id: started.session_id,
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;
        assert!(report_stopped_result.is_ok());

        let history_result = service.list_call_history(1).await;
        assert!(history_result.is_ok());
        let history = match history_result {
            Ok(history) => history,
            Err(_) => return,
        };
        assert_eq!(history[0].status, "ended");
    }

    #[tokio::test]
    async fn report_runtime_stopped_rejects_invalid_transition() {
        let sessions = Arc::new(StubSessions::default());
        let service = build_service(
            sessions.clone(),
            StubTransactionRunner {
                sessions: sessions.clone(),
                ..Default::default()
            },
        );

        let started_result = service
            .start_call(StartCallCommand {
                user_id: 1,
                character_id: 2,
                scene_id: None,
            })
            .await;
        assert!(started_result.is_ok());
        let started = match started_result {
            Ok(started) => started,
            Err(_) => return,
        };

        let result = service
            .mark_runtime_stopped(MarkRuntimeStoppedCommand {
                session_id: started.session_id,
                runtime_owner_id: "worker-a".to_owned(),
            })
            .await;

        assert!(result.is_err());
    }
}
