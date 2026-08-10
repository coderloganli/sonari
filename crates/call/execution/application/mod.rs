use async_trait::async_trait;
use call_runtime_context::{RuntimeSessionContext, RuntimeSessionContextPort};
use call_runtime_control::{
    RuntimeEventFact, RuntimeFactKind, RuntimeLaunchSpec, RuntimeWorkItem, RuntimeWorkKind,
};

use call::{
    CallExecutionControlUseCases, ClaimRuntimeStartCommand, ClaimRuntimeStopCommand,
    MarkRuntimeFailedCommand, MarkRuntimeMissingCommand, MarkRuntimeReadyCommand,
    MarkRuntimeStoppedCommand, RuntimeWorkDescriptor, RuntimeWorkDescriptorKind,
};
use serde_json::json;
use shared_kernel::{AppError, AppResult};
use tracing::Instrument;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStatusKind {
    Ready,
    Failed,
    Stopped,
    Missing,
}

impl RuntimeStatusKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            "stopped" => Some(Self::Stopped),
            "missing" => Some(Self::Missing),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollRuntimeWorkCommand {
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRuntimeStatusCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub status: RuntimeStatusKind,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishRuntimeEventCommand {
    pub fact: RuntimeEventFact,
}

#[async_trait]
pub trait RuntimeLaunchProvisionPort: Send + Sync {
    async fn prepare_runtime_launch(
        &self,
        session_id: i64,
        remote_participant_identity: String,
        voice: String,
        agent_session_id: &str,
        asr_language: &str,
    ) -> AppResult<RuntimeLaunchSpec>;
}

/// 组装进程内编排所需的「每会话非密钥配置」(从 DB 解析 ASR/LLM/TTS selection + 分段 +
/// voiceprint→voice_id)。返回 None 表示走旧的后端编排路径(launch-spec.speech 为 None)。
/// 控制面在 dispatch 时调用,把结果填入 launch-spec 下发给 worker(决策 F:控制/数据面分离)。
#[async_trait]
pub trait SpeechBootstrapComposerPort: Send + Sync {
    async fn compose(
        &self,
        voice: String,
        agent_session_id: &str,
        language: &str,
    ) -> AppResult<Option<call_runtime_control::SpeechRuntimeBootstrap>>;
}

/// 默认组装器:始终返回 None(走旧后端编排路径)。进程内编排未启用时用它。
pub struct NoopSpeechBootstrapComposer;

#[async_trait]
impl SpeechBootstrapComposerPort for NoopSpeechBootstrapComposer {
    async fn compose(
        &self,
        _voice: String,
        _agent_session_id: &str,
        _language: &str,
    ) -> AppResult<Option<call_runtime_control::SpeechRuntimeBootstrap>> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExecutionEvent {
    pub session_id: i64,
    pub round_id: Option<String>,
    pub source: String,
    pub event: String,
    pub ts_ms: i64,
    pub fields: serde_json::Value,
}

#[async_trait]
pub trait CallExecutionEventPort: Send + Sync {
    async fn publish(&self, event: CallExecutionEvent) -> AppResult<()>;
}

#[async_trait]
pub trait RuntimeFactLogPort: Send + Sync {
    async fn append(&self, fact: &RuntimeEventFact) -> AppResult<()>;
}

pub struct CallExecutionDependencies<C, L, R, E, F> {
    pub control: C,
    pub launch: L,
    pub runtime_context: R,
    pub events: E,
    pub fact_log: F,
}

pub struct CallExecutionService<C, L, R, E, F> {
    control: C,
    launch: L,
    runtime_context: R,
    events: E,
    fact_log: F,
}

impl<C, L, R, E, F> CallExecutionService<C, L, R, E, F> {
    pub fn new(deps: CallExecutionDependencies<C, L, R, E, F>) -> Self {
        Self {
            control: deps.control,
            launch: deps.launch,
            runtime_context: deps.runtime_context,
            events: deps.events,
            fact_log: deps.fact_log,
        }
    }
}

impl<C, L, R, E, F> CallExecutionService<C, L, R, E, F>
where
    C: CallExecutionControlUseCases + Send + Sync,
{
    async fn converge_claim_failure(
        &self,
        descriptor: &RuntimeWorkDescriptor,
        failure_reason: impl Into<String>,
    ) -> AppResult<()> {
        self.control
            .mark_runtime_failed(MarkRuntimeFailedCommand {
                session_id: descriptor.session_id,
                runtime_owner_id: descriptor.runtime_owner_id.clone(),
                failure_reason: Some(failure_reason.into()),
            })
            .await
    }
}

#[async_trait]
pub trait CallExecutionUseCases: Send + Sync {
    async fn poll_runtime_work(
        &self,
        command: PollRuntimeWorkCommand,
    ) -> AppResult<Option<RuntimeWorkItem>>;
    async fn report_runtime_status(&self, command: ReportRuntimeStatusCommand) -> AppResult<()>;
    async fn publish_runtime_event(&self, command: PublishRuntimeEventCommand) -> AppResult<()>;
}

#[async_trait]
impl<C, L, R, E, F> CallExecutionUseCases for CallExecutionService<C, L, R, E, F>
where
    C: CallExecutionControlUseCases + Send + Sync,
    L: RuntimeLaunchProvisionPort + Send + Sync,
    E: CallExecutionEventPort + Send + Sync,
    F: RuntimeFactLogPort + Send + Sync,
    R: RuntimeSessionContextPort + Send + Sync,
{
    async fn poll_runtime_work(
        &self,
        command: PollRuntimeWorkCommand,
    ) -> AppResult<Option<RuntimeWorkItem>> {
        let poll_span = tracing::info_span!(
            "poll_runtime_work",
            runtime_owner_id = %command.runtime_owner_id
        );
        async move {
            if command.runtime_owner_id.trim().is_empty() {
                return Err(AppError::invalid_input("runtime_owner_id is required"));
            }

            if let Some(descriptor) = self
                .control
                .claim_runtime_start_work(ClaimRuntimeStartCommand {
                    runtime_owner_id: command.runtime_owner_id.clone(),
                })
                .await?
            {
                if let Err(error) = self
                    .events
                    .publish(CallExecutionEvent {
                        session_id: descriptor.session_id,
                        round_id: None,
                        source: "call_execution".to_owned(),
                        event: "runtime_start_claimed".to_owned(),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                        fields: json!({
                            "runtime_owner_id": descriptor.runtime_owner_id,
                        }),
                    })
                    .await
                {
                    self.converge_claim_failure(&descriptor, error.to_string())
                        .await?;
                    return Err(error);
                }
                let runtime_context = match self
                    .runtime_context
                    .get_runtime_session_context(descriptor.session_id)
                    .await?
                {
                    Some(context) => context,
                    None => {
                        let error = AppError::not_found("runtime session context not found");
                        self.converge_claim_failure(&descriptor, error.to_string())
                            .await?;
                        return Err(error);
                    }
                };
                let launch = match async {
                    self.launch
                        .prepare_runtime_launch(
                            descriptor.session_id,
                            descriptor.remote_participant_identity.clone(),
                            descriptor.voice.clone(),
                            &runtime_context.agent_session_id,
                            runtime_context.asr_language.as_str(),
                        )
                        .await
                }
                .instrument(tracing::info_span!(
                    "prepare_runtime_launch",
                    session_id = descriptor.session_id,
                    remote_participant_identity = %descriptor.remote_participant_identity,
                    voice = descriptor.voice.clone(),
                ))
                .await
                {
                    Ok(launch) => launch,
                    Err(error) => {
                        self.converge_claim_failure(&descriptor, error.to_string())
                            .await?;
                        return Err(error);
                    }
                };
                if let Err(error) = self
                    .events
                    .publish(CallExecutionEvent {
                        session_id: descriptor.session_id,
                        round_id: None,
                        source: "call_execution".to_owned(),
                        event: "runtime_launch_prepared".to_owned(),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                        fields: json!({
                            "runtime_owner_id": descriptor.runtime_owner_id,
                            "room_name": launch.room_name,
                            "agent_session_id": runtime_context.agent_session_id,
                        }),
                    })
                    .await
                {
                    self.converge_claim_failure(&descriptor, error.to_string())
                        .await?;
                    return Err(error);
                }
                return Ok(Some(map_work_item(descriptor, Some(launch))));
            }

            if let Some(descriptor) = self
                .control
                .claim_runtime_stop_work(ClaimRuntimeStopCommand {
                    runtime_owner_id: command.runtime_owner_id,
                })
                .await?
            {
                if let Err(error) = self
                    .events
                    .publish(CallExecutionEvent {
                        session_id: descriptor.session_id,
                        round_id: None,
                        source: "call_execution".to_owned(),
                        event: "runtime_stop_claimed".to_owned(),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                        fields: json!({
                            "runtime_owner_id": descriptor.runtime_owner_id,
                        }),
                    })
                    .await
                {
                    self.converge_claim_failure(&descriptor, error.to_string())
                        .await?;
                    return Err(error);
                }
                return Ok(Some(map_work_item(descriptor, None)));
            }

            Ok(None)
        }
        .instrument(poll_span)
        .await
    }

    async fn report_runtime_status(&self, command: ReportRuntimeStatusCommand) -> AppResult<()> {
        if command.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input("runtime_owner_id is required"));
        }

        match command.status {
            RuntimeStatusKind::Ready => {
                self.control
                    .mark_runtime_ready(MarkRuntimeReadyCommand {
                        session_id: command.session_id,
                        runtime_owner_id: command.runtime_owner_id,
                    })
                    .await
            }
            RuntimeStatusKind::Failed => {
                self.control
                    .mark_runtime_failed(MarkRuntimeFailedCommand {
                        session_id: command.session_id,
                        runtime_owner_id: command.runtime_owner_id,
                        failure_reason: command.failure_reason,
                    })
                    .await
            }
            RuntimeStatusKind::Stopped => {
                self.control
                    .mark_runtime_stopped(MarkRuntimeStoppedCommand {
                        session_id: command.session_id,
                        runtime_owner_id: command.runtime_owner_id,
                    })
                    .await
            }
            RuntimeStatusKind::Missing => {
                self.control
                    .mark_runtime_missing(MarkRuntimeMissingCommand {
                        session_id: command.session_id,
                        runtime_owner_id: command.runtime_owner_id,
                    })
                    .await
            }
        }
    }

    async fn publish_runtime_event(&self, command: PublishRuntimeEventCommand) -> AppResult<()> {
        if command.fact.runtime_owner_id.trim().is_empty() {
            return Err(AppError::invalid_input(
                "runtime event runtime_owner_id is required",
            ));
        }
        if command.fact.source.trim().is_empty() {
            return Err(AppError::invalid_input("runtime event source is required"));
        }
        if command.fact.ts_ms <= 0 {
            return Err(AppError::invalid_input(
                "runtime event timestamp must be positive",
            ));
        }
        validate_runtime_fact(&command.fact)?;
        let context = self
            .runtime_context
            .get_runtime_session_context(command.fact.session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        if context.runtime_owner_id.as_deref() != Some(command.fact.runtime_owner_id.as_str()) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }
        let (event, fields) = map_runtime_fact_kind(&command.fact.kind);
        self.fact_log.append(&command.fact).await?;
        self.events
            .publish(CallExecutionEvent {
                session_id: command.fact.session_id,
                round_id: command.fact.round_id,
                source: command.fact.source,
                event,
                ts_ms: command.fact.ts_ms,
                fields,
            })
            .await
    }
}

#[async_trait]
impl<C, L, R, E, F> RuntimeSessionContextPort for CallExecutionService<C, L, R, E, F>
where
    C: CallExecutionControlUseCases + Send + Sync,
    L: RuntimeLaunchProvisionPort + Send + Sync,
    R: RuntimeSessionContextPort + Send + Sync,
    E: CallExecutionEventPort + Send + Sync,
    F: RuntimeFactLogPort + Send + Sync,
{
    async fn get_runtime_session_context(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionContext>> {
        self.runtime_context
            .get_runtime_session_context(session_id)
            .await
    }
}

fn map_work_item(
    descriptor: RuntimeWorkDescriptor,
    launch: Option<RuntimeLaunchSpec>,
) -> RuntimeWorkItem {
    RuntimeWorkItem {
        session_id: descriptor.session_id,
        runtime_owner_id: descriptor.runtime_owner_id,
        kind: match descriptor.kind {
            RuntimeWorkDescriptorKind::Start => RuntimeWorkKind::Start,
            RuntimeWorkDescriptorKind::Stop => RuntimeWorkKind::Stop,
        },
        launch,
    }
}

fn validate_runtime_fact(fact: &RuntimeEventFact) -> AppResult<()> {
    let requires_round_id = matches!(
        fact.kind,
        RuntimeFactKind::RuntimeReplyStarted { .. }
            | RuntimeFactKind::RuntimeReplyFinished
            | RuntimeFactKind::RuntimePlaybackCompleted
            | RuntimeFactKind::ExternalAudioStarted { .. }
            | RuntimeFactKind::ExternalAudioFinished { .. }
            | RuntimeFactKind::RuntimeInputRoundFailed { .. }
    );
    if requires_round_id
        && fact
            .round_id
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Err(AppError::invalid_input(
            "round_id is required for the runtime fact kind",
        ));
    }
    Ok(())
}

fn map_runtime_fact_kind(kind: &RuntimeFactKind) -> (String, serde_json::Value) {
    match kind {
        RuntimeFactKind::WorkerRuntimeStarted => ("worker_runtime_started".to_owned(), json!({})),
        RuntimeFactKind::WorkerRuntimeReady => ("worker_runtime_ready".to_owned(), json!({})),
        RuntimeFactKind::WorkerRuntimeStopped => ("worker_runtime_stopped".to_owned(), json!({})),
        RuntimeFactKind::WorkerRuntimeFailed { reason } => (
            "worker_runtime_failed".to_owned(),
            json!({ "reason": reason }),
        ),
        RuntimeFactKind::WorkerRuntimeMissing => ("worker_runtime_missing".to_owned(), json!({})),
        RuntimeFactKind::RuntimeReplyStarted { reply_text } => (
            "runtime_reply_started".to_owned(),
            json!({ "reply_text": reply_text }),
        ),
        RuntimeFactKind::RuntimeReplyFinished => ("runtime_reply_finished".to_owned(), json!({})),
        RuntimeFactKind::RuntimePlaybackCompleted => {
            ("runtime_playback_completed".to_owned(), json!({}))
        }
        RuntimeFactKind::ExternalAudioStarted { audio_url, band } => (
            "external_audio_started".to_owned(),
            json!({ "audio_url": audio_url, "band": band }),
        ),
        RuntimeFactKind::ExternalAudioFinished { audio_url, band } => (
            "external_audio_finished".to_owned(),
            json!({ "audio_url": audio_url, "band": band }),
        ),
        RuntimeFactKind::WorkerBargeInDetected { rms } => {
            ("worker_barge_in_detected".to_owned(), json!({ "rms": rms }))
        }
        RuntimeFactKind::RuntimeInputRoundFailed { reason } => (
            "runtime_input_round_failed".to_owned(),
            json!({ "reason": reason }),
        ),
    }
}
