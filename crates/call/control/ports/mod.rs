use crate::domain::BotSpeechState;
use async_trait::async_trait;
use call_control_bot_speech::BotSpeechItem;
use call_control_orchestration::ServerInitiatedTurnTrigger;
pub use call_log_contract::CallEventSinkPort as EventSinkPort;
pub use call_speech_contract::BotSpeechRuntimeTriggerPort;
use std::{future::Future, pin::Pin, sync::Arc};

use shared_kernel::AppResult;

use crate::application::InstanceIdentity;
use crate::domain::{
    CallCallerIdentity, CallEvent, CallSession, CallStatus, CallTimelineEvent, RuntimeWorkStatus,
    SpeechInputLanguage,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallUserContext {
    pub user_id: i64,
    pub timezone: String,
    pub needs_profile_completion: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionHandle {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeWorkDescriptorKind {
    Start,
    Stop,
}

impl RuntimeWorkDescriptorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWorkDescriptor {
    pub session_id: i64,
    pub remote_participant_identity: String,
    pub voice: String,
    pub runtime_owner_id: String,
    pub kind: RuntimeWorkDescriptorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRuntimeStartCommand {
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRuntimeStopCommand {
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRuntimeReadyCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRuntimeFailedCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRuntimeStoppedCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRuntimeMissingCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionState {
    pub session_id: i64,
    pub agent_session_id: String,
    pub voice: String,
    pub asr_language: SpeechInputLanguage,
    pub runtime_owner_id: Option<String>,
    pub status: CallStatus,
    pub runtime_status: RuntimeWorkStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildServerInitiatedTurnCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub trigger: ServerInitiatedTurnTrigger,
}

#[async_trait]
pub trait CallSessionRepository: Send + Sync {
    async fn create(&self, session: &CallSession) -> AppResult<CallSession>;
    async fn get_by_id(&self, session_id: i64) -> AppResult<Option<CallSession>>;
    async fn list_for_user(&self, user_id: i64) -> AppResult<Vec<CallSession>>;
    async fn claim_runtime_start_work(
        &self,
        runtime_owner_id: &str,
    ) -> AppResult<Option<CallSession>>;
    async fn claim_runtime_stop_work(
        &self,
        runtime_owner_id: &str,
    ) -> AppResult<Option<CallSession>>;
    async fn mark_status(
        &self,
        session_id: i64,
        status: crate::domain::CallStatus,
    ) -> AppResult<CallSession>;
    async fn mark_runtime_state(
        &self,
        session_id: i64,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession>;
    async fn mark_status_and_runtime(
        &self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession>;
    async fn mark_ended(
        &self,
        session_id: i64,
        status: crate::domain::CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
        ended_at: chrono::DateTime<chrono::Utc>,
        duration_seconds: i32,
    ) -> AppResult<CallSession>;
}

#[async_trait]
pub trait CallTransactionStore: Send {
    async fn create(&mut self, session: &CallSession) -> AppResult<CallSession>;

    async fn mark_status(&mut self, session_id: i64, status: CallStatus) -> AppResult<CallSession>;
    async fn mark_runtime_state(
        &mut self,
        session_id: i64,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession>;
    async fn mark_status_and_runtime(
        &mut self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession>;

    async fn mark_ended(
        &mut self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
        ended_at: chrono::DateTime<chrono::Utc>,
        duration_seconds: i32,
    ) -> AppResult<CallSession>;
    async fn insert(&mut self, event: CallEvent) -> AppResult<()>;
}

pub struct CallTransactionContext<'a> {
    pub store: &'a mut dyn CallTransactionStore,
}

pub type CallTransactionFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

pub trait CallTransactionRunner: Send + Sync {
    fn run<'a, T, F>(&'a self, op: F) -> CallTransactionFuture<'a, T>
    where
        T: Send + 'a,
        F: for<'tx> FnOnce(CallTransactionContext<'tx>) -> CallTransactionFuture<'tx, T>
            + Send
            + 'a;
}

#[async_trait]
pub trait AgentSessionPort: Send + Sync {
    async fn create_session(
        &self,
        caller: CallCallerIdentity,
        character_id: i64,
        scene_id: Option<i64>,
        timezone: &str,
    ) -> AppResult<AgentSessionHandle>;
}

#[async_trait]
pub trait CallInputLanguagePort: Send + Sync {
    async fn get_input_language(&self) -> AppResult<SpeechInputLanguage>;
}

#[async_trait]
pub trait CallUserContextPort: Send + Sync {
    async fn get_user_context(&self, user_id: i64) -> AppResult<CallUserContext>;
}

#[async_trait]
pub trait CallEventLogRepository: Send + Sync {
    async fn append_batch(&self, events: &[CallEvent]) -> AppResult<()>;
    async fn list_timeline(&self, session_id: i64) -> AppResult<Vec<CallTimelineEvent>>;
    async fn list_prompt_logs(&self, session_id: i64) -> AppResult<Vec<CallPromptLogFact>>;
    /// 通话的原始对话(来自 llm_messages,经 call_sessions.agent_session_id 关联),按轮次有序。
    async fn list_call_conversation(
        &self,
        session_id: i64,
    ) -> AppResult<Vec<CallConversationMessageFact>>;
    async fn get_started_at_ms(&self, session_id: i64) -> AppResult<Option<i64>>;
    async fn list_call_log_facts(
        &self,
        query: &CallLogFactsQuery,
    ) -> AppResult<(Vec<CallLogFact>, i64)>;
    async fn get_call_log_fact(&self, session_id: i64) -> AppResult<Option<CallLogFact>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallLogFactsQuery {
    pub character_id: Option<i64>,
    pub page: i64,
    pub page_size: i64,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLogFact {
    pub session_id: i64,
    pub caller: CallCallerIdentity,
    pub character_id: i64,
    pub character_name: String,
    pub scene_id: Option<i64>,
    pub scene_name: Option<String>,
    pub agent_session_id: String,
    pub voice: String,
    pub asr_language: String,
    pub status: String,
    pub runtime_status: String,
    pub runtime_failure_reason: Option<String>,
    pub owner_instance: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: i32,
    pub event_count: i64,
    pub round_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallPromptLogMessageFact {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallPromptLogFact {
    pub id: i64,
    pub session_id: i64,
    pub round_id: Option<String>,
    pub source: String,
    pub model_name: String,
    pub request_messages: Vec<CallPromptLogMessageFact>,
    pub response_text: String,
    pub created_at: DateTime<Utc>,
}

/// 通话原始对话的单条消息(来自 llm_messages)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallConversationMessageFact {
    pub id: i64,
    pub turn_number: i32,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

pub trait InstanceIdentityPort: Send + Sync {
    fn current_instance(&self) -> InstanceIdentity;
}

pub trait RuntimeOwnerPort: Send + Sync {
    fn current_runtime_owner(&self) -> String;
}

#[async_trait]
pub trait RuntimeSessionStatePort: Send + Sync {
    async fn get_runtime_session_state(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionState>>;
}

#[async_trait]
impl<T> RuntimeSessionStatePort for Arc<T>
where
    T: RuntimeSessionStatePort + Send + Sync + ?Sized,
{
    async fn get_runtime_session_state(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionState>> {
        (**self).get_runtime_session_state(session_id).await
    }
}

#[async_trait]
pub trait CallExecutionControlUseCases: Send + Sync {
    async fn claim_runtime_start_work(
        &self,
        command: ClaimRuntimeStartCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>>;
    async fn claim_runtime_stop_work(
        &self,
        command: ClaimRuntimeStopCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>>;
    async fn mark_runtime_ready(&self, command: MarkRuntimeReadyCommand) -> AppResult<()>;
    async fn mark_runtime_failed(&self, command: MarkRuntimeFailedCommand) -> AppResult<()>;
    async fn mark_runtime_stopped(&self, command: MarkRuntimeStoppedCommand) -> AppResult<()>;
    async fn mark_runtime_missing(&self, command: MarkRuntimeMissingCommand) -> AppResult<()>;
}

#[async_trait]
pub trait BotSpeechStateStorePort: Send + Sync {
    async fn load_state(&self, session_id: i64) -> AppResult<Option<BotSpeechState>>;
    async fn save_state(&self, session_id: i64, state: &BotSpeechState) -> AppResult<()>;
}

#[async_trait]
pub trait ServerInitiatedBotSpeechPort: Send + Sync {
    async fn build_server_initiated_turn(
        &self,
        command: BuildServerInitiatedTurnCommand,
    ) -> AppResult<BotSpeechItem>;
}

#[async_trait]
impl<T> CallExecutionControlUseCases for Arc<T>
where
    T: CallExecutionControlUseCases + Send + Sync + ?Sized,
{
    async fn claim_runtime_start_work(
        &self,
        command: ClaimRuntimeStartCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>> {
        (**self).claim_runtime_start_work(command).await
    }

    async fn claim_runtime_stop_work(
        &self,
        command: ClaimRuntimeStopCommand,
    ) -> AppResult<Option<RuntimeWorkDescriptor>> {
        (**self).claim_runtime_stop_work(command).await
    }

    async fn mark_runtime_ready(&self, command: MarkRuntimeReadyCommand) -> AppResult<()> {
        (**self).mark_runtime_ready(command).await
    }

    async fn mark_runtime_failed(&self, command: MarkRuntimeFailedCommand) -> AppResult<()> {
        (**self).mark_runtime_failed(command).await
    }

    async fn mark_runtime_stopped(&self, command: MarkRuntimeStoppedCommand) -> AppResult<()> {
        (**self).mark_runtime_stopped(command).await
    }

    async fn mark_runtime_missing(&self, command: MarkRuntimeMissingCommand) -> AppResult<()> {
        (**self).mark_runtime_missing(command).await
    }
}

#[async_trait]
impl<T> ServerInitiatedBotSpeechPort for Arc<T>
where
    T: ServerInitiatedBotSpeechPort + Send + Sync + ?Sized,
{
    async fn build_server_initiated_turn(
        &self,
        command: BuildServerInitiatedTurnCommand,
    ) -> AppResult<BotSpeechItem> {
        (**self).build_server_initiated_turn(command).await
    }
}

#[async_trait]
impl<T> BotSpeechStateStorePort for Arc<T>
where
    T: BotSpeechStateStorePort + Send + Sync + ?Sized,
{
    async fn load_state(&self, session_id: i64) -> AppResult<Option<BotSpeechState>> {
        (**self).load_state(session_id).await
    }

    async fn save_state(&self, session_id: i64, state: &BotSpeechState) -> AppResult<()> {
        (**self).save_state(session_id, state).await
    }
}
