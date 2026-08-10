mod segmentation;
pub(crate) mod session_progress;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use call_runtime_control::SpeechInputMediaState;
use futures::StreamExt;
pub use segmentation::{
    SpeechSegmentationConfig, SpeechSegmentationConfigPort, SpeechSegmentationDecision,
    SpeechSegmentationPolicy, ThresholdSpeechSegmentationPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shared_kernel::{AppError, AppErrorCode, AppResult};
use tokio::{
    sync::{Mutex, oneshot, watch},
    task::JoinHandle,
};

const DEFAULT_MAX_EVENTS: usize = 32;
pub(crate) const TERMINAL_SESSION_MESSAGE: &str = "speech session is terminal";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechRuntimeContext {
    pub session_id: i64,
    pub agent_session_id: String,
    pub voice: String,
    pub runtime_owner_id: String,
    pub language: voice::AsrLanguage,
}

#[async_trait]
pub trait SpeechRuntimeContextPort: Send + Sync {
    async fn resolve_accepting_runtime_context(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
    ) -> AppResult<Option<SpeechRuntimeContext>>;
}

#[async_trait]
impl<T> SpeechRuntimeContextPort for Arc<T>
where
    T: SpeechRuntimeContextPort + Send + Sync + ?Sized,
{
    async fn resolve_accepting_runtime_context(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
    ) -> AppResult<Option<SpeechRuntimeContext>> {
        (**self)
            .resolve_accepting_runtime_context(session_id, runtime_owner_id)
            .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechSessionPhase {
    Listening,
    SpeechDetected,
    Flushing,
    Responding,
    Closing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSpeechTurn {
    pub round_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAsrRound {
    pub round_id: String,
    #[serde(default)]
    pub latest_partial_transcript: String,
    #[serde(default)]
    pub commit_started_at_ms: Option<i64>,
    #[serde(default)]
    pub force_agent_deadline_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSpeechSession {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub agent_session_id: String,
    pub owner_backend_url: String,
    pub voice: String,
    pub asr_session_id: String,
    pub asr_owner_instance_id: String,
    pub asr_owner_instance_epoch: String,
    pub language: voice::AsrLanguage,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
    pub segmentation_config: SpeechSegmentationConfig,
    pub utterance_pcm: Vec<i16>,
    pub phase: SpeechSessionPhase,
    pub trailing_silence_ms: u32,
    #[serde(default)]
    pub deferred_flush: bool,
    pub next_round_seq: u32,
    #[serde(default, deserialize_with = "deserialize_pending_rounds")]
    pub pending_rounds: VecDeque<PendingAsrRound>,
    pub dispatched_round_ids: Vec<String>,
    #[serde(default)]
    pub active_turn: Option<ActiveSpeechTurn>,
    /// 起话候选累计语音时长(ms);达到 min_speech_confirm_ms 才确认进入 utterance。
    #[serde(default)]
    pub candidate_speech_ms: u32,
    /// 候选期缓冲的原始帧;确认时整段并入 utterance(前缀回溯,不丢说话开头)。
    #[serde(default)]
    pub candidate_pcm: Vec<i16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRoundCommitRequest {
    round_id: String,
    force_agent_deadline_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForcedTranscriptTurn {
    round_id: String,
    transcript: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum PendingAsrRoundSerde {
    Round(PendingAsrRound),
    RoundId(String),
}

fn deserialize_pending_rounds<'de, D>(
    deserializer: D,
) -> Result<VecDeque<PendingAsrRound>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<PendingAsrRoundSerde>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|value| match value {
            PendingAsrRoundSerde::Round(round) => round,
            PendingAsrRoundSerde::RoundId(round_id) => PendingAsrRound {
                round_id,
                latest_partial_transcript: String::new(),
                commit_started_at_ms: None,
                force_agent_deadline_at_ms: None,
            },
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechSessionInputProgressSaveResult {
    Saved,
    Dropped,
    Terminal,
}

#[async_trait]
pub trait SpeechSessionStorePort: Send + Sync {
    async fn create_session(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<()>;
    async fn get_session(&self, speech_session_id: &str) -> AppResult<Option<StoredSpeechSession>>;
    async fn save_session(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<()>;
    /// Persist the input/ASR projection without taking ownership of turn or terminal lifecycle
    /// fields.
    async fn save_input_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult>;
    /// Persist provider ASR progress after provider events have been consumed; this must not use
    /// normal input admission drop semantics.
    async fn save_asr_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult>;
    /// Persist ASR drain progress during close while preserving the durable `Closing` lifecycle.
    async fn save_closing_drain_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult>;
    async fn save_session_and_push_events(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
        events: &[SpeechRuntimeEvent],
    ) -> AppResult<()>;
    async fn take_session(&self, speech_session_id: &str)
    -> AppResult<Option<StoredSpeechSession>>;
    async fn push_events(
        &self,
        speech_session_id: &str,
        events: &[SpeechRuntimeEvent],
    ) -> AppResult<()>;
    async fn pop_events(
        &self,
        speech_session_id: &str,
        max_events: usize,
    ) -> AppResult<Vec<SpeechRuntimeEvent>>;
    async fn drain_events(&self, speech_session_id: &str) -> AppResult<Vec<SpeechRuntimeEvent>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSpeechSessionCommand {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSpeechSessionResult {
    pub speech_session_id: String,
    pub owner_backend_url: String,
    pub owner_instance_id: String,
    pub owner_instance_epoch: String,
    pub owner_route_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSpeechInputCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
    pub pcm_s16le: Vec<i16>,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
    pub media_state: SpeechInputMediaState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollSpeechEventsCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
    pub max_events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseSpeechSessionCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSpeechSessionCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailOwnerRouteCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
    pub owner_backend_url: String,
    pub owner_instance_id: String,
    pub owner_instance_epoch: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitServerTurnCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
    pub round_id: String,
    pub reply_text: String,
    pub interruptible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitPreRecordedTurnCommand {
    pub speech_session_id: String,
    pub runtime_owner_id: String,
    pub round_id: String,
    pub audio_url: String,
    pub band: String,
    pub interruptible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpeechRuntimeEvent {
    ListeningStarted,
    SpeechDetected,
    UtteranceFlushing,
    RespondingStarted,
    SessionClosing,
    SessionFailed {
        message: String,
    },
    ReplyStarted {
        round_id: String,
        reply_text: String,
        interruptible: bool,
    },
    AudioChunk {
        pcm_s16le: Vec<i16>,
        sample_rate_hz: u32,
        channels: u16,
    },
    PreRecordedAsset {
        round_id: String,
        audio_url: String,
        band: String,
        interruptible: bool,
    },
    ReplyFinished {
        round_id: String,
    },
    RoundFailed {
        round_id: String,
        message: String,
    },
    Warning {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollSpeechEventsResult {
    pub events: Vec<SpeechRuntimeEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseSpeechSessionResult {
    pub events: Vec<SpeechRuntimeEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptSpeechSessionResult {
    pub events: Vec<SpeechRuntimeEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechLogEvent {
    pub session_id: i64,
    pub round_id: Option<String>,
    pub event: String,
    pub fields: serde_json::Value,
}

#[async_trait]
pub trait SpeechRuntimeEventPort: Send + Sync {
    async fn publish(&self, event: SpeechLogEvent) -> AppResult<()>;
}

pub struct SpeechRuntimeDependencies<C, G, S, E> {
    pub runtime_context: C,
    pub segmentation_config: G,
    pub session_store: S,
    pub events: E,
    pub instance_id: String,
    pub instance_epoch: String,
    pub internal_runtime_advertise_url: String,
    pub segmentation_policy: Arc<dyn SpeechSegmentationPolicy>,
    pub voice_runtime: Arc<dyn voice::VoiceRuntimeExecutionPort>,
    pub agent_turn: Arc<dyn AgentTurnPort>,
}

pub struct SpeechRuntimeService<C, G, S, E> {
    runtime_context: C,
    segmentation_config: G,
    session_store: S,
    events: E,
    instance_id: String,
    instance_epoch: String,
    internal_runtime_advertise_url: String,
    segmentation_policy: Arc<dyn SpeechSegmentationPolicy>,
    voice_runtime: Arc<dyn voice::VoiceRuntimeExecutionPort>,
    agent_turn: Arc<dyn AgentTurnPort>,
    turn_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    turn_start_signals: Arc<Mutex<HashMap<String, watch::Receiver<TurnStartState>>>>,
    turn_gates: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    state_gates: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    closing_sessions: Arc<Mutex<HashSet<String>>>,
}

impl<C, G, S, E> SpeechRuntimeService<C, G, S, E> {
    pub fn new(deps: SpeechRuntimeDependencies<C, G, S, E>) -> Self {
        Self {
            runtime_context: deps.runtime_context,
            segmentation_config: deps.segmentation_config,
            session_store: deps.session_store,
            events: deps.events,
            instance_id: deps.instance_id,
            instance_epoch: deps.instance_epoch,
            internal_runtime_advertise_url: deps.internal_runtime_advertise_url,
            segmentation_policy: deps.segmentation_policy,
            voice_runtime: deps.voice_runtime,
            agent_turn: deps.agent_turn,
            turn_tasks: Arc::new(Mutex::new(HashMap::new())),
            turn_start_signals: Arc::new(Mutex::new(HashMap::new())),
            turn_gates: Arc::new(Mutex::new(HashMap::new())),
            state_gates: Arc::new(Mutex::new(HashMap::new())),
            closing_sessions: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurnRequest {
    pub session_id: String,
    pub user_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurnResult {
    pub reply_text: String,
}

#[async_trait]
pub trait AgentTurnPort: Send + Sync {
    async fn chat_once(&self, request: AgentTurnRequest) -> AppResult<AgentTurnResult>;
}

#[async_trait]
pub trait SpeechRuntimeUseCases: Send + Sync {
    async fn create_session(
        &self,
        command: CreateSpeechSessionCommand,
    ) -> AppResult<CreateSpeechSessionResult>;
    async fn push_input_audio(&self, command: PushSpeechInputCommand) -> AppResult<()>;
    async fn poll_events(
        &self,
        command: PollSpeechEventsCommand,
    ) -> AppResult<PollSpeechEventsResult>;
    async fn close_session(
        &self,
        command: CloseSpeechSessionCommand,
    ) -> AppResult<CloseSpeechSessionResult>;
    async fn interrupt_session(
        &self,
        command: InterruptSpeechSessionCommand,
    ) -> AppResult<InterruptSpeechSessionResult>;
    async fn fail_owner_route(&self, command: FailOwnerRouteCommand) -> AppResult<()>;
    async fn submit_server_turn(&self, command: SubmitServerTurnCommand) -> AppResult<()>;
    async fn submit_pre_recorded_turn(
        &self,
        command: SubmitPreRecordedTurnCommand,
    ) -> AppResult<()>;
}

#[async_trait]
impl<C, G, S, E> SpeechRuntimeUseCases for SpeechRuntimeService<C, G, S, E>
where
    C: SpeechRuntimeContextPort + Send + Sync,
    G: SpeechSegmentationConfigPort + Send + Sync,
    S: SpeechSessionStorePort + Clone + Send + Sync + 'static,
    E: SpeechRuntimeEventPort + Clone + Send + Sync + 'static,
{
    async fn create_session(
        &self,
        command: CreateSpeechSessionCommand,
    ) -> AppResult<CreateSpeechSessionResult> {
        validate_runtime_owner(&command.runtime_owner_id)?;
        if command.sample_rate_hz == 0 || command.num_channels == 0 {
            return Err(AppError::invalid_input(
                "sample_rate_hz and num_channels must be positive",
            ));
        }

        let context = self
            .require_runtime_context(command.session_id, &command.runtime_owner_id)
            .await?;
        let segmentation_config = self
            .segmentation_config
            .get_speech_segmentation_config()
            .await?;
        let speech_session_id = speech_session_id(command.session_id);
        let state_gate = self.state_gate(&speech_session_id).await;
        let _state_guard = state_gate.lock().await;
        if let Some(existing) = self
            .load_existing_session_for_create(&speech_session_id, &command)
            .await?
        {
            return Ok(existing);
        }
        let asr_session_id = self
            .voice_runtime
            .open_asr_session_for_runtime(voice::OpenRuntimeAsrSessionRequest {
                speech_session_id: speech_session_id.clone(),
                sample_rate_hz: command.sample_rate_hz,
                num_channels: command.num_channels,
                language: context.language,
            })
            .await?
            .asr_session_id;
        let session_id = command.session_id;
        let runtime_owner_id = command.runtime_owner_id.clone();
        let asr_owner = AsrSessionOwner {
            asr_session_id,
            instance_id: self.instance_id.clone(),
            instance_epoch: self.instance_epoch.clone(),
        };

        let stored_session = StoredSpeechSession::new(
            command,
            context.agent_session_id,
            self.internal_runtime_advertise_url.clone(),
            context.voice.clone(),
            asr_owner.clone(),
            context.language,
            segmentation_config,
        );
        if let Err(error) = self
            .session_store
            .create_session(&speech_session_id, &stored_session)
            .await
        {
            let _ = self
                .rollback_created_session(&speech_session_id, &asr_owner.asr_session_id)
                .await;
            return Err(error);
        }
        if let Err(error) = self
            .session_store
            .push_events(&speech_session_id, &[SpeechRuntimeEvent::ListeningStarted])
            .await
        {
            let _ = self
                .rollback_created_session(&speech_session_id, &asr_owner.asr_session_id)
                .await;
            return Err(error);
        }
        if let Err(error) = self
            .publish_log_event(
                session_id,
                None,
                "speech_listening_started",
                json!({
                    "speech_session_id": speech_session_id,
                    "runtime_owner_id": runtime_owner_id,
                }),
            )
            .await
        {
            let _ = self
                .rollback_created_session(&speech_session_id, &asr_owner.asr_session_id)
                .await;
            return Err(error);
        }

        Ok(CreateSpeechSessionResult {
            speech_session_id,
            owner_backend_url: self.internal_runtime_advertise_url.clone(),
            owner_instance_id: self.instance_id.clone(),
            owner_instance_epoch: self.instance_epoch.clone(),
            owner_route_only: false,
        })
    }

    async fn push_input_audio(&self, command: PushSpeechInputCommand) -> AppResult<()> {
        validate_runtime_owner(&command.runtime_owner_id)?;
        if command.pcm_s16le.is_empty() {
            return Err(AppError::invalid_input("pcm_s16le is required"));
        }
        if command.sample_rate_hz == 0 || command.num_channels == 0 {
            return Err(AppError::invalid_input(
                "sample_rate_hz and num_channels must be positive",
            ));
        }

        let session_id = parse_speech_session_id(&command.speech_session_id)?;
        self.require_runtime_context(session_id, &command.runtime_owner_id)
            .await?;

        let speech_session_id = command.speech_session_id.clone();
        let state_gate = self.state_gate(&speech_session_id).await;
        let (mut session, transition, asr_audio_to_push) = {
            let _state_guard = state_gate.lock().await;
            let mut session = self
                .load_owned_session(&speech_session_id, &command.runtime_owner_id)
                .await?;
            if self
                .fail_orphaned_active_turn_on_local_route(&speech_session_id, &mut session)
                .await?
            {
                return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
            }
            self.ensure_backend_instance_owner(&session)?;
            self.ensure_local_asr_owner(&session)?;
            require_active_session(&session)?;

            if should_reset_live_input_buffer(&session, command.media_state) {
                session.reset_live_input_buffer();
            }
            if let Some(reason) = input_drop_reason(&session, command.media_state) {
                let save_result = self
                    .save_input_progress(&speech_session_id, &session, InputProgressMode::Active)
                    .await?;
                tracing::debug!(
                    session_id,
                    speech_session_id = %speech_session_id,
                    reason,
                    phase = ?session.phase,
                    media_state = ?command.media_state,
                    active_round_id = ?session.active_turn.as_ref().map(|turn| turn.round_id.as_str()),
                    save_result = ?save_result,
                    "speech runtime dropped input frame without entering ASR"
                );
                return Ok(());
            }

            let (transition, asr_audio_to_push) =
                session.accept_input(&command, self.segmentation_policy.as_ref());
            if let Some(commit_request) = transition.commit_request.as_ref() {
                session.push_pending_round(commit_request.clone());
            }
            let save_result = self
                .save_input_progress(&speech_session_id, &session, InputProgressMode::Active)
                .await?;
            if matches!(save_result, SpeechSessionInputProgressSaveResult::Dropped) {
                tracing::debug!(
                    session_id,
                    speech_session_id = %speech_session_id,
                    reason = "concurrent_live_output_turn",
                    phase = ?session.phase,
                    media_state = ?command.media_state,
                    active_round_id = ?session.active_turn.as_ref().map(|turn| turn.round_id.as_str()),
                    "speech runtime dropped input frame after state compare"
                );
                return Ok(());
            }

            if !transition.events.is_empty() {
                self.session_store
                    .push_events(&speech_session_id, &transition.events)
                    .await?;
                self.publish_transition_events(
                    session_id,
                    transition
                        .commit_request
                        .as_ref()
                        .map(|request| request.round_id.as_str()),
                    &transition.events,
                )
                .await?;
            }
            (session, transition, asr_audio_to_push)
        };

        // 仅在 transition 指示时 append 给 ASR:候选期(SpeechPending)为 None,不推噪音;
        // 确认时补发候选前缀+当前帧;其余(含空闲静音)推当前帧维持 provider 活性。
        if let Some(asr_pcm) = asr_audio_to_push
            && let Err(error) = self
                .voice_runtime
                .push_asr_audio_for_runtime(voice::PushRuntimeAsrAudioRequest {
                    asr_session_id: session.asr_session_id.clone(),
                    pcm_s16le: asr_pcm,
                    sample_rate_hz: command.sample_rate_hz,
                    num_channels: command.num_channels,
                })
                .await
        {
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }

        if let Some(commit_request) = transition.commit_request {
            let round_id = commit_request.round_id;
            self.publish_log_event(
                session_id,
                Some(round_id.as_str()),
                "speech_asr_commit_requested",
                json!({
                    "speech_session_id": speech_session_id,
                    "asr_session_id": session.asr_session_id,
                }),
            )
            .await?;
            if let Err(error) = self
                .voice_runtime
                .commit_asr_session_for_runtime(voice::CommitRuntimeAsrSessionRequest {
                    asr_session_id: session.asr_session_id.clone(),
                    round_id: round_id.clone(),
                })
                .await
            {
                let _state_guard = state_gate.lock().await;
                self.record_asr_round_failed(
                    &speech_session_id,
                    &mut session,
                    &round_id,
                    &error.to_string(),
                    true,
                    InputProgressMode::Active,
                )
                .await?;
                return Ok(());
            }
        }

        if let Err(error) = self
            .drive_asr_runtime(
                &speech_session_id,
                &mut session,
                true,
                true,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return Err(error);
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }
        if let Err(error) = self
            .commit_deferred_flush_if_ready(
                &speech_session_id,
                &mut session,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return Err(error);
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }
        if let Err(error) = self
            .drive_asr_runtime(
                &speech_session_id,
                &mut session,
                true,
                true,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return Err(error);
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }

        Ok(())
    }

    async fn poll_events(
        &self,
        command: PollSpeechEventsCommand,
    ) -> AppResult<PollSpeechEventsResult> {
        validate_runtime_owner(&command.runtime_owner_id)?;

        let state_gate = self.state_gate(&command.speech_session_id).await;
        let mut session = {
            let _state_guard = state_gate.lock().await;
            let mut session = self
                .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
                .await?;
            if self
                .fail_orphaned_active_turn_on_local_route(&command.speech_session_id, &mut session)
                .await?
            {
                return self.pop_session_events(&command).await;
            }
            self.ensure_backend_instance_owner(&session)?;
            if matches!(
                session.phase,
                SpeechSessionPhase::Closing | SpeechSessionPhase::Failed
            ) {
                return self.pop_session_events(&command).await;
            }
            self.ensure_local_asr_owner(&session)?;
            session
        };

        if let Err(error) = self
            .drive_asr_runtime(
                &command.speech_session_id,
                &mut session,
                true,
                true,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return self.pop_session_events(&command).await;
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&command.speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }
        if let Err(error) = self
            .commit_deferred_flush_if_ready(
                &command.speech_session_id,
                &mut session,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return self.pop_session_events(&command).await;
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&command.speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }
        if let Err(error) = self
            .drive_asr_runtime(
                &command.speech_session_id,
                &mut session,
                true,
                true,
                InputProgressMode::Active,
                &state_gate,
            )
            .await
        {
            if is_stale_turn_error(&error) {
                return self.pop_session_events(&command).await;
            }
            let _state_guard = state_gate.lock().await;
            self.fail_speech_session(&command.speech_session_id, &mut session, &error.to_string())
                .await?;
            return Err(error);
        }

        let max_events = command.max_events.clamp(1, DEFAULT_MAX_EVENTS);
        let events = self
            .session_store
            .pop_events(&command.speech_session_id, max_events)
            .await?;

        Ok(PollSpeechEventsResult { events })
    }

    async fn close_session(
        &self,
        command: CloseSpeechSessionCommand,
    ) -> AppResult<CloseSpeechSessionResult> {
        validate_runtime_owner(&command.runtime_owner_id)?;

        let state_gate = self.state_gate(&command.speech_session_id).await;
        let _state_guard = state_gate.lock().await;
        let mut session = self
            .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
            .await?;
        if self
            .fail_orphaned_active_turn_on_local_route(&command.speech_session_id, &mut session)
            .await?
        {
            let events = self
                .session_store
                .drain_events(&command.speech_session_id)
                .await?;
            let _ = self
                .session_store
                .take_session(&command.speech_session_id)
                .await?
                .ok_or_else(|| AppError::not_found("speech session not found"))?;
            self.remove_state_gate(&command.speech_session_id).await;
            return Ok(CloseSpeechSessionResult { events });
        }
        self.ensure_backend_instance_owner(&session)?;
        if matches!(session.phase, SpeechSessionPhase::Closing) {
            return Err(AppError::conflict(
                "speech session close is already in progress",
            ));
        }
        if matches!(session.phase, SpeechSessionPhase::Failed) {
            self.abort_turn_tasks(&command.speech_session_id).await?;
            self.remove_turn_gate(&command.speech_session_id).await;
            let events = self
                .session_store
                .drain_events(&command.speech_session_id)
                .await?;
            let _ = self
                .session_store
                .take_session(&command.speech_session_id)
                .await?
                .ok_or_else(|| AppError::not_found("speech session not found"))?;
            self.remove_state_gate(&command.speech_session_id).await;
            return Ok(CloseSpeechSessionResult { events });
        }
        self.ensure_local_asr_owner(&session)?;

        self.mark_session_closing(&command.speech_session_id).await;
        let close_result: AppResult<()> = async {
            self.abort_turn_tasks(&command.speech_session_id).await?;
            if !matches!(session.phase, SpeechSessionPhase::Failed) {
                session.phase = SpeechSessionPhase::Closing;
                session.active_turn = None;
                self.session_store
                    .save_session(&command.speech_session_id, &session)
                    .await?;
                self.session_store
                    .push_events(
                        &command.speech_session_id,
                        &[SpeechRuntimeEvent::SessionClosing],
                    )
                    .await?;
            }
            self.publish_log_event(
                session.session_id,
                None,
                "speech_session_closing",
                json!({}),
            )
            .await?;
            drop(_state_guard);
            self.drain_asr_runtime_for_close(&command.speech_session_id, &mut session, &state_gate)
                .await?;
            self.voice_runtime
                .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                    asr_session_id: session.asr_session_id.clone(),
                })
                .await?;
            Ok(())
        }
        .await;
        if let Err(error) = close_result {
            if is_terminal_session_conflict(&error) {
                let _ = self
                    .voice_runtime
                    .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                        asr_session_id: session.asr_session_id.clone(),
                    })
                    .await;
                self.remove_turn_gate(&command.speech_session_id).await;
                let drained_events = self
                    .session_store
                    .drain_events(&command.speech_session_id)
                    .await?;
                let _ = self
                    .session_store
                    .take_session(&command.speech_session_id)
                    .await?;
                self.clear_session_closing(&command.speech_session_id).await;
                self.remove_state_gate(&command.speech_session_id).await;
                return Ok(CloseSpeechSessionResult {
                    events: drained_events,
                });
            }
            let _state_guard = state_gate.lock().await;
            let _ = self
                .fail_speech_session(&command.speech_session_id, &mut session, &error.to_string())
                .await;
            self.clear_session_closing(&command.speech_session_id).await;
            return Err(error);
        }
        self.remove_turn_gate(&command.speech_session_id).await;

        let drained_events = self
            .session_store
            .drain_events(&command.speech_session_id)
            .await?;
        let _ = self
            .session_store
            .take_session(&command.speech_session_id)
            .await?
            .ok_or_else(|| AppError::not_found("speech session not found"))?;
        self.clear_session_closing(&command.speech_session_id).await;
        self.remove_state_gate(&command.speech_session_id).await;

        Ok(CloseSpeechSessionResult {
            events: drained_events,
        })
    }

    async fn interrupt_session(
        &self,
        command: InterruptSpeechSessionCommand,
    ) -> AppResult<InterruptSpeechSessionResult> {
        validate_runtime_owner(&command.runtime_owner_id)?;

        let state_gate = self.state_gate(&command.speech_session_id).await;
        let _state_guard = state_gate.lock().await;
        let mut session = self
            .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
            .await?;
        self.ensure_backend_instance_owner(&session)?;
        self.ensure_local_asr_owner(&session)?;

        // 轮换 ASR:先开好新连接(成功才推进);open 失败时尚未改动 turn/session,worker 重试即可。
        let old_asr_session_id = session.asr_session_id.clone();
        let new_asr_session_id = self
            .voice_runtime
            .open_asr_session_for_runtime(voice::OpenRuntimeAsrSessionRequest {
                speech_session_id: command.speech_session_id.clone(),
                sample_rate_hz: session.sample_rate_hz,
                num_channels: session.num_channels,
                language: session.language,
            })
            .await?
            .asr_session_id;

        // open 成功后再中止当前 turn;此后任一步失败都 rollback 关闭刚开的新 ASR,避免 orphan/半改。
        if let Err(error) = self.abort_turn_tasks(&command.speech_session_id).await {
            self.spawn_close_asr(new_asr_session_id);
            return Err(error);
        }
        self.remove_turn_gate(&command.speech_session_id).await;

        // 复位到可监听状态:清当前轮输入/候选与未决 round,轮换到新 ASR;保留 dispatched_round_ids 忽略 late final。
        session.reset_for_interrupt(
            new_asr_session_id.clone(),
            self.instance_id.clone(),
            self.instance_epoch.clone(),
        );

        // 持久化 Listening 态(保留会话与 state_gate)。失败则 rollback 新 ASR,避免 orphan ASR 泄漏;
        // 否则 Redis compare 在 Responding 态会丢弃后续输入。
        if let Err(error) = self
            .session_store
            .save_session(&command.speech_session_id, &session)
            .await
        {
            self.spawn_close_asr(new_asr_session_id);
            return Err(error);
        }

        // 旧 ASR 后台关闭并忽略失败(不在持锁时等待可能很慢的 websocket close)。
        self.spawn_close_asr(old_asr_session_id);

        // 打断此时已生效(已 reset+save);drain 失败不应让 worker 收到错误而退出 pipeline。
        let drained_events = match self
            .session_store
            .drain_events(&command.speech_session_id)
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    speech_session_id = %command.speech_session_id,
                    "drain events after interrupt failed; proceeding with empty events"
                );
                Vec::new()
            }
        };

        Ok(InterruptSpeechSessionResult {
            events: drained_events,
        })
    }

    async fn fail_owner_route(&self, command: FailOwnerRouteCommand) -> AppResult<()> {
        validate_runtime_owner(&command.runtime_owner_id)?;
        if command.owner_backend_url.trim().is_empty() {
            return Err(AppError::invalid_input("owner_backend_url is required"));
        }
        if command.owner_instance_id.trim().is_empty() {
            return Err(AppError::invalid_input("owner_instance_id is required"));
        }
        if command.owner_instance_epoch.trim().is_empty() {
            return Err(AppError::invalid_input("owner_instance_epoch is required"));
        }
        if command.reason.trim().is_empty() {
            return Err(AppError::invalid_input(
                "owner route failure reason is required",
            ));
        }

        let state_gate = self.state_gate(&command.speech_session_id).await;
        let _state_guard = state_gate.lock().await;
        let mut session = self
            .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
            .await?;
        if matches!(
            session.phase,
            SpeechSessionPhase::Closing | SpeechSessionPhase::Failed
        ) {
            return Ok(());
        }
        if session.owner_backend_url != command.owner_backend_url
            || session.asr_owner_instance_id != command.owner_instance_id
            || session.asr_owner_instance_epoch != command.owner_instance_epoch
        {
            return Err(AppError::conflict(
                "speech session owner route failure does not match stored owner",
            ));
        }
        self.ensure_backend_instance_owner(&session)?;
        self.fail_speech_session(&command.speech_session_id, &mut session, &command.reason)
            .await
    }

    async fn submit_server_turn(&self, command: SubmitServerTurnCommand) -> AppResult<()> {
        validate_runtime_owner(&command.runtime_owner_id)?;
        if command.round_id.trim().is_empty() {
            return Err(AppError::invalid_input("round_id is required"));
        }
        if command.reply_text.trim().is_empty() {
            return Err(AppError::invalid_input("reply_text is required"));
        }

        let (session_id, voice) = {
            let state_gate = self.state_gate(&command.speech_session_id).await;
            let _state_guard = state_gate.lock().await;
            let mut session = self
                .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
                .await?;
            if self
                .fail_orphaned_active_turn_on_local_route(&command.speech_session_id, &mut session)
                .await?
            {
                return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
            }
            self.ensure_backend_instance_owner(&session)?;
            self.ensure_local_asr_owner(&session)?;
            require_active_session(&session)?;
            if session
                .dispatched_round_ids
                .iter()
                .any(|round_id| round_id == &command.round_id)
            {
                return Ok(());
            }
            (session.session_id, session.voice.clone())
        };

        let speech_session_id = command.speech_session_id.clone();
        let reply_text = command.reply_text.clone();
        let interruptible = command.interruptible;
        self.start_server_turn(
            speech_session_id,
            session_id,
            voice,
            command.round_id,
            reply_text,
            interruptible,
        )
        .await?;

        Ok(())
    }

    async fn submit_pre_recorded_turn(
        &self,
        command: SubmitPreRecordedTurnCommand,
    ) -> AppResult<()> {
        validate_runtime_owner(&command.runtime_owner_id)?;
        if command.round_id.trim().is_empty() {
            return Err(AppError::invalid_input("round_id is required"));
        }
        if command.audio_url.trim().is_empty() {
            return Err(AppError::invalid_input("audio_url is required"));
        }
        if command.band.trim().is_empty() {
            return Err(AppError::invalid_input("band is required"));
        }

        let state_gate = self.state_gate(&command.speech_session_id).await;
        let _state_guard = state_gate.lock().await;
        let mut session = self
            .load_owned_session(&command.speech_session_id, &command.runtime_owner_id)
            .await?;
        if self
            .fail_orphaned_active_turn_on_local_route(&command.speech_session_id, &mut session)
            .await?
        {
            return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
        }
        self.ensure_backend_instance_owner(&session)?;
        self.ensure_local_asr_owner(&session)?;
        require_active_session(&session)?;
        let claimed = session.claim_output_turn(&command.round_id)?;
        let events = self
            .process_pre_recorded_turn(
                session.session_id,
                &command.round_id,
                &command.audio_url,
                &command.band,
                command.interruptible,
            )
            .await?;
        let mut all_events = Vec::with_capacity(events.len() + usize::from(claimed));
        if claimed {
            all_events.push(SpeechRuntimeEvent::RespondingStarted);
        }
        all_events.extend(events);
        self.session_store
            .save_session_and_push_events(&command.speech_session_id, &session, &all_events)
            .await?;
        Ok(())
    }
}

#[derive(Debug)]
struct SpeechInputTransition {
    events: Vec<SpeechRuntimeEvent>,
    commit_request: Option<PendingRoundCommitRequest>,
}

#[derive(Debug)]
struct PendingTranscriptTurn {
    speech_session_id: String,
    session_id: i64,
    voice: String,
    agent_session_id: String,
    round_id: String,
    transcript: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnStartState {
    Pending,
    Started,
    Failed(TurnStartError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputProgressMode {
    Active,
    ClosingDrain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnStartError {
    code: AppErrorCode,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AsrSessionOwner {
    asr_session_id: String,
    instance_id: String,
    instance_epoch: String,
}

impl SpeechInputTransition {
    fn noop() -> Self {
        Self {
            events: Vec::new(),
            commit_request: None,
        }
    }
}

impl<C, G, S, E> SpeechRuntimeService<C, G, S, E>
where
    C: SpeechRuntimeContextPort + Send + Sync,
    G: SpeechSegmentationConfigPort + Send + Sync,
    S: SpeechSessionStorePort + Clone + Send + Sync + 'static,
    E: SpeechRuntimeEventPort + Clone + Send + Sync + 'static,
{
    async fn load_owned_session(
        &self,
        speech_session_id: &str,
        runtime_owner_id: &str,
    ) -> AppResult<StoredSpeechSession> {
        let session = self
            .session_store
            .get_session(speech_session_id)
            .await?
            .ok_or_else(|| AppError::not_found("speech session not found"))?;
        if session.runtime_owner_id != runtime_owner_id {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }
        Ok(session)
    }

    async fn load_existing_session_for_create(
        &self,
        speech_session_id: &str,
        command: &CreateSpeechSessionCommand,
    ) -> AppResult<Option<CreateSpeechSessionResult>> {
        let Some(mut session) = self.session_store.get_session(speech_session_id).await? else {
            return Ok(None);
        };
        if session.runtime_owner_id != command.runtime_owner_id {
            return Err(AppError::conflict(
                "speech session already exists for a different runtime owner",
            ));
        }
        if session.sample_rate_hz != command.sample_rate_hz
            || session.num_channels != command.num_channels
        {
            return Err(AppError::conflict(
                "speech session already exists with a different audio shape",
            ));
        }
        if matches!(
            session.phase,
            SpeechSessionPhase::Closing | SpeechSessionPhase::Failed
        ) {
            return Err(AppError::conflict(
                "speech session already exists in a terminal phase",
            ));
        }
        if self
            .fail_orphaned_active_turn_on_local_route(speech_session_id, &mut session)
            .await?
        {
            return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
        }
        if !self.has_local_asr_owner(&session) {
            return Ok(Some(CreateSpeechSessionResult {
                speech_session_id: speech_session_id.to_owned(),
                owner_backend_url: session.owner_backend_url,
                owner_instance_id: session.asr_owner_instance_id,
                owner_instance_epoch: session.asr_owner_instance_epoch,
                owner_route_only: true,
            }));
        }
        self.ensure_backend_instance_owner(&session)?;
        self.ensure_local_asr_owner(&session)?;
        Ok(Some(CreateSpeechSessionResult {
            speech_session_id: speech_session_id.to_owned(),
            owner_backend_url: session.owner_backend_url,
            owner_instance_id: session.asr_owner_instance_id,
            owner_instance_epoch: session.asr_owner_instance_epoch,
            owner_route_only: false,
        }))
    }

    async fn require_runtime_context(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
    ) -> AppResult<SpeechRuntimeContext> {
        let session = self
            .runtime_context
            .resolve_accepting_runtime_context(session_id, runtime_owner_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        Ok(session)
    }

    async fn spawn_transcript_turn(
        &self,
        speech_session_id: String,
        session_id: i64,
        voice: String,
        agent_session_id: String,
        round_id: String,
        transcript: String,
    ) {
        let session_store = self.session_store.clone();
        let events = self.events.clone();
        let voice_runtime = self.voice_runtime.clone();
        let agent_turn = self.agent_turn.clone();
        let turn_gate = self.turn_gate(&speech_session_id).await;
        let state_gate = self.state_gate(&speech_session_id).await;
        let task_key = turn_task_key(&speech_session_id, &round_id);
        let task_key_for_task = task_key.clone();
        let task_registry = self.turn_tasks.clone();
        let task_start_signals = self.turn_start_signals.clone();
        let registration_speech_session_id = speech_session_id.clone();
        let (registered_tx, registered_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = registered_rx.await;
            let _turn_guard = turn_gate.lock().await;
            let result = run_transcript_turn(
                session_store.clone(),
                events.clone(),
                voice_runtime.clone(),
                agent_turn,
                speech_session_id.clone(),
                session_id,
                voice,
                agent_session_id,
                round_id.clone(),
                transcript,
                true,
                state_gate.clone(),
            )
            .await;
            if let Err(error) = result {
                if is_stale_turn_error(&error) {
                    let _ = publish_log_event(
                        &events,
                        session_id,
                        Some(round_id.as_str()),
                        "speech_turn_stale_absorbed",
                        json!({ "reason": error.to_string() }),
                    )
                    .await;
                    task_registry.lock().await.remove(&task_key_for_task);
                    task_start_signals.lock().await.remove(&task_key_for_task);
                    return;
                }
                let message = error.to_string();
                if let Err(failure_error) = fail_speech_session_state(FailSpeechSessionState {
                    session_store: &session_store,
                    events: &events,
                    voice_runtime: voice_runtime.as_ref(),
                    speech_session_id: &speech_session_id,
                    session_id,
                    round_id: Some(round_id.as_str()),
                    reason: &message,
                    state_gate: &state_gate,
                })
                .await
                {
                    tracing::error!(
                        speech_session_id,
                        session_id,
                        session_round_id = %round_id,
                        error = %failure_error,
                        "failed to persist speech session failure"
                    );
                }
            }
            task_registry.lock().await.remove(&task_key_for_task);
            task_start_signals.lock().await.remove(&task_key_for_task);
        });
        let _ = self
            .register_turn_task(
                &registration_speech_session_id,
                task_key,
                handle,
                registered_tx,
                None,
            )
            .await;
    }

    async fn start_server_turn(
        &self,
        speech_session_id: String,
        session_id: i64,
        voice: String,
        round_id: String,
        reply_text: String,
        interruptible: bool,
    ) -> AppResult<()> {
        let session_store = self.session_store.clone();
        let events = self.events.clone();
        let voice_runtime = self.voice_runtime.clone();
        let turn_gate = self.turn_gate(&speech_session_id).await;
        let state_gate = self.state_gate(&speech_session_id).await;
        let task_key = turn_task_key(&speech_session_id, &round_id);
        let task_key_for_task = task_key.clone();
        let task_registry = self.turn_tasks.clone();
        let task_start_signals = self.turn_start_signals.clone();
        let registration_speech_session_id = speech_session_id.clone();
        let (registered_tx, registered_rx) = oneshot::channel();
        let (started_tx, started_rx) = watch::channel(TurnStartState::Pending);
        let handle = tokio::spawn(async move {
            let _ = registered_rx.await;
            let _turn_guard = turn_gate.lock().await;
            let result = run_tts_reply_stream(
                session_store.clone(),
                events.clone(),
                voice_runtime.clone(),
                speech_session_id.clone(),
                session_id,
                voice,
                round_id.clone(),
                reply_text,
                interruptible,
                true,
                Some(started_tx),
                state_gate.clone(),
            )
            .await;
            if let Err(error) = result {
                if is_stale_turn_error(&error) {
                    let _ = publish_log_event(
                        &events,
                        session_id,
                        Some(round_id.as_str()),
                        "speech_turn_stale_absorbed",
                        json!({ "reason": error.to_string() }),
                    )
                    .await;
                    task_registry.lock().await.remove(&task_key_for_task);
                    task_start_signals.lock().await.remove(&task_key_for_task);
                    return;
                }
                let message = error.to_string();
                if let Err(failure_error) = fail_speech_session_state(FailSpeechSessionState {
                    session_store: &session_store,
                    events: &events,
                    voice_runtime: voice_runtime.as_ref(),
                    speech_session_id: &speech_session_id,
                    session_id,
                    round_id: Some(round_id.as_str()),
                    reason: &message,
                    state_gate: &state_gate,
                })
                .await
                {
                    tracing::error!(
                        speech_session_id,
                        session_id,
                        session_round_id = %round_id,
                        error = %failure_error,
                        "failed to persist speech session failure"
                    );
                }
            }
            task_registry.lock().await.remove(&task_key_for_task);
            task_start_signals.lock().await.remove(&task_key_for_task);
        });
        let started_rx = self
            .register_turn_task(
                &registration_speech_session_id,
                task_key,
                handle,
                registered_tx,
                Some(started_rx),
            )
            .await;
        wait_for_turn_start(started_rx).await
    }

    async fn turn_gate(&self, speech_session_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.turn_gates.lock().await;
        gates
            .entry(speech_session_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn state_gate(&self, speech_session_id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.state_gates.lock().await;
        gates
            .entry(speech_session_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn register_turn_task(
        &self,
        speech_session_id: &str,
        task_key: String,
        handle: JoinHandle<()>,
        registered_tx: oneshot::Sender<()>,
        start_signal: Option<watch::Receiver<TurnStartState>>,
    ) -> watch::Receiver<TurnStartState> {
        if self.is_session_closing(speech_session_id).await {
            handle.abort();
            return failed_turn_start_receiver(AppError::conflict(TERMINAL_SESSION_MESSAGE));
        }
        let mut registry = self.turn_tasks.lock().await;
        let mut start_signals = self.turn_start_signals.lock().await;
        if registry.contains_key(&task_key) {
            handle.abort();
            return start_signals.get(&task_key).cloned().unwrap_or_else(|| {
                failed_turn_start_receiver(AppError::internal(
                    "speech turn task start signal is missing",
                ))
            });
        }
        let started_rx = start_signal.unwrap_or_else(completed_turn_start_receiver);
        start_signals.insert(task_key.clone(), started_rx.clone());
        registry.insert(task_key.clone(), handle);
        drop(start_signals);
        drop(registry);
        let _ = registered_tx.send(());
        started_rx
    }

    async fn pop_session_events(
        &self,
        command: &PollSpeechEventsCommand,
    ) -> AppResult<PollSpeechEventsResult> {
        let max_events = command.max_events.clamp(1, DEFAULT_MAX_EVENTS);
        let events = self
            .session_store
            .pop_events(&command.speech_session_id, max_events)
            .await?;
        Ok(PollSpeechEventsResult { events })
    }

    async fn save_input_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
        progress_mode: InputProgressMode,
    ) -> AppResult<SpeechSessionInputProgressSaveResult> {
        let result = match progress_mode {
            InputProgressMode::Active => {
                self.session_store
                    .save_input_progress(speech_session_id, session)
                    .await
            }
            InputProgressMode::ClosingDrain => {
                self.session_store
                    .save_closing_drain_progress(speech_session_id, session)
                    .await
            }
        }?;
        match result {
            SpeechSessionInputProgressSaveResult::Saved => Ok(result),
            SpeechSessionInputProgressSaveResult::Dropped => Ok(result),
            SpeechSessionInputProgressSaveResult::Terminal => {
                Err(AppError::conflict(TERMINAL_SESSION_MESSAGE))
            }
        }
    }

    async fn save_asr_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
        progress_mode: InputProgressMode,
    ) -> AppResult<()> {
        let result = match progress_mode {
            InputProgressMode::Active => {
                self.session_store
                    .save_asr_progress(speech_session_id, session)
                    .await
            }
            InputProgressMode::ClosingDrain => {
                self.session_store
                    .save_closing_drain_progress(speech_session_id, session)
                    .await
            }
        }?;
        match result {
            SpeechSessionInputProgressSaveResult::Saved => Ok(()),
            SpeechSessionInputProgressSaveResult::Dropped => Err(AppError::internal(
                "ASR progress cannot be dropped after provider events were consumed",
            )),
            SpeechSessionInputProgressSaveResult::Terminal => {
                Err(AppError::conflict(TERMINAL_SESSION_MESSAGE))
            }
        }
    }

    async fn fail_orphaned_active_turn(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
    ) -> AppResult<bool> {
        let Some(active_turn) = session.active_turn.as_ref() else {
            if matches!(session.phase, SpeechSessionPhase::Responding) {
                let reason =
                    "speech responding turn was orphaned because active turn metadata is missing";
                self.fail_speech_session(speech_session_id, session, reason)
                    .await?;
                return Ok(true);
            }
            return Ok(false);
        };
        if self
            .has_turn_task(&turn_task_key(speech_session_id, &active_turn.round_id))
            .await
        {
            return Ok(false);
        }
        let reason = format!(
            "speech turn {} was orphaned because owner backend task is missing",
            active_turn.round_id
        );
        self.fail_speech_session(speech_session_id, session, &reason)
            .await?;
        Ok(true)
    }

    async fn fail_orphaned_active_turn_on_local_route(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
    ) -> AppResult<bool> {
        if session.asr_owner_instance_id != self.instance_id {
            return Ok(false);
        }
        if session.asr_owner_instance_epoch != self.instance_epoch {
            let reason = format!(
                "speech session ASR owner epoch {} is stale for backend instance {} epoch {}",
                session.asr_owner_instance_epoch, self.instance_id, self.instance_epoch
            );
            self.fail_speech_session(speech_session_id, session, &reason)
                .await?;
            return Ok(true);
        }
        self.fail_orphaned_active_turn(speech_session_id, session)
            .await
    }

    async fn has_turn_task(&self, task_key: &str) -> bool {
        self.turn_tasks.lock().await.contains_key(task_key)
    }

    async fn mark_session_closing(&self, speech_session_id: &str) {
        self.closing_sessions
            .lock()
            .await
            .insert(speech_session_id.to_owned());
    }

    async fn clear_session_closing(&self, speech_session_id: &str) {
        self.closing_sessions.lock().await.remove(speech_session_id);
    }

    async fn remove_turn_gate(&self, speech_session_id: &str) {
        self.turn_gates.lock().await.remove(speech_session_id);
    }

    async fn remove_state_gate(&self, speech_session_id: &str) {
        self.state_gates.lock().await.remove(speech_session_id);
    }

    async fn is_session_closing(&self, speech_session_id: &str) -> bool {
        self.closing_sessions
            .lock()
            .await
            .contains(speech_session_id)
    }

    fn spawn_close_asr(&self, asr_session_id: String) {
        let voice_runtime = self.voice_runtime.clone();
        tokio::spawn(async move {
            let _ = voice_runtime
                .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                    asr_session_id,
                })
                .await;
        });
    }

    async fn abort_turn_tasks(&self, speech_session_id: &str) -> AppResult<()> {
        let prefix = format!("{speech_session_id}:");
        let tasks = {
            let mut registry = self.turn_tasks.lock().await;
            let mut start_signals = self.turn_start_signals.lock().await;
            let keys = registry
                .keys()
                .filter(|key| key.starts_with(&prefix))
                .cloned()
                .collect::<Vec<_>>();
            for key in &keys {
                start_signals.remove(key);
            }
            keys.into_iter()
                .filter_map(|key| registry.remove(&key))
                .collect::<Vec<_>>()
        };
        for task in tasks {
            task.abort();
            match task.await {
                Ok(()) => {}
                Err(error) if error.is_cancelled() => {}
                Err(error) => {
                    return Err(AppError::internal(format!(
                        "speech turn task failed while aborting: {error}"
                    )));
                }
            }
        }
        Ok(())
    }

    async fn commit_deferred_flush_if_ready(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        progress_mode: InputProgressMode,
        state_gate: &Arc<Mutex<()>>,
    ) -> AppResult<()> {
        self.commit_flush_if_ready(
            speech_session_id,
            session,
            true,
            progress_mode,
            state_gate,
            StoredSpeechSession::take_deferred_flush_transition,
        )
        .await
        .map(|_| ())
    }

    async fn commit_final_flush_if_ready(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        progress_mode: InputProgressMode,
        state_gate: &Arc<Mutex<()>>,
    ) -> AppResult<bool> {
        self.commit_flush_if_ready(
            speech_session_id,
            session,
            false,
            progress_mode,
            state_gate,
            StoredSpeechSession::take_final_flush_transition,
        )
        .await
    }

    async fn commit_flush_if_ready(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        resume_listening_after_failure: bool,
        progress_mode: InputProgressMode,
        state_gate: &Arc<Mutex<()>>,
        take_transition: fn(&mut StoredSpeechSession) -> SpeechInputTransition,
    ) -> AppResult<bool> {
        let commit_request = {
            let _state_guard = state_gate.lock().await;
            let latest_session = self
                .session_store
                .get_session(speech_session_id)
                .await?
                .ok_or_else(|| AppError::not_found("speech session not found"))?;
            *session = latest_session;
            let mut proposed_session = session.clone();
            let transition = take_transition(&mut proposed_session);
            let Some(commit_request) = transition.commit_request.as_ref() else {
                return Ok(false);
            };
            let round_id = commit_request.round_id.as_str();
            proposed_session.push_pending_round(commit_request.clone());
            let save_result = self
                .save_input_progress(speech_session_id, &proposed_session, progress_mode)
                .await?;
            if matches!(save_result, SpeechSessionInputProgressSaveResult::Dropped) {
                tracing::debug!(
                    session_id = session.session_id,
                    speech_session_id,
                    round_id,
                    proposed_phase = ?proposed_session.phase,
                    active_round_id = ?session.active_turn.as_ref().map(|turn| turn.round_id.as_str()),
                    "speech runtime dropped flush transition after state compare"
                );
                return Ok(false);
            }
            *session = proposed_session;
            if !transition.events.is_empty() {
                self.session_store
                    .push_events(speech_session_id, &transition.events)
                    .await?;
                self.publish_transition_events(
                    session.session_id,
                    Some(round_id),
                    &transition.events,
                )
                .await?;
            }

            self.publish_log_event(
                session.session_id,
                Some(round_id),
                "speech_asr_commit_requested",
                json!({
                    "speech_session_id": speech_session_id,
                    "asr_session_id": session.asr_session_id,
                }),
            )
            .await?;
            (session.asr_session_id.clone(), round_id.to_owned())
        };
        let (asr_session_id, round_id) = commit_request;
        if let Err(error) = self
            .voice_runtime
            .commit_asr_session_for_runtime(voice::CommitRuntimeAsrSessionRequest {
                asr_session_id,
                round_id: round_id.clone(),
            })
            .await
        {
            let _state_guard = state_gate.lock().await;
            let latest_session = self
                .session_store
                .get_session(speech_session_id)
                .await?
                .ok_or_else(|| AppError::not_found("speech session not found"))?;
            *session = latest_session;
            self.record_asr_round_failed(
                speech_session_id,
                session,
                &round_id,
                &error.to_string(),
                resume_listening_after_failure,
                progress_mode,
            )
            .await?;
        }
        Ok(true)
    }

    async fn record_asr_round_failed(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        round_id: &str,
        reason: &str,
        resume_listening: bool,
        progress_mode: InputProgressMode,
    ) -> AppResult<()> {
        session.remove_pending_round_id(round_id)?;
        if resume_listening && !session.deferred_flush {
            session.phase = SpeechSessionPhase::Listening;
        }
        self.save_asr_progress(speech_session_id, session, progress_mode)
            .await?;
        self.session_store
            .push_events(
                speech_session_id,
                &[SpeechRuntimeEvent::RoundFailed {
                    round_id: round_id.to_owned(),
                    message: reason.to_owned(),
                }],
            )
            .await?;
        self.publish_log_event(
            session.session_id,
            Some(round_id),
            "speech_asr_round_failed",
            json!({ "reason": reason }),
        )
        .await
    }

    async fn drain_asr_runtime_for_close(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        state_gate: &Arc<Mutex<()>>,
    ) -> AppResult<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let committed_final = self
                .commit_final_flush_if_ready(
                    speech_session_id,
                    session,
                    InputProgressMode::ClosingDrain,
                    state_gate,
                )
                .await?;
            self.drive_asr_runtime(
                speech_session_id,
                session,
                false,
                false,
                InputProgressMode::ClosingDrain,
                state_gate,
            )
            .await?;
            self.commit_deferred_flush_if_ready(
                speech_session_id,
                session,
                InputProgressMode::ClosingDrain,
                state_gate,
            )
            .await?;
            self.drive_asr_runtime(
                speech_session_id,
                session,
                false,
                false,
                InputProgressMode::ClosingDrain,
                state_gate,
            )
            .await?;

            if session.pending_rounds.is_empty() && !committed_final {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::unavailable(
                    "timed out draining ASR runtime before closing speech session",
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn drive_asr_runtime(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        resume_listening_after_final: bool,
        spawn_transcript_turns: bool,
        progress_mode: InputProgressMode,
        state_gate: &Arc<Mutex<()>>,
    ) -> AppResult<()> {
        let asr_session_id = session.asr_session_id.clone();
        let result = self
            .voice_runtime
            .poll_asr_events_for_runtime(voice::PollRuntimeAsrEventsRequest {
                asr_session_id,
                max_events: DEFAULT_MAX_EVENTS,
            })
            .await?;
        let had_events = !result.events.is_empty();

        let _state_guard = state_gate.lock().await;
        let latest_session = self
            .session_store
            .get_session(speech_session_id)
            .await?
            .ok_or_else(|| AppError::not_found("speech session not found"))?;
        *session = latest_session;
        let mut worker_events = Vec::new();
        let mut transcript_turns = Vec::new();
        let mut session_failure = None;
        let mut state_dirty = false;
        let session_id = session.session_id;

        let mut forced_turn = None;
        for event in result.events {
            match event {
                voice::RuntimeAsrEvent::PartialTranscript {
                    round_id,
                    transcript,
                } => {
                    if let Some(pending_round) = session.current_pending_round_mut()
                        && pending_round.round_id == round_id
                    {
                        pending_round.latest_partial_transcript = transcript.clone();
                        state_dirty = true;
                    }
                    self.publish_log_event(
                        session_id,
                        Some(round_id.as_str()),
                        "speech_asr_partial_received",
                        json!({
                            "transcript": transcript,
                        }),
                    )
                    .await?;
                }
                voice::RuntimeAsrEvent::FinalTranscript {
                    round_id,
                    transcript,
                } => {
                    match session.remove_pending_round_id(&round_id) {
                        Ok(()) => {
                            state_dirty = true;
                        }
                        Err(_error)
                            if session
                                .dispatched_round_ids
                                .iter()
                                .any(|dispatched| dispatched == &round_id) =>
                        {
                            self.publish_log_event(
                                session_id,
                                Some(round_id.as_str()),
                                "speech_asr_late_final_ignored",
                                json!({ "transcript": transcript }),
                            )
                            .await?;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                    if spawn_transcript_turns {
                        transcript_turns.push(PendingTranscriptTurn {
                            speech_session_id: speech_session_id.to_owned(),
                            session_id: session.session_id,
                            voice: session.voice.clone(),
                            agent_session_id: session.agent_session_id.clone(),
                            round_id: round_id.clone(),
                            transcript: transcript.clone(),
                        });
                    }
                    if resume_listening_after_final && !session.deferred_flush {
                        session.phase = SpeechSessionPhase::Listening;
                    }
                    self.publish_log_event(
                        session_id,
                        Some(round_id.as_str()),
                        "speech_asr_final_received",
                        json!({
                            "transcript": transcript,
                        }),
                    )
                    .await?;
                }
                voice::RuntimeAsrEvent::Warning { message } => {
                    worker_events.push(SpeechRuntimeEvent::Warning {
                        message: message.clone(),
                    });
                    self.publish_log_event(
                        session_id,
                        session.current_pending_round_id(),
                        "speech_asr_warning",
                        json!({ "message": message }),
                    )
                    .await?;
                }
                voice::RuntimeAsrEvent::RoundFailed { round_id, message } => {
                    match session.remove_pending_round_id(&round_id) {
                        Ok(()) => {
                            state_dirty = true;
                        }
                        Err(_error)
                            if session
                                .dispatched_round_ids
                                .iter()
                                .any(|dispatched| dispatched == &round_id) =>
                        {
                            self.publish_log_event(
                                session_id,
                                Some(round_id.as_str()),
                                "speech_asr_late_round_failed_ignored",
                                json!({ "reason": message }),
                            )
                            .await?;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                    if resume_listening_after_final && !session.deferred_flush {
                        session.phase = SpeechSessionPhase::Listening;
                    }
                    worker_events.push(SpeechRuntimeEvent::RoundFailed {
                        round_id: round_id.clone(),
                        message: message.clone(),
                    });
                    self.publish_log_event(
                        session_id,
                        Some(round_id.as_str()),
                        "speech_asr_round_failed",
                        json!({ "reason": message }),
                    )
                    .await?;
                }
                voice::RuntimeAsrEvent::SessionFailed { message } => {
                    self.publish_log_event(
                        session_id,
                        session.current_pending_round_id(),
                        "speech_asr_failed",
                        json!({ "reason": message }),
                    )
                    .await?;
                    session_failure = Some(message);
                    break;
                }
                voice::RuntimeAsrEvent::Closed => {
                    self.publish_log_event(
                        session_id,
                        session.current_pending_round_id(),
                        "speech_asr_session_closed",
                        json!({ "asr_session_id": session.asr_session_id }),
                    )
                    .await?;
                }
            }
        }

        if let Some(turn) = session.take_forced_transcript_turn_if_overdue(now_millis()) {
            state_dirty = true;
            if spawn_transcript_turns {
                transcript_turns.push(PendingTranscriptTurn {
                    speech_session_id: speech_session_id.to_owned(),
                    session_id: session.session_id,
                    voice: session.voice.clone(),
                    agent_session_id: session.agent_session_id.clone(),
                    round_id: turn.round_id.clone(),
                    transcript: turn.transcript.clone(),
                });
            }
            if resume_listening_after_final && !session.deferred_flush {
                session.phase = SpeechSessionPhase::Listening;
            }
            self.publish_log_event(
                session_id,
                Some(turn.round_id.as_str()),
                "speech_asr_forced_finalized",
                json!({
                    "transcript": turn.transcript,
                }),
            )
            .await?;
            forced_turn = Some(turn);
        }

        if had_events || state_dirty || session_failure.is_some() {
            self.save_asr_progress(speech_session_id, session, progress_mode)
                .await?;
        }
        if forced_turn.is_some() {
            self.rotate_asr_session_after_forced_finalize(
                speech_session_id,
                session,
                progress_mode,
            )
            .await?;
        }
        if !worker_events.is_empty() {
            self.session_store
                .push_events(speech_session_id, &worker_events)
                .await?;
        }
        for turn in transcript_turns {
            self.spawn_transcript_turn(
                turn.speech_session_id,
                turn.session_id,
                turn.voice,
                turn.agent_session_id,
                turn.round_id,
                turn.transcript,
            )
            .await;
        }
        if let Some(message) = session_failure {
            return Err(AppError::unavailable(message));
        }
        Ok(())
    }

    async fn fail_speech_session(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        reason: &str,
    ) -> AppResult<()> {
        session.phase = SpeechSessionPhase::Failed;
        session.active_turn = None;
        self.session_store
            .save_session(speech_session_id, session)
            .await?;
        self.session_store
            .push_events(
                speech_session_id,
                &[SpeechRuntimeEvent::SessionFailed {
                    message: reason.to_owned(),
                }],
            )
            .await?;
        self.publish_log_event(
            session.session_id,
            session.current_pending_round_id(),
            "speech_session_failed",
            json!({ "reason": reason }),
        )
        .await?;
        self.abort_turn_tasks(speech_session_id).await?;
        self.remove_turn_gate(speech_session_id).await;
        if self.has_local_asr_owner(session) {
            self.voice_runtime
                .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                    asr_session_id: session.asr_session_id.clone(),
                })
                .await?;
        }
        self.remove_state_gate(speech_session_id).await;
        Ok(())
    }

    async fn rollback_created_session(
        &self,
        speech_session_id: &str,
        asr_session_id: &str,
    ) -> AppResult<()> {
        let _ = self.session_store.take_session(speech_session_id).await?;
        let _ = self.session_store.drain_events(speech_session_id).await?;
        self.remove_state_gate(speech_session_id).await;
        let _ = self
            .voice_runtime
            .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                asr_session_id: asr_session_id.to_owned(),
            })
            .await;
        Ok(())
    }

    async fn rotate_asr_session_after_forced_finalize(
        &self,
        speech_session_id: &str,
        session: &mut StoredSpeechSession,
        progress_mode: InputProgressMode,
    ) -> AppResult<()> {
        let old_asr_session_id = session.asr_session_id.clone();
        let opened = self
            .voice_runtime
            .open_asr_session_for_runtime(voice::OpenRuntimeAsrSessionRequest {
                speech_session_id: speech_session_id.to_owned(),
                sample_rate_hz: session.sample_rate_hz,
                num_channels: session.num_channels,
                language: session.language,
            })
            .await?;
        session.asr_session_id = opened.asr_session_id.clone();
        session.asr_owner_instance_id = self.instance_id.clone();
        session.asr_owner_instance_epoch = self.instance_epoch.clone();
        self.save_asr_progress(speech_session_id, session, progress_mode)
            .await?;
        self.publish_log_event(
            session.session_id,
            None,
            "speech_asr_session_rotated",
            json!({
                "previous_asr_session_id": old_asr_session_id,
                "asr_session_id": session.asr_session_id,
                "reason": "forced_finalized_round",
            }),
        )
        .await?;
        let voice_runtime = self.voice_runtime.clone();
        let events = self.events.clone();
        let session_id = session.session_id;
        let speech_session_id = speech_session_id.to_owned();
        tokio::spawn(async move {
            if let Err(error) = voice_runtime
                .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest {
                    asr_session_id: old_asr_session_id.clone(),
                })
                .await
            {
                tracing::warn!(
                    session_id,
                    speech_session_id,
                    old_asr_session_id,
                    error = %error,
                    "speech runtime failed to close previous ASR session after forced finalization"
                );
                let _ = publish_log_event(
                    &events,
                    session_id,
                    None,
                    "speech_asr_session_rotate_close_failed",
                    json!({
                        "previous_asr_session_id": old_asr_session_id,
                        "reason": error.to_string(),
                    }),
                )
                .await;
            }
        });
        Ok(())
    }

    fn ensure_local_asr_owner(&self, session: &StoredSpeechSession) -> AppResult<()> {
        self.ensure_backend_instance_owner(session)?;
        if session.asr_owner_instance_id == self.instance_id {
            if session.asr_owner_instance_epoch == self.instance_epoch {
                return Ok(());
            }
            return Err(AppError::conflict(format!(
                "speech session ASR owner epoch {} is stale for backend instance {} epoch {}",
                session.asr_owner_instance_epoch, self.instance_id, self.instance_epoch
            )));
        }
        Err(AppError::conflict(format!(
            "speech session ASR connection is owned by backend instance {}, not {}",
            session.asr_owner_instance_id, self.instance_id
        )))
    }

    fn ensure_backend_instance_owner(&self, session: &StoredSpeechSession) -> AppResult<()> {
        if session.asr_owner_instance_id == self.instance_id
            && session.asr_owner_instance_epoch == self.instance_epoch
        {
            return Ok(());
        }
        Err(AppError::conflict(format!(
            "speech session ASR connection is owned by backend instance {} epoch {}, not {} epoch {}",
            session.asr_owner_instance_id,
            session.asr_owner_instance_epoch,
            self.instance_id,
            self.instance_epoch
        )))
    }

    fn has_local_asr_owner(&self, session: &StoredSpeechSession) -> bool {
        session.asr_owner_instance_id == self.instance_id
            && session.asr_owner_instance_epoch == self.instance_epoch
    }

    async fn process_pre_recorded_turn(
        &self,
        session_id: i64,
        round_id: &str,
        audio_url: &str,
        band: &str,
        interruptible: bool,
    ) -> AppResult<Vec<SpeechRuntimeEvent>> {
        self.publish_log_event(
            session_id,
            Some(round_id),
            "pre_recorded_turn_started",
            json!({
                "audio_url": audio_url,
                "band": band,
                "interruptible": interruptible,
            }),
        )
        .await?;

        let events = vec![SpeechRuntimeEvent::PreRecordedAsset {
            round_id: round_id.to_owned(),
            audio_url: audio_url.to_owned(),
            band: band.to_owned(),
            interruptible,
        }];

        self.publish_log_event(
            session_id,
            Some(round_id),
            "pre_recorded_asset_dispatched",
            json!({
                "audio_url": audio_url,
                "band": band,
                "interruptible": interruptible,
            }),
        )
        .await?;

        Ok(events)
    }

    async fn publish_transition_events(
        &self,
        session_id: i64,
        round_id: Option<&str>,
        events: &[SpeechRuntimeEvent],
    ) -> AppResult<()> {
        for event in events {
            match event {
                SpeechRuntimeEvent::SpeechDetected => {
                    self.publish_log_event(session_id, round_id, "speech_detected", json!({}))
                        .await?;
                }
                SpeechRuntimeEvent::UtteranceFlushing => {
                    self.publish_log_event(
                        session_id,
                        round_id,
                        "speech_utterance_flushing",
                        json!({}),
                    )
                    .await?;
                }
                SpeechRuntimeEvent::SessionFailed { message } => {
                    self.publish_log_event(
                        session_id,
                        round_id,
                        "speech_session_failed",
                        json!({ "reason": message }),
                    )
                    .await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn publish_log_event(
        &self,
        session_id: i64,
        round_id: Option<&str>,
        event: &str,
        fields: serde_json::Value,
    ) -> AppResult<()> {
        self.events
            .publish(SpeechLogEvent {
                session_id,
                round_id: round_id.map(ToOwned::to_owned),
                event: event.to_owned(),
                fields,
            })
            .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_transcript_turn<S, E>(
    session_store: S,
    events: E,
    voice_runtime: Arc<dyn voice::VoiceRuntimeExecutionPort>,
    agent_turn: Arc<dyn AgentTurnPort>,
    speech_session_id: String,
    session_id: i64,
    voice: String,
    agent_session_id: String,
    round_id: String,
    transcript: String,
    interruptible: bool,
    state_gate: Arc<Mutex<()>>,
) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
    E: SpeechRuntimeEventPort + Send + Sync,
{
    let transcript = transcript.trim().to_owned();
    if transcript.is_empty() {
        return Err(AppError::internal(
            "speech turn failed because ASR returned empty transcript",
        ));
    }
    {
        let _state_guard = state_gate.lock().await;
        let session = session_store
            .get_session(&speech_session_id)
            .await?
            .ok_or_else(|| AppError::not_found("speech session not found"))?;
        require_active_session(&session)?;
    }
    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_asr_completed",
        json!({
            "empty": false,
            "transcript": transcript,
        }),
    )
    .await?;
    claim_output_turn_if_needed(&session_store, &speech_session_id, &round_id, &state_gate).await?;
    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_agent_started",
        json!({}),
    )
    .await?;

    let reply_text = agent_turn
        .chat_once(AgentTurnRequest {
            session_id: agent_session_id,
            user_message: transcript,
        })
        .await?
        .reply_text;
    if reply_text.trim().is_empty() {
        publish_log_event(
            &events,
            session_id,
            Some(round_id.as_str()),
            "speech_agent_completed",
            json!({ "empty": true }),
        )
        .await?;
        return Err(AppError::internal(
            "speech turn failed because agent returned empty reply",
        ));
    }
    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_agent_completed",
        json!({
            "empty": false,
            "reply_text": reply_text,
        }),
    )
    .await?;

    run_tts_reply_stream(
        session_store,
        events,
        voice_runtime,
        speech_session_id,
        session_id,
        voice,
        round_id,
        reply_text,
        interruptible,
        false,
        None,
        state_gate,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_tts_reply_stream<S, E>(
    session_store: S,
    events: E,
    voice_runtime: Arc<dyn voice::VoiceRuntimeExecutionPort>,
    speech_session_id: String,
    session_id: i64,
    voice: String,
    round_id: String,
    reply_text: String,
    interruptible: bool,
    log_server_turn: bool,
    start_ack: Option<watch::Sender<TurnStartState>>,
    state_gate: Arc<Mutex<()>>,
) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
    E: SpeechRuntimeEventPort + Send + Sync,
{
    claim_output_turn_if_needed(&session_store, &speech_session_id, &round_id, &state_gate).await?;
    if log_server_turn {
        publish_log_event(
            &events,
            session_id,
            Some(round_id.as_str()),
            "server_turn_started",
            json!({}),
        )
        .await?;
    }
    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_tts_started",
        json!({}),
    )
    .await?;
    tracing::debug!(reply_text, "speech-runtime starting streaming TTS stage");

    let mut start_ack = start_ack;
    if let Err(error) = session_store
        .push_events(
            &speech_session_id,
            &[SpeechRuntimeEvent::ReplyStarted {
                round_id: round_id.clone(),
                reply_text: reply_text.clone(),
                interruptible,
            }],
        )
        .await
    {
        if let Some(start_ack) = start_ack.take() {
            let _ = start_ack.send(TurnStartState::Failed(TurnStartError::from_app_error(
                &error,
            )));
        }
        return Err(error);
    }
    let stream_result = voice_runtime
        .stream_tts_for_runtime(voice::RuntimeTtsExecutionRequest {
            text: reply_text.clone(),
            voice,
        })
        .await;
    let mut stream = match stream_result {
        Ok(stream) => stream.chunks,
        Err(error) => {
            if let Some(start_ack) = start_ack.take() {
                let _ = start_ack.send(TurnStartState::Failed(TurnStartError::from_app_error(
                    &error,
                )));
            }
            return Err(error);
        }
    };
    if let Err(error) = publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_reply_started",
        json!({ "reply_text": reply_text }),
    )
    .await
    {
        if let Some(start_ack) = start_ack.take() {
            let _ = start_ack.send(TurnStartState::Failed(TurnStartError::from_app_error(
                &error,
            )));
        }
        return Err(error);
    }
    if let Some(start_ack) = start_ack.take() {
        let _ = start_ack.send(TurnStartState::Started);
    }

    let mut chunk_count: u64 = 0;
    let mut sample_count: u64 = 0;
    let mut sample_rate_hz: Option<u32> = None;
    let mut channels: Option<u16> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.pcm_s16le.is_empty() {
            continue;
        }
        ensure_reply_stream_active(&session_store, &speech_session_id, &round_id, &state_gate)
            .await?;
        if chunk_count == 0 {
            publish_log_event(
                &events,
                session_id,
                Some(round_id.as_str()),
                "speech_tts_first_chunk_received",
                json!({
                    "sample_rate_hz": chunk.sample_rate_hz,
                    "channels": chunk.channels,
                    "sample_count": chunk.pcm_s16le.len(),
                }),
            )
            .await?;
        }
        sample_rate_hz = Some(chunk.sample_rate_hz);
        channels = Some(chunk.channels);
        sample_count += chunk.pcm_s16le.len() as u64;
        chunk_count += 1;
        session_store
            .push_events(
                &speech_session_id,
                &[SpeechRuntimeEvent::AudioChunk {
                    pcm_s16le: chunk.pcm_s16le,
                    sample_rate_hz: chunk.sample_rate_hz,
                    channels: chunk.channels,
                }],
            )
            .await?;
    }

    if chunk_count == 0 {
        publish_log_event(
            &events,
            session_id,
            Some(round_id.as_str()),
            "speech_tts_completed",
            json!({ "empty": true }),
        )
        .await?;
        return Err(AppError::internal(
            "speech turn failed because TTS returned empty audio",
        ));
    }

    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_tts_completed",
        json!({
            "empty": false,
            "chunk_count": chunk_count,
            "sample_count": sample_count,
            "sample_rate_hz": sample_rate_hz,
            "channels": channels,
        }),
    )
    .await?;
    finish_reply_stream(&session_store, &speech_session_id, &round_id, &state_gate).await?;
    publish_log_event(
        &events,
        session_id,
        Some(round_id.as_str()),
        "speech_reply_finished",
        json!({}),
    )
    .await?;
    if log_server_turn {
        publish_log_event(
            &events,
            session_id,
            Some(round_id.as_str()),
            "server_turn_finished",
            json!({}),
        )
        .await?;
    }
    Ok(())
}

async fn finish_reply_stream<S>(
    session_store: &S,
    speech_session_id: &str,
    round_id: &str,
    state_gate: &Arc<Mutex<()>>,
) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
{
    let _state_guard = state_gate.lock().await;
    let Some(session) = session_store.get_session(speech_session_id).await? else {
        return Err(AppError::not_found("speech session not found"));
    };
    let mut session = session;
    require_active_reply_turn(&session, round_id)?;
    let mut final_events = vec![SpeechRuntimeEvent::ReplyFinished {
        round_id: round_id.to_owned(),
    }];
    session.active_turn = None;
    session.phase = SpeechSessionPhase::Listening;
    final_events.push(SpeechRuntimeEvent::ListeningStarted);
    session_store
        .save_session_and_push_events(speech_session_id, &session, &final_events)
        .await
}

async fn claim_output_turn_if_needed<S>(
    session_store: &S,
    speech_session_id: &str,
    round_id: &str,
    state_gate: &Arc<Mutex<()>>,
) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
{
    let _state_guard = state_gate.lock().await;
    let Some(mut session) = session_store.get_session(speech_session_id).await? else {
        return Err(AppError::not_found("speech session not found"));
    };
    require_active_session(&session)?;
    let claimed = session.claim_output_turn(round_id)?;
    if !claimed {
        return Ok(());
    }
    session_store
        .save_session_and_push_events(
            speech_session_id,
            &session,
            &[SpeechRuntimeEvent::RespondingStarted],
        )
        .await
}

async fn ensure_reply_stream_active<S>(
    session_store: &S,
    speech_session_id: &str,
    round_id: &str,
    state_gate: &Arc<Mutex<()>>,
) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
{
    let _state_guard = state_gate.lock().await;
    let Some(session) = session_store.get_session(speech_session_id).await? else {
        return Err(AppError::not_found("speech session not found"));
    };
    require_active_reply_turn(&session, round_id)
}

fn require_active_reply_turn(session: &StoredSpeechSession, round_id: &str) -> AppResult<()> {
    require_active_session(session)?;
    if session
        .active_turn
        .as_ref()
        .is_some_and(|active_turn| active_turn.round_id == round_id)
    {
        return Ok(());
    }
    Err(AppError::conflict(TERMINAL_SESSION_MESSAGE))
}

async fn publish_log_event<E>(
    events: &E,
    session_id: i64,
    round_id: Option<&str>,
    event: &str,
    fields: serde_json::Value,
) -> AppResult<()>
where
    E: SpeechRuntimeEventPort + Send + Sync,
{
    events
        .publish(SpeechLogEvent {
            session_id,
            round_id: round_id.map(ToOwned::to_owned),
            event: event.to_owned(),
            fields,
        })
        .await
}

struct FailSpeechSessionState<'a, S, E> {
    session_store: &'a S,
    events: &'a E,
    voice_runtime: &'a dyn voice::VoiceRuntimeExecutionPort,
    speech_session_id: &'a str,
    session_id: i64,
    round_id: Option<&'a str>,
    reason: &'a str,
    state_gate: &'a Arc<Mutex<()>>,
}

async fn fail_speech_session_state<S, E>(command: FailSpeechSessionState<'_, S, E>) -> AppResult<()>
where
    S: SpeechSessionStorePort + Send + Sync,
    E: SpeechRuntimeEventPort + Send + Sync,
{
    let asr_session_id = {
        let _state_guard = command.state_gate.lock().await;
        let mut session = match command
            .session_store
            .get_session(command.speech_session_id)
            .await
        {
            Ok(Some(session)) => session,
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        };
        if matches!(
            session.phase,
            SpeechSessionPhase::Closing | SpeechSessionPhase::Failed
        ) {
            publish_log_event(
                command.events,
                command.session_id,
                command.round_id,
                "speech_turn_stale_absorbed",
                json!({ "reason": command.reason }),
            )
            .await?;
            return Ok(());
        }
        session.phase = SpeechSessionPhase::Failed;
        session.active_turn = None;
        command
            .session_store
            .save_session(command.speech_session_id, &session)
            .await?;
        command
            .session_store
            .push_events(
                command.speech_session_id,
                &[SpeechRuntimeEvent::SessionFailed {
                    message: command.reason.to_owned(),
                }],
            )
            .await?;
        publish_log_event(
            command.events,
            command.session_id,
            command.round_id,
            "speech_session_failed",
            json!({ "reason": command.reason }),
        )
        .await?;
        session.asr_session_id
    };
    command
        .voice_runtime
        .close_asr_session_for_runtime(voice::CloseRuntimeAsrSessionRequest { asr_session_id })
        .await?;
    Ok(())
}

fn validate_runtime_owner(runtime_owner_id: &str) -> AppResult<()> {
    if runtime_owner_id.trim().is_empty() {
        return Err(AppError::invalid_input("runtime_owner_id is required"));
    }
    Ok(())
}

fn require_active_session(session: &StoredSpeechSession) -> AppResult<()> {
    if matches!(
        session.phase,
        SpeechSessionPhase::Closing | SpeechSessionPhase::Failed
    ) {
        return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
    }
    Ok(())
}

fn is_stale_turn_error(error: &AppError) -> bool {
    matches!(error.code, AppErrorCode::NotFound)
        || (matches!(error.code, AppErrorCode::Conflict)
            && error.message.as_ref() == TERMINAL_SESSION_MESSAGE)
}

fn is_terminal_session_conflict(error: &AppError) -> bool {
    matches!(error.code, AppErrorCode::Conflict)
        && error.message.as_ref() == TERMINAL_SESSION_MESSAGE
}

fn input_drop_reason(
    session: &StoredSpeechSession,
    media_state: SpeechInputMediaState,
) -> Option<&'static str> {
    match media_state {
        SpeechInputMediaState::Open => {}
        SpeechInputMediaState::OutputTurnPending => return Some("worker_output_turn_pending"),
        SpeechInputMediaState::BotPlayback => return Some("worker_bot_playback"),
    }
    if matches!(session.phase, SpeechSessionPhase::Responding) || session.active_turn.is_some() {
        return Some("speech_runtime_live_output_turn");
    }
    None
}

fn should_reset_live_input_buffer(
    session: &StoredSpeechSession,
    media_state: SpeechInputMediaState,
) -> bool {
    matches!(
        media_state,
        SpeechInputMediaState::OutputTurnPending | SpeechInputMediaState::BotPlayback
    ) && !session.utterance_pcm.is_empty()
        && session.pending_rounds.is_empty()
        && session.active_turn.is_none()
}

fn completed_turn_start_receiver() -> watch::Receiver<TurnStartState> {
    let (_tx, rx) = watch::channel(TurnStartState::Started);
    rx
}

fn failed_turn_start_receiver(error: AppError) -> watch::Receiver<TurnStartState> {
    let (_tx, rx) = watch::channel(TurnStartState::Failed(TurnStartError::from_app_error(
        &error,
    )));
    rx
}

async fn wait_for_turn_start(mut rx: watch::Receiver<TurnStartState>) -> AppResult<()> {
    loop {
        match rx.borrow().clone() {
            TurnStartState::Started => return Ok(()),
            TurnStartState::Failed(error) => return Err(error.into_app_error()),
            TurnStartState::Pending => {}
        }
        if rx.changed().await.is_err() {
            return Err(AppError::internal(
                "speech server turn task exited before starting",
            ));
        }
    }
}

impl TurnStartError {
    fn from_app_error(error: &AppError) -> Self {
        Self {
            code: error.code,
            message: error.message.to_string(),
        }
    }

    fn into_app_error(self) -> AppError {
        AppError::new(self.code, self.message)
    }
}

fn speech_session_id(session_id: i64) -> String {
    format!("call-{session_id}")
}

fn turn_task_key(speech_session_id: &str, round_id: &str) -> String {
    format!("{speech_session_id}:{round_id}")
}

fn parse_speech_session_id(raw: &str) -> AppResult<i64> {
    raw.strip_prefix("call-")
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(|| AppError::invalid_input("invalid speech_session_id"))
}

impl StoredSpeechSession {
    fn new(
        command: CreateSpeechSessionCommand,
        agent_session_id: String,
        owner_backend_url: String,
        voice: String,
        asr_owner: AsrSessionOwner,
        language: voice::AsrLanguage,
        segmentation_config: SpeechSegmentationConfig,
    ) -> Self {
        Self {
            session_id: command.session_id,
            runtime_owner_id: command.runtime_owner_id,
            agent_session_id,
            owner_backend_url,
            voice,
            asr_session_id: asr_owner.asr_session_id,
            asr_owner_instance_id: asr_owner.instance_id,
            asr_owner_instance_epoch: asr_owner.instance_epoch,
            language,
            sample_rate_hz: command.sample_rate_hz,
            num_channels: command.num_channels,
            segmentation_config,
            utterance_pcm: Vec::new(),
            phase: SpeechSessionPhase::Listening,
            trailing_silence_ms: 0,
            deferred_flush: false,
            next_round_seq: 1,
            pending_rounds: VecDeque::new(),
            dispatched_round_ids: Vec::new(),
            active_turn: None,
            candidate_speech_ms: 0,
            candidate_pcm: Vec::new(),
        }
    }

    fn accept_input(
        &mut self,
        command: &PushSpeechInputCommand,
        segmentation_policy: &dyn SpeechSegmentationPolicy,
    ) -> (SpeechInputTransition, Option<Vec<i16>>) {
        self.sample_rate_hz = command.sample_rate_hz;
        self.num_channels = command.num_channels;

        let current: Vec<i16> = command.pcm_s16le.clone();
        match segmentation_policy.decide(self, &command.pcm_s16le) {
            SpeechSegmentationDecision::Ignore => {
                if self.phase == SpeechSessionPhase::SpeechDetected {
                    self.trailing_silence_ms += frame_duration_ms(
                        command.pcm_s16le.len(),
                        command.sample_rate_hz,
                        command.num_channels,
                    );
                }
                // 空闲/尾随静音照常 append,维持 ASR websocket 活性(静音不污染识别)
                (SpeechInputTransition::noop(), Some(current))
            }
            SpeechSegmentationDecision::SpeechPending => {
                let frame_ms = frame_duration_ms(
                    command.pcm_s16le.len(),
                    command.sample_rate_hz,
                    command.num_channels,
                );
                // 缓冲候选帧(确认后并入,不丢开头);计时加减(抗抖 / hangover)
                self.candidate_pcm.extend(command.pcm_s16le.iter().copied());
                if segmentation::has_voice_activity(
                    &command.pcm_s16le,
                    self.segmentation_config.voice_activity_threshold,
                ) {
                    self.candidate_speech_ms = self.candidate_speech_ms.saturating_add(frame_ms);
                } else {
                    self.candidate_speech_ms = self.candidate_speech_ms.saturating_sub(frame_ms);
                    if self.candidate_speech_ms == 0 {
                        self.candidate_pcm.clear();
                    }
                }
                // 候选期不进入 ASR(噪音不污染 provider buffer)
                (SpeechInputTransition::noop(), None)
            }
            SpeechSegmentationDecision::SpeechStarted => {
                self.phase = SpeechSessionPhase::SpeechDetected;
                self.trailing_silence_ms = 0;
                if self.pending_rounds.is_empty() {
                    self.deferred_flush = false;
                }
                // 前缀回溯:候选缓冲 + 当前帧 一并进入 utterance 与 ASR
                let mut asr_audio = std::mem::take(&mut self.candidate_pcm);
                asr_audio.extend(command.pcm_s16le.iter().copied());
                self.candidate_speech_ms = 0;
                self.utterance_pcm.extend(asr_audio.iter().copied());
                (
                    SpeechInputTransition {
                        events: vec![SpeechRuntimeEvent::SpeechDetected],
                        commit_request: None,
                    },
                    Some(asr_audio),
                )
            }
            SpeechSegmentationDecision::SpeechContinues => {
                self.trailing_silence_ms = 0;
                if self.pending_rounds.is_empty() {
                    self.deferred_flush = false;
                }
                self.utterance_pcm.extend(command.pcm_s16le.iter().copied());
                (SpeechInputTransition::noop(), Some(current))
            }
            SpeechSegmentationDecision::FlushUtterance => {
                if !self.pending_rounds.is_empty() {
                    if !self.deferred_flush {
                        self.trailing_silence_ms += frame_duration_ms(
                            command.pcm_s16le.len(),
                            command.sample_rate_hz,
                            command.num_channels,
                        );
                    }
                    self.deferred_flush = true;
                    return (SpeechInputTransition::noop(), Some(current));
                }
                self.phase = SpeechSessionPhase::Flushing;
                (
                    SpeechInputTransition {
                        events: vec![SpeechRuntimeEvent::UtteranceFlushing],
                        commit_request: self.take_pending_round_commit_request(),
                    },
                    Some(current),
                )
            }
        }
    }

    fn take_deferred_flush_transition(&mut self) -> SpeechInputTransition {
        if !self.deferred_flush || !self.pending_rounds.is_empty() {
            return SpeechInputTransition::noop();
        }
        self.phase = SpeechSessionPhase::Flushing;
        SpeechInputTransition {
            events: vec![SpeechRuntimeEvent::UtteranceFlushing],
            commit_request: self.take_pending_round_commit_request(),
        }
    }

    fn take_final_flush_transition(&mut self) -> SpeechInputTransition {
        if self.utterance_pcm.is_empty()
            || utterance_duration_ms(
                self.utterance_pcm.len(),
                self.sample_rate_hz,
                self.num_channels,
            ) < self.segmentation_config.min_utterance_ms
        {
            return SpeechInputTransition::noop();
        }
        if !self.pending_rounds.is_empty() {
            self.deferred_flush = true;
            return SpeechInputTransition::noop();
        }
        self.phase = SpeechSessionPhase::Flushing;
        SpeechInputTransition {
            events: vec![SpeechRuntimeEvent::UtteranceFlushing],
            commit_request: self.take_pending_round_commit_request(),
        }
    }

    fn take_pending_round_commit_request(&mut self) -> Option<PendingRoundCommitRequest> {
        if self.utterance_pcm.is_empty() {
            return None;
        }

        self.trailing_silence_ms = 0;
        self.deferred_flush = false;
        self.utterance_pcm.clear();
        let round_id = format!("speech-{}", self.next_round_seq);
        self.next_round_seq += 1;
        let commit_started_at_ms = now_millis();
        let extra_wait_ms = self
            .segmentation_config
            .silence_force_agent_ms
            .saturating_sub(self.segmentation_config.silence_flush_ms);
        Some(PendingRoundCommitRequest {
            round_id,
            force_agent_deadline_at_ms: commit_started_at_ms.checked_add(i64::from(extra_wait_ms)),
        })
    }

    fn push_pending_round(&mut self, commit_request: PendingRoundCommitRequest) {
        self.pending_rounds.push_back(PendingAsrRound {
            round_id: commit_request.round_id,
            latest_partial_transcript: String::new(),
            commit_started_at_ms: Some(now_millis()),
            force_agent_deadline_at_ms: commit_request.force_agent_deadline_at_ms,
        });
    }

    fn reset_live_input_buffer(&mut self) {
        self.utterance_pcm.clear();
        self.trailing_silence_ms = 0;
        self.deferred_flush = false;
        self.candidate_speech_ms = 0;
        self.candidate_pcm.clear();
        self.phase = SpeechSessionPhase::Listening;
    }

    /// 打断(barge-in)轻量复位:中止当前 turn、清当前轮输入/候选与未决 round,轮换到新 ASR。
    /// 保留 `dispatched_round_ids` 以忽略旧轮的 late final;不销毁会话。
    fn reset_for_interrupt(
        &mut self,
        new_asr_session_id: String,
        asr_owner_instance_id: String,
        asr_owner_instance_epoch: String,
    ) {
        self.active_turn = None;
        self.reset_live_input_buffer();
        self.pending_rounds.clear();
        self.asr_session_id = new_asr_session_id;
        self.asr_owner_instance_id = asr_owner_instance_id;
        self.asr_owner_instance_epoch = asr_owner_instance_epoch;
    }

    fn claim_output_turn(&mut self, round_id: &str) -> AppResult<bool> {
        if self
            .dispatched_round_ids
            .iter()
            .any(|dispatched| dispatched == round_id)
        {
            if self
                .active_turn
                .as_ref()
                .is_some_and(|active_turn| active_turn.round_id == round_id)
                && matches!(self.phase, SpeechSessionPhase::Responding)
            {
                return Ok(false);
            }
            return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
        }
        if let Some(index) = self
            .pending_rounds
            .iter()
            .position(|pending_round| pending_round.round_id == round_id)
        {
            if self.pending_rounds.len() > 1 {
                return Err(AppError::conflict(
                    "speech output turn cannot start while input rounds remain unresolved",
                ));
            }
            self.pending_rounds.remove(index);
        } else if !self.pending_rounds.is_empty() {
            return Err(AppError::conflict(
                "speech output turn cannot start while input rounds remain unresolved",
            ));
        }
        if self.active_turn.is_some() || matches!(self.phase, SpeechSessionPhase::Responding) {
            return Err(AppError::conflict(TERMINAL_SESSION_MESSAGE));
        }

        self.reset_live_input_buffer();
        self.phase = SpeechSessionPhase::Responding;
        self.dispatched_round_ids.push(round_id.to_owned());
        self.active_turn = Some(ActiveSpeechTurn {
            round_id: round_id.to_owned(),
        });
        Ok(true)
    }

    fn current_pending_round_id(&self) -> Option<&str> {
        self.pending_rounds
            .front()
            .map(|round| round.round_id.as_str())
    }

    fn current_pending_round_mut(&mut self) -> Option<&mut PendingAsrRound> {
        self.pending_rounds.front_mut()
    }

    fn take_forced_transcript_turn_if_overdue(
        &mut self,
        now_ms: i64,
    ) -> Option<ForcedTranscriptTurn> {
        let pending_round = self.pending_rounds.front()?;
        let deadline_at_ms = pending_round.force_agent_deadline_at_ms?;
        if now_ms < deadline_at_ms || pending_round.latest_partial_transcript.trim().is_empty() {
            return None;
        }
        let pending_round = self.pending_rounds.pop_front()?;
        Some(ForcedTranscriptTurn {
            round_id: pending_round.round_id,
            transcript: pending_round.latest_partial_transcript,
        })
    }

    fn remove_pending_round_id(&mut self, round_id: &str) -> AppResult<()> {
        let Some(index) = self
            .pending_rounds
            .iter()
            .position(|value| value.round_id == round_id)
        else {
            return Err(AppError::internal(
                "ASR runtime produced a final transcript for an unknown pending round",
            ));
        };
        self.pending_rounds.remove(index);
        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn frame_duration_ms(sample_count: usize, sample_rate_hz: u32, num_channels: u16) -> u32 {
    let channels = usize::from(num_channels.max(1));
    let samples_per_channel = sample_count / channels;
    ((samples_per_channel as u64) * 1000 / u64::from(sample_rate_hz.max(1))) as u32
}

fn utterance_duration_ms(sample_count: usize, sample_rate_hz: u32, num_channels: u16) -> u32 {
    frame_duration_ms(sample_count, sample_rate_hz, num_channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedSegmentationPolicy(SpeechSegmentationDecision);

    impl SpeechSegmentationPolicy for FixedSegmentationPolicy {
        fn decide(
            &self,
            _session: &StoredSpeechSession,
            _pcm_s16le: &[i16],
        ) -> SpeechSegmentationDecision {
            self.0
        }
    }

    fn test_session() -> StoredSpeechSession {
        StoredSpeechSession::new(
            CreateSpeechSessionCommand {
                session_id: 1,
                runtime_owner_id: "worker-test".to_owned(),
                sample_rate_hz: 16_000,
                num_channels: 1,
            },
            "agent-test".to_owned(),
            "http://backend:8080".to_owned(),
            "voice-test".to_owned(),
            AsrSessionOwner {
                asr_session_id: "asr-test".to_owned(),
                instance_id: "backend-test".to_owned(),
                instance_epoch: "epoch-test".to_owned(),
            },
            voice::AsrLanguage::Zh,
            SpeechSegmentationConfig {
                min_utterance_ms: 1,
                silence_flush_ms: 1,
                silence_force_agent_ms: 1,
                voice_activity_threshold: 1,
                min_speech_confirm_ms: 0,
            },
        )
    }

    #[test]
    fn flush_waits_when_asr_round_is_still_unresolved() {
        let mut session = test_session();
        session.utterance_pcm = vec![1; 160];
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });
        session.next_round_seq = 2;

        let (transition, _) = session.accept_input(
            &PushSpeechInputCommand {
                speech_session_id: "call-1".to_owned(),
                runtime_owner_id: "worker-test".to_owned(),
                pcm_s16le: vec![0; 160],
                sample_rate_hz: 16_000,
                num_channels: 1,
                media_state: SpeechInputMediaState::Open,
            },
            &FixedSegmentationPolicy(SpeechSegmentationDecision::FlushUtterance),
        );

        assert!(transition.commit_request.is_none());
        assert!(transition.events.is_empty());
        assert_eq!(session.pending_rounds.len(), 1);
        assert!(!session.utterance_pcm.is_empty());
        assert!(session.deferred_flush);
        assert_eq!(session.trailing_silence_ms, 10);
    }

    #[test]
    fn flush_commits_when_no_asr_round_is_unresolved() {
        let mut session = test_session();
        session.utterance_pcm = vec![1; 160];

        let (transition, _) = session.accept_input(
            &PushSpeechInputCommand {
                speech_session_id: "call-1".to_owned(),
                runtime_owner_id: "worker-test".to_owned(),
                pcm_s16le: vec![0; 160],
                sample_rate_hz: 16_000,
                num_channels: 1,
                media_state: SpeechInputMediaState::Open,
            },
            &FixedSegmentationPolicy(SpeechSegmentationDecision::FlushUtterance),
        );

        assert_eq!(
            transition
                .commit_request
                .as_ref()
                .map(|request| request.round_id.as_str()),
            Some("speech-1")
        );
        assert_eq!(
            transition.events,
            vec![SpeechRuntimeEvent::UtteranceFlushing]
        );
        assert!(session.utterance_pcm.is_empty());
        assert!(!session.deferred_flush);
    }

    #[test]
    fn input_drop_reason_drops_worker_output_states_without_asr() {
        let session = test_session();

        assert_eq!(
            input_drop_reason(&session, SpeechInputMediaState::OutputTurnPending),
            Some("worker_output_turn_pending")
        );
        assert_eq!(
            input_drop_reason(&session, SpeechInputMediaState::BotPlayback),
            Some("worker_bot_playback")
        );
    }

    fn voiced_or_silent_frame(samples: Vec<i16>) -> PushSpeechInputCommand {
        PushSpeechInputCommand {
            speech_session_id: "call-1".to_owned(),
            runtime_owner_id: "worker-test".to_owned(),
            pcm_s16le: samples,
            sample_rate_hz: 16_000,
            num_channels: 1,
            media_state: SpeechInputMediaState::Open,
        }
    }

    #[test]
    fn min_speech_confirm_holds_candidate_then_confirms_with_prefix() {
        let mut session = test_session();
        session.segmentation_config.voice_activity_threshold = 1000;
        session.segmentation_config.min_speech_confirm_ms = 30; // 3 帧 @10ms
        let policy = ThresholdSpeechSegmentationPolicy;

        // 帧 1、2:候选累积,不进入 ASR、不计入 utterance
        let (t1, a1) = session.accept_input(&voiced_or_silent_frame(vec![5000; 160]), &policy);
        assert!(t1.events.is_empty());
        assert!(a1.is_none());
        assert!(session.utterance_pcm.is_empty());
        assert_eq!(session.candidate_speech_ms, 10);

        let (_, a2) = session.accept_input(&voiced_or_silent_frame(vec![5000; 160]), &policy);
        assert!(a2.is_none());
        assert_eq!(session.candidate_speech_ms, 20);

        // 帧 3:达到确认窗 → 正式开始,前缀(2 帧)+ 当前帧一并并入 utterance 与 ASR
        let (t3, a3) = session.accept_input(&voiced_or_silent_frame(vec![5000; 160]), &policy);
        assert_eq!(t3.events, vec![SpeechRuntimeEvent::SpeechDetected]);
        assert_eq!(a3.expect("confirm pushes prefix+current").len(), 160 * 3);
        assert_eq!(session.utterance_pcm.len(), 160 * 3);
        assert_eq!(session.candidate_speech_ms, 0);
        assert!(session.candidate_pcm.is_empty());
    }

    #[test]
    fn short_noise_is_discarded_without_asr_push() {
        let mut session = test_session();
        session.segmentation_config.voice_activity_threshold = 1000;
        session.segmentation_config.min_speech_confirm_ms = 100;
        let policy = ThresholdSpeechSegmentationPolicy;

        let (_, a1) = session.accept_input(&voiced_or_silent_frame(vec![5000; 160]), &policy);
        assert!(a1.is_none());
        assert_eq!(session.candidate_speech_ms, 10);
        assert_eq!(session.candidate_pcm.len(), 160);

        // 随后静音:候选递减归零并清空,不进入 ASR、不计入 utterance
        let (_, a2) = session.accept_input(&voiced_or_silent_frame(vec![0; 160]), &policy);
        assert!(a2.is_none());
        assert_eq!(session.candidate_speech_ms, 0);
        assert!(session.candidate_pcm.is_empty());
        assert!(session.utterance_pcm.is_empty());
    }

    #[test]
    fn confirm_zero_starts_on_first_voiced_frame() {
        let mut session = test_session(); // min_speech_confirm_ms = 0
        session.segmentation_config.voice_activity_threshold = 1000;
        let policy = ThresholdSpeechSegmentationPolicy;

        let (t, a) = session.accept_input(&voiced_or_silent_frame(vec![5000; 160]), &policy);
        assert_eq!(t.events, vec![SpeechRuntimeEvent::SpeechDetected]);
        assert_eq!(a.expect("push current frame").len(), 160);
        assert_eq!(session.utterance_pcm.len(), 160);
    }

    #[test]
    fn idle_silence_keeps_pushing_to_asr() {
        let mut session = test_session();
        session.segmentation_config.voice_activity_threshold = 1000;
        let policy = ThresholdSpeechSegmentationPolicy;

        // 完全空闲静音:不进入语音,但仍 append 维持 ASR 活性
        let (t, a) = session.accept_input(&voiced_or_silent_frame(vec![0; 160]), &policy);
        assert!(t.events.is_empty());
        assert!(a.is_some());
        assert_eq!(session.candidate_speech_ms, 0);
        assert!(session.utterance_pcm.is_empty());
    }

    #[test]
    fn reset_for_interrupt_returns_to_listening_and_rotates_asr() {
        let mut session = test_session();
        // 构造"正在回复 + 有未决轮 + 有缓冲"的脏状态
        session.phase = SpeechSessionPhase::Responding;
        session.utterance_pcm = vec![1; 320];
        session.candidate_speech_ms = 40;
        session.candidate_pcm = vec![2; 160];
        session.trailing_silence_ms = 30;
        session.deferred_flush = true;
        session.active_turn = Some(ActiveSpeechTurn {
            round_id: "speech-1".to_owned(),
        });
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: "半句".to_owned(),
            commit_started_at_ms: Some(1),
            force_agent_deadline_at_ms: Some(2),
        });
        session.dispatched_round_ids = vec!["speech-0".to_owned()];
        session.asr_session_id = "asr-old".to_owned();

        session.reset_for_interrupt(
            "asr-new".to_owned(),
            "backend-2".to_owned(),
            "epoch-2".to_owned(),
        );

        // 回到可监听、当前轮全部清空
        assert!(session.active_turn.is_none());
        assert_eq!(session.phase, SpeechSessionPhase::Listening);
        assert!(session.utterance_pcm.is_empty());
        assert_eq!(session.candidate_speech_ms, 0);
        assert!(session.candidate_pcm.is_empty());
        assert_eq!(session.trailing_silence_ms, 0);
        assert!(!session.deferred_flush);
        assert!(session.pending_rounds.is_empty());
        // 轮换到新 ASR
        assert_eq!(session.asr_session_id, "asr-new");
        assert_eq!(session.asr_owner_instance_id, "backend-2");
        assert_eq!(session.asr_owner_instance_epoch, "epoch-2");
        // dispatched 保留(用于忽略旧轮 late final)
        assert_eq!(session.dispatched_round_ids, vec!["speech-0".to_owned()]);
    }

    #[test]
    fn output_turn_pending_resets_uncommitted_live_input_buffer() {
        let mut session = test_session();
        session.phase = SpeechSessionPhase::SpeechDetected;
        session.utterance_pcm = vec![1; 320];
        session.trailing_silence_ms = 120;
        session.deferred_flush = true;

        assert!(should_reset_live_input_buffer(
            &session,
            SpeechInputMediaState::OutputTurnPending
        ));

        session.reset_live_input_buffer();

        assert_eq!(session.phase, SpeechSessionPhase::Listening);
        assert!(session.utterance_pcm.is_empty());
        assert_eq!(session.trailing_silence_ms, 0);
        assert!(!session.deferred_flush);
    }

    #[test]
    fn output_turn_pending_does_not_reset_committed_round_state() {
        let mut session = test_session();
        session.phase = SpeechSessionPhase::SpeechDetected;
        session.utterance_pcm = vec![1; 320];
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });

        assert!(!should_reset_live_input_buffer(
            &session,
            SpeechInputMediaState::OutputTurnPending
        ));
    }

    #[test]
    fn claim_output_turn_clears_uncommitted_live_input_buffer() {
        let mut session = test_session();
        session.phase = SpeechSessionPhase::SpeechDetected;
        session.utterance_pcm = vec![1; 320];
        session.trailing_silence_ms = 120;
        session.deferred_flush = true;

        let claimed = session.claim_output_turn("speech-1").unwrap();

        assert!(claimed);
        assert_eq!(session.phase, SpeechSessionPhase::Responding);
        assert!(session.utterance_pcm.is_empty());
        assert_eq!(session.trailing_silence_ms, 0);
        assert!(!session.deferred_flush);
        assert_eq!(
            session.active_turn,
            Some(ActiveSpeechTurn {
                round_id: "speech-1".to_owned(),
            })
        );
    }

    #[test]
    fn claim_output_turn_accepts_and_consumes_matching_pending_round() {
        let mut session = test_session();
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });

        let claimed = session.claim_output_turn("speech-1").unwrap();

        assert!(claimed);
        assert!(session.pending_rounds.is_empty());
        assert_eq!(session.phase, SpeechSessionPhase::Responding);
        assert_eq!(
            session.active_turn,
            Some(ActiveSpeechTurn {
                round_id: "speech-1".to_owned(),
            })
        );
    }

    #[test]
    fn claim_output_turn_rejects_overlapping_input_rounds() {
        let mut session = test_session();
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });

        let error = session.claim_output_turn("speech-2").unwrap_err();

        assert_eq!(error.code, AppErrorCode::Conflict);
        assert_eq!(
            error.message.as_ref(),
            "speech output turn cannot start while input rounds remain unresolved"
        );
    }

    #[test]
    fn input_drop_reason_drops_concurrent_live_turn_even_when_worker_is_open() {
        let mut session = test_session();
        session.phase = SpeechSessionPhase::Responding;
        session.active_turn = Some(ActiveSpeechTurn {
            round_id: "round-1".to_owned(),
        });

        assert_eq!(
            input_drop_reason(&session, SpeechInputMediaState::Open),
            Some("speech_runtime_live_output_turn")
        );
    }

    #[test]
    fn deferred_flush_commits_after_unresolved_round_is_cleared() {
        let mut session = test_session();
        session.utterance_pcm = vec![1; 160];
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });
        session.next_round_seq = 2;

        let (first_transition, _) = session.accept_input(
            &PushSpeechInputCommand {
                speech_session_id: "call-1".to_owned(),
                runtime_owner_id: "worker-test".to_owned(),
                pcm_s16le: vec![0; 160],
                sample_rate_hz: 16_000,
                num_channels: 1,
                media_state: SpeechInputMediaState::Open,
            },
            &FixedSegmentationPolicy(SpeechSegmentationDecision::FlushUtterance),
        );
        assert!(first_transition.commit_request.is_none());

        let removed = session.remove_pending_round_id("speech-1");
        assert!(removed.is_ok());
        let deferred_transition = session.take_deferred_flush_transition();

        assert_eq!(
            deferred_transition
                .commit_request
                .as_ref()
                .map(|request| request.round_id.as_str()),
            Some("speech-2")
        );
        assert_eq!(
            deferred_transition.events,
            vec![SpeechRuntimeEvent::UtteranceFlushing]
        );
        assert!(session.utterance_pcm.is_empty());
        assert!(!session.deferred_flush);
    }

    #[test]
    fn deferred_flush_survives_continued_speech_before_unresolved_round_clears() {
        let mut session = test_session();
        session.utterance_pcm = vec![1; 160];
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
        });
        session.next_round_seq = 2;

        let (flush_transition, _) = session.accept_input(
            &PushSpeechInputCommand {
                speech_session_id: "call-1".to_owned(),
                runtime_owner_id: "worker-test".to_owned(),
                pcm_s16le: vec![0; 160],
                sample_rate_hz: 16_000,
                num_channels: 1,
                media_state: SpeechInputMediaState::Open,
            },
            &FixedSegmentationPolicy(SpeechSegmentationDecision::FlushUtterance),
        );
        assert!(flush_transition.commit_request.is_none());
        assert!(session.deferred_flush);

        let (continued_transition, _) = session.accept_input(
            &PushSpeechInputCommand {
                speech_session_id: "call-1".to_owned(),
                runtime_owner_id: "worker-test".to_owned(),
                pcm_s16le: vec![2; 160],
                sample_rate_hz: 16_000,
                num_channels: 1,
                media_state: SpeechInputMediaState::Open,
            },
            &FixedSegmentationPolicy(SpeechSegmentationDecision::SpeechContinues),
        );
        assert!(continued_transition.commit_request.is_none());
        assert!(session.deferred_flush);

        let removed = session.remove_pending_round_id("speech-1");
        assert!(removed.is_ok());
        let deferred_transition = session.take_deferred_flush_transition();

        assert_eq!(
            deferred_transition
                .commit_request
                .as_ref()
                .map(|request| request.round_id.as_str()),
            Some("speech-2")
        );
        assert!(!session.deferred_flush);
    }

    #[test]
    fn deferred_flush_transition_does_not_reapply_min_utterance_gate() {
        let mut session = test_session();
        session.segmentation_config.min_utterance_ms = 1_000;
        session.utterance_pcm = vec![1; 160];
        session.deferred_flush = true;

        let transition = session.take_deferred_flush_transition();

        assert_eq!(
            transition
                .commit_request
                .as_ref()
                .map(|request| request.round_id.as_str()),
            Some("speech-1")
        );
        assert!(!session.deferred_flush);
    }

    #[test]
    fn force_finalize_uses_latest_partial_after_deadline() {
        let mut session = test_session();
        session.pending_rounds.push_back(PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: "让我".to_owned(),
            commit_started_at_ms: Some(1_000),
            force_agent_deadline_at_ms: Some(1_400),
        });

        let forced = session.take_forced_transcript_turn_if_overdue(1_401);

        assert_eq!(
            forced,
            Some(ForcedTranscriptTurn {
                round_id: "speech-1".to_owned(),
                transcript: "让我".to_owned(),
            })
        );
        assert!(session.pending_rounds.is_empty());
    }
}
