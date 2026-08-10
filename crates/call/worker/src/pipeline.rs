use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use call_runtime_control::{RuntimeEventFact, RuntimeFactKind, SpeechInputMediaState};
use reqwest::Client as HttpClient;
use tokio::{
    sync::{
        Mutex,
        mpsc::{UnboundedSender, unbounded_channel},
        oneshot, watch,
    },
    task::{JoinError, JoinHandle, JoinSet},
};

use crate::{
    client::{
        CreateSpeechSessionRequest, FailOwnerRouteRequest, PushSpeechInputRequest,
        RuntimeControlClient, SpeechRuntimeEvent, is_not_found_control_plane_error,
        is_push_input_not_accepting_error, is_retryable_control_plane_error,
    },
    external_audio::load_pre_recorded_frames,
    input::StreamingUserAudioInput,
    playback::BotPlayback,
    preprocess::{AudioPreprocessor, PreprocessedFrame},
};
use rtc::livekit::pcm::PcmFrame;

const MAX_EVENT_BATCH: usize = 16;
const BARGE_IN_PREFIX_MS: u32 = 500;
const BARGE_IN_CONFIRM_MS: u32 = 250;
const BARGE_IN_SUPPRESS_LOG_INTERVAL_MS: i64 = 1_000;
const ECHO_CORRELATION_THRESHOLD: f32 = 0.72;
const ECHO_MIN_DELAY_MS: u32 = 40;
const ECHO_MAX_DELAY_MS: u32 = 420;
const ECHO_DELAY_STEP_MS: u32 = 20;
const PIPELINE_STOP_GRACE_MS: u64 = 2_000;
// 控制面调用(push/poll/publish 等)重试的总时长上限。此前是无限重试,而这些调用
// 内联在唯一拉取用户音频的任务里,持续可重试错误会永久冻结音频管线 → 丢帧 + ASR 空闲断开。
const CONTROL_PLANE_RETRY_MAX_TOTAL_MS: u64 = 30_000;
// 入站音频解耦队列容量(约 1s,正常抖动够用);下游短暂卡顿时丢最新帧,避免无界堆积/原生队列溢出。
const INBOUND_AUDIO_QUEUE_CAP: usize = 100;
const INBOUND_DROP_LOG_INTERVAL: u64 = 100;

pub struct SpeechPipelineConfig {
    pub preprocessor: AudioPreprocessor,
    pub speech_poll_interval_ms: u64,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
}

#[async_trait]
pub trait SpeechHandler: Send + Sync {
    async fn ensure_available(&self) -> Result<()>;
    async fn start_stream(&self, sample_rate_hz: u32, num_channels: u16) -> Result<()>;
    async fn push_frame(&self, frame: PcmFrame, media_state: SpeechInputMediaState) -> Result<()>;
    async fn poll_output_events(&self) -> Result<Vec<SpeechRuntimeEvent>>;
    async fn interrupt_stream(&self) -> Result<Vec<SpeechRuntimeEvent>>;
    async fn publish_worker_event(
        &self,
        round_id: Option<&str>,
        kind: RuntimeFactKind,
    ) -> Result<()>;
    async fn close_stream(&self) -> Result<Vec<SpeechRuntimeEvent>>;
}

pub struct RemoteSpeechHandler {
    client: RuntimeControlClient,
    session_id: i64,
    runtime_owner_id: String,
    control_plane_retry_initial_ms: u64,
    control_plane_retry_max_ms: u64,
    speech_session_id: tokio::sync::Mutex<Option<String>>,
    speech_client: tokio::sync::Mutex<Option<RuntimeControlClient>>,
    stream_config: tokio::sync::Mutex<Option<(u32, u16)>>,
}

impl RemoteSpeechHandler {
    pub fn new(
        client: RuntimeControlClient,
        session_id: i64,
        runtime_owner_id: String,
        control_plane_retry_initial_ms: u64,
        control_plane_retry_max_ms: u64,
    ) -> Self {
        Self {
            client,
            session_id,
            runtime_owner_id,
            control_plane_retry_initial_ms,
            control_plane_retry_max_ms,
            speech_session_id: tokio::sync::Mutex::new(None),
            speech_client: tokio::sync::Mutex::new(None),
            stream_config: tokio::sync::Mutex::new(None),
        }
    }

    async fn require_speech_session_id(&self) -> Result<String> {
        self.speech_session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("speech session has not been started"))
    }

    async fn require_speech_client(&self) -> Result<RuntimeControlClient> {
        self.speech_client
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("speech session control-plane client is not pinned"))
    }

    async fn run_with_control_plane_retry<T, F, Fut>(
        &self,
        operation: &'static str,
        mut op: F,
    ) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut retry_delay_ms = self.control_plane_retry_initial_ms.max(1);
        let retry_deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(CONTROL_PLANE_RETRY_MAX_TOTAL_MS);
        loop {
            match op().await {
                Ok(value) => return Ok(value),
                Err(error) if is_retryable_control_plane_error(&error) => {
                    // 重试必须有总时长上限:该调用内联在唯一拉取用户音频的任务里,无限重试会永久
                    // 冻结音频管线(丢帧 + ASR 空闲断开)。超时即放弃,让上层降级而非冻结。
                    if tokio::time::Instant::now() >= retry_deadline {
                        tracing::warn!(
                            session_id = self.session_id,
                            runtime_owner_id = %self.runtime_owner_id,
                            operation,
                            max_total_ms = CONTROL_PLANE_RETRY_MAX_TOTAL_MS,
                            error = %error,
                            "worker speech control-plane operation exceeded retry deadline; giving up"
                        );
                        return Err(error);
                    }
                    tracing::warn!(
                        session_id = self.session_id,
                        runtime_owner_id = %self.runtime_owner_id,
                        retry_delay_ms,
                        error = %error,
                        operation,
                        "worker speech control-plane operation hit temporary error; retrying"
                    );
                    let remaining_ms = retry_deadline
                        .saturating_duration_since(tokio::time::Instant::now())
                        .as_millis() as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        retry_delay_ms.min(remaining_ms).max(1),
                    ))
                    .await;
                    retry_delay_ms = (retry_delay_ms.saturating_mul(2))
                        .min(self.control_plane_retry_max_ms.max(1));
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn fail_owner_route(&self, reason: &str, owner_route: &OwnerRoute) -> Result<()> {
        self.client
            .with_base_url(owner_route.backend_url.clone())
            .fail_owner_route(
                &format!("call-{}", self.session_id),
                FailOwnerRouteRequest {
                    runtime_owner_id: self.runtime_owner_id.clone(),
                    owner_backend_url: owner_route.backend_url.clone(),
                    owner_instance_id: owner_route.instance_id.clone(),
                    owner_instance_epoch: owner_route.instance_epoch.clone(),
                    reason: reason.to_owned(),
                },
            )
            .await
    }
}

#[async_trait]
impl SpeechHandler for RemoteSpeechHandler {
    async fn ensure_available(&self) -> Result<()> {
        if self.runtime_owner_id.trim().is_empty() {
            return Err(anyhow!("worker runtime_owner_id is required"));
        }
        Ok(())
    }

    async fn start_stream(&self, sample_rate_hz: u32, num_channels: u16) -> Result<()> {
        let mut retry_delay_ms = self.control_plane_retry_initial_ms.max(1);
        // 同样加总时长上限:barge-in 远程打断后会用 start_stream 重建会话,无限重试会让主循环
        // 一直停在 pending_interrupt 并把入站帧无界堆进 buffered_frames(重新引入卡死/内存增长)。
        let retry_deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(CONTROL_PLANE_RETRY_MAX_TOTAL_MS);
        let mut create_client = self.client.clone();
        let mut expected_owner_route: Option<OwnerRoute> = None;
        let response = loop {
            let response = match create_client
                .create_speech_session(CreateSpeechSessionRequest {
                    session_id: self.session_id,
                    runtime_owner_id: self.runtime_owner_id.clone(),
                    sample_rate_hz,
                    num_channels,
                })
                .await
            {
                Ok(response) => response,
                Err(error)
                    if expected_owner_route.is_none()
                        && is_retryable_control_plane_error(&error) =>
                {
                    if tokio::time::Instant::now() >= retry_deadline {
                        tracing::warn!(
                            session_id = self.session_id,
                            runtime_owner_id = %self.runtime_owner_id,
                            max_total_ms = CONTROL_PLANE_RETRY_MAX_TOTAL_MS,
                            error = %error,
                            "worker speech session create exceeded retry deadline; giving up"
                        );
                        return Err(error);
                    }
                    tracing::warn!(
                        session_id = self.session_id,
                        runtime_owner_id = %self.runtime_owner_id,
                        retry_delay_ms,
                        error = %error,
                        "worker speech session create hit temporary control-plane error; retrying"
                    );
                    let remaining_ms = retry_deadline
                        .saturating_duration_since(tokio::time::Instant::now())
                        .as_millis() as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(
                        retry_delay_ms.min(remaining_ms).max(1),
                    ))
                    .await;
                    retry_delay_ms = (retry_delay_ms.saturating_mul(2))
                        .min(self.control_plane_retry_max_ms.max(1));
                    continue;
                }
                Err(error) => {
                    if let Some(owner_route) = expected_owner_route.as_ref() {
                        self.fail_owner_route(
                            "speech session owner route was unreachable during create",
                            owner_route,
                        )
                        .await?;
                    }
                    return Err(error);
                }
            };
            if !response.owner_route_only {
                if let Some(owner_route) = expected_owner_route.as_ref() {
                    let actual = OwnerRoute {
                        backend_url: response.owner_backend_url.trim().to_owned(),
                        instance_id: response.owner_instance_id.clone(),
                        instance_epoch: response.owner_instance_epoch.clone(),
                    };
                    if actual != *owner_route {
                        self.fail_owner_route(
                            "speech session owner route resolved to a different owner",
                            owner_route,
                        )
                        .await?;
                        return Err(anyhow!(
                            "speech session owner route resolved to a different owner"
                        ));
                    }
                }
                break response;
            }
            let owner_backend_url = response.owner_backend_url.trim();
            if owner_backend_url.is_empty() {
                return Err(anyhow!(
                    "speech session owner route response is missing owner_backend_url"
                ));
            }
            let owner_route = OwnerRoute {
                backend_url: owner_backend_url.to_owned(),
                instance_id: response.owner_instance_id.clone(),
                instance_epoch: response.owner_instance_epoch.clone(),
            };
            if owner_route.instance_id.trim().is_empty()
                || owner_route.instance_epoch.trim().is_empty()
            {
                return Err(anyhow!(
                    "speech session owner route response is missing owner identity"
                ));
            }
            if let Some(expected_owner_route) = expected_owner_route.as_ref() {
                self.fail_owner_route(
                    "speech session owner route did not converge",
                    expected_owner_route,
                )
                .await?;
                return Err(anyhow!(
                    "speech session owner route did not converge after routing to {owner_backend_url}"
                ));
            }
            tracing::debug!(
                session_id = self.session_id,
                runtime_owner_id = %self.runtime_owner_id,
                owner_backend_url,
                owner_instance_id = %owner_route.instance_id,
                owner_instance_epoch = %owner_route.instance_epoch,
                "worker routing speech session create to owner backend"
            );
            create_client = self.client.with_base_url(owner_backend_url.to_owned());
            expected_owner_route = Some(owner_route);
            retry_delay_ms = self.control_plane_retry_initial_ms.max(1);
        };
        if response.owner_backend_url.trim().is_empty() {
            return Err(anyhow!(
                "speech session create response is missing owner_backend_url"
            ));
        }

        *self.speech_session_id.lock().await = Some(response.speech_session_id);
        *self.speech_client.lock().await = Some(
            self.client
                .with_base_url(response.owner_backend_url.trim().to_owned()),
        );
        *self.stream_config.lock().await = Some((sample_rate_hz, num_channels));
        Ok(())
    }

    async fn push_frame(&self, frame: PcmFrame, media_state: SpeechInputMediaState) -> Result<()> {
        let speech_session_id = self.require_speech_session_id().await?;
        let speech_client = self.require_speech_client().await?;
        tracing::debug!(
            session_id = self.session_id,
            sample_rate = frame.sample_rate,
            num_channels = frame.num_channels,
            samples_per_channel = frame.samples_per_channel,
            media_state = ?media_state,
            "worker streaming speech input frame to backend"
        );

        self.run_with_control_plane_retry("push speech input", || async {
            speech_client
                .push_speech_input(
                    &speech_session_id,
                    PushSpeechInputRequest {
                        runtime_owner_id: self.runtime_owner_id.clone(),
                        pcm_s16le: frame.data.clone(),
                        sample_rate_hz: frame.sample_rate,
                        num_channels: frame.num_channels as u16,
                        media_state,
                    },
                )
                .await
        })
        .await
    }

    async fn poll_output_events(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let speech_session_id = self.require_speech_session_id().await?;
        let speech_client = self.require_speech_client().await?;
        let response = self
            .run_with_control_plane_retry("poll speech events", || async {
                speech_client
                    .poll_speech_events(&speech_session_id, &self.runtime_owner_id, MAX_EVENT_BATCH)
                    .await
            })
            .await?;
        Ok(response.events)
    }

    async fn interrupt_stream(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let Some(speech_session_id) = self.speech_session_id.lock().await.clone() else {
            return Ok(Vec::new());
        };
        let Some((sample_rate_hz, num_channels)) = *self.stream_config.lock().await else {
            return Err(anyhow!("speech stream configuration is missing"));
        };

        tracing::debug!(
            session_id = self.session_id,
            runtime_owner_id = %self.runtime_owner_id,
            speech_session_id = %speech_session_id,
            sample_rate_hz,
            num_channels,
            "worker speech control-plane interrupt_stream begin"
        );

        let speech_client = self.require_speech_client().await?;
        let response = match self
            .run_with_control_plane_retry("interrupt speech session", || async {
                speech_client
                    .interrupt_speech_session(&speech_session_id, &self.runtime_owner_id)
                    .await
            })
            .await
        {
            Ok(response) => response,
            Err(error) if is_not_found_control_plane_error(&error, "interrupt speech session") => {
                tracing::warn!(
                    session_id = self.session_id,
                    runtime_owner_id = %self.runtime_owner_id,
                    speech_session_id = %speech_session_id,
                    error = %error,
                    "worker speech control-plane interrupt_stream target was already gone; restarting stream"
                );
                *self.speech_session_id.lock().await = None;
                *self.speech_client.lock().await = None;
                self.start_stream(sample_rate_hz, num_channels).await?;
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        tracing::debug!(
            session_id = self.session_id,
            runtime_owner_id = %self.runtime_owner_id,
            event_count = response.events.len(),
            "worker speech control-plane interrupt_stream acknowledged"
        );
        *self.speech_session_id.lock().await = None;
        *self.speech_client.lock().await = None;
        tracing::debug!(
            session_id = self.session_id,
            runtime_owner_id = %self.runtime_owner_id,
            "worker speech control-plane interrupt_stream restarting stream"
        );
        self.start_stream(sample_rate_hz, num_channels).await?;
        tracing::debug!(
            session_id = self.session_id,
            runtime_owner_id = %self.runtime_owner_id,
            "worker speech control-plane interrupt_stream completed"
        );
        Ok(response.events)
    }

    async fn publish_worker_event(
        &self,
        round_id: Option<&str>,
        kind: RuntimeFactKind,
    ) -> Result<()> {
        let fact = RuntimeEventFact {
            session_id: self.session_id,
            runtime_owner_id: self.runtime_owner_id.clone(),
            round_id: round_id.map(ToOwned::to_owned),
            source: "worker".to_owned(),
            ts_ms: now_ms(),
            kind,
        };
        self.run_with_control_plane_retry("publish worker event", || async {
            self.client.publish_event(fact.clone()).await
        })
        .await
    }

    async fn close_stream(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let speech_session_id = self.require_speech_session_id().await?;
        let speech_client = self.require_speech_client().await?;
        let response = self
            .run_with_control_plane_retry("close speech session", || async {
                speech_client
                    .close_speech_session(&speech_session_id, &self.runtime_owner_id)
                    .await
            })
            .await?;
        *self.speech_session_id.lock().await = None;
        *self.speech_client.lock().await = None;
        Ok(response.events)
    }
}

#[derive(Clone)]
struct ActivePlayback {
    round_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BotTurnGate {
    round_id: String,
    interruptible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOutputTurn {
    round_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputGateMode {
    Closed,
    Open,
    OutputTurnPending {
        round_id: Option<String>,
    },
    BotTurn {
        round_id: String,
        interruptible: bool,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct InputGate {
    pending_output: Option<PendingOutputTurn>,
    bot_playback: Option<BotTurnGate>,
    terminal: bool,
}

impl InputGate {
    fn mode(&self) -> InputGateMode {
        if self.terminal {
            return InputGateMode::Closed;
        }
        if let Some(bot_turn) = self.bot_playback.as_ref() {
            return InputGateMode::BotTurn {
                round_id: bot_turn.round_id.clone(),
                interruptible: bot_turn.interruptible,
            };
        }
        if let Some(pending) = self.pending_output.as_ref() {
            return InputGateMode::OutputTurnPending {
                round_id: pending.round_id.clone(),
            };
        }
        InputGateMode::Open
    }

    fn apply_event(&mut self, event: &SpeechRuntimeEvent) {
        match event {
            SpeechRuntimeEvent::ListeningStarted => {}
            SpeechRuntimeEvent::RespondingStarted => {
                self.start_pending_output(None);
            }
            SpeechRuntimeEvent::ReplyStarted { round_id, .. } => {
                self.start_pending_output(Some(round_id));
            }
            SpeechRuntimeEvent::PreRecordedAsset { round_id, .. } => {
                self.start_pending_output(Some(round_id));
            }
            SpeechRuntimeEvent::SessionClosing | SpeechRuntimeEvent::SessionFailed { .. } => {
                self.terminal = true;
                self.pending_output = None;
                self.bot_playback = None;
            }
            SpeechRuntimeEvent::RoundFailed { round_id, .. } => {
                self.clear_output_turn(round_id);
            }
            SpeechRuntimeEvent::SpeechDetected
            | SpeechRuntimeEvent::UtteranceFlushing
            | SpeechRuntimeEvent::ReplyFinished { .. }
            | SpeechRuntimeEvent::AudioChunk { .. }
            | SpeechRuntimeEvent::Warning { .. } => {}
        }
    }

    fn start_pending_output(&mut self, round_id: Option<&str>) {
        if self.terminal || self.bot_playback.is_some() {
            return;
        }
        let from_state = self.state_name();
        self.pending_output = Some(PendingOutputTurn {
            round_id: round_id.map(ToOwned::to_owned),
        });
        self.log_gate_changed(
            from_state,
            self.state_name(),
            round_id,
            "output_turn_started",
        );
    }

    fn start_bot_playback(&mut self, round_id: &str, interruptible: bool) {
        let from_state = self.state_name();
        self.clear_pending_output(round_id);
        self.bot_playback = Some(BotTurnGate {
            round_id: round_id.to_owned(),
            interruptible,
        });
        self.log_gate_changed(
            from_state,
            self.state_name(),
            Some(round_id),
            "local_playback_started",
        );
    }

    fn clear_pending_output(&mut self, round_id: &str) {
        if self.pending_output.as_ref().is_some_and(|pending| {
            pending
                .round_id
                .as_ref()
                .is_none_or(|pending_round_id| pending_round_id == round_id)
        }) {
            self.pending_output = None;
        }
    }

    fn clear_bot_playback(&mut self, round_id: &str) {
        if self
            .bot_playback
            .as_ref()
            .is_some_and(|bot_turn| bot_turn.round_id == round_id)
        {
            self.bot_playback = None;
        }
    }

    fn clear_output_turn(&mut self, round_id: &str) {
        let from_state = self.state_name();
        self.clear_pending_output(round_id);
        self.clear_bot_playback(round_id);
        self.log_gate_changed(
            from_state,
            self.state_name(),
            Some(round_id),
            "output_turn_cleared",
        );
    }

    fn clear_all(&mut self) {
        let from_state = self.state_name();
        self.terminal = true;
        self.pending_output = None;
        self.bot_playback = None;
        self.log_gate_changed(from_state, self.state_name(), None, "session_terminal");
    }

    fn open_after_interrupt(&mut self) {
        let from_state = self.state_name();
        self.terminal = false;
        self.pending_output = None;
        self.bot_playback = None;
        self.log_gate_changed(from_state, self.state_name(), None, "barge_in_interrupt");
    }

    fn state_name(&self) -> &'static str {
        match self.mode() {
            InputGateMode::Closed => "closed",
            InputGateMode::Open => "open",
            InputGateMode::OutputTurnPending { .. } => "output_turn_pending",
            InputGateMode::BotTurn { .. } => "bot_playback",
        }
    }

    fn log_gate_changed(
        &self,
        from_state: &'static str,
        to_state: &'static str,
        round_id: Option<&str>,
        reason: &'static str,
    ) {
        if from_state == to_state {
            return;
        }
        tracing::debug!(
            session_round_id = round_id.unwrap_or("unknown"),
            from_state,
            to_state,
            reason,
            "worker input gate changed"
        );
    }
}

struct PendingReply {
    round_id: String,
    reply_text: String,
    interruptible: bool,
    frames_tx: Option<UnboundedSender<PcmFrame>>,
    playback_started: bool,
}

struct PendingInterruptTransition {
    interrupted_round_id: String,
    buffered_frames: Vec<PcmFrame>,
    completion_rx: Option<oneshot::Receiver<Result<Vec<SpeechRuntimeEvent>>>>,
}

impl PendingInterruptTransition {
    fn is_waiting_for_remote(&self) -> bool {
        self.completion_rx.is_some()
    }
}

#[derive(Default)]
struct InterruptedOutputTracker {
    round_ids: HashSet<String>,
}

impl InterruptedOutputTracker {
    fn remember(&mut self, round_id: &str) {
        self.round_ids.insert(round_id.to_owned());
    }

    fn contains(&self, round_id: &str) -> bool {
        self.round_ids.contains(round_id)
    }

    fn finish(&mut self, round_id: &str) -> bool {
        self.round_ids.remove(round_id)
    }

    fn should_ignore_orphan_audio(&self) -> bool {
        !self.round_ids.is_empty()
    }

    fn len(&self) -> usize {
        self.round_ids.len()
    }

    fn clear(&mut self) {
        self.round_ids.clear();
    }
}

enum PushInputOutcome {
    Accepted,
    InputClosed,
}

async fn push_input_frame(
    handler: &Arc<dyn SpeechHandler>,
    frame: PcmFrame,
    media_state: SpeechInputMediaState,
) -> Result<PushInputOutcome> {
    match handler.push_frame(frame, media_state).await {
        Ok(()) => Ok(PushInputOutcome::Accepted),
        Err(error) if is_push_input_not_accepting_error(&error) => {
            tracing::debug!(
                error = %error,
                "worker stopped pushing speech input because runtime is no longer accepting input"
            );
            Ok(PushInputOutcome::InputClosed)
        }
        Err(error) => {
            // 推帧失败也不杀整通:重试有 deadline 兜底,丢这帧、继续(与 poll 降级一致)。
            tracing::warn!(
                error = %error,
                "worker push speech frame failed; dropping frame and continuing"
            );
            Ok(PushInputOutcome::Accepted)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerRoute {
    backend_url: String,
    instance_id: String,
    instance_epoch: String,
}

enum BargeInDecision {
    Suppress { reason: &'static str },
    Confirm { prefix_frames: Vec<PcmFrame> },
}

struct BargeInDetector {
    prefix_frames: VecDeque<PcmFrame>,
    prefix_duration_ms: u32,
    max_prefix_ms: u32,
    confirmed_speech_ms: u32,
    last_suppress_log_ms: i64,
}

impl BargeInDetector {
    fn new(max_prefix_ms: u32) -> Self {
        Self {
            prefix_frames: VecDeque::new(),
            prefix_duration_ms: 0,
            max_prefix_ms,
            confirmed_speech_ms: 0,
            last_suppress_log_ms: 0,
        }
    }

    fn observe_bot_turn_frame(
        &mut self,
        processed: &PreprocessedFrame,
        echo_like: bool,
    ) -> BargeInDecision {
        self.push_prefix(processed.frame.clone());
        let frame_ms = frame_duration_ms(&processed.frame);
        if echo_like {
            self.confirmed_speech_ms = 0;
            return BargeInDecision::Suppress {
                reason: "echo_like",
            };
        }
        if !processed.likely_speech {
            self.confirmed_speech_ms = self.confirmed_speech_ms.saturating_sub(frame_ms);
            return BargeInDecision::Suppress {
                reason: "below_voice_gate",
            };
        }

        self.confirmed_speech_ms = self.confirmed_speech_ms.saturating_add(frame_ms);
        if self.confirmed_speech_ms < BARGE_IN_CONFIRM_MS {
            return BargeInDecision::Suppress {
                reason: "voice_not_confirmed",
            };
        }

        let frames = self.prefix_frames.iter().cloned().collect::<Vec<_>>();
        self.reset();
        BargeInDecision::Confirm {
            prefix_frames: frames,
        }
    }

    fn reset(&mut self) {
        self.prefix_frames.clear();
        self.prefix_duration_ms = 0;
        self.confirmed_speech_ms = 0;
    }

    fn log_suppressed(&mut self, reason: &'static str, rms: f32, round_id: &str) {
        let now = now_ms();
        if now.saturating_sub(self.last_suppress_log_ms) < BARGE_IN_SUPPRESS_LOG_INTERVAL_MS {
            return;
        }
        self.last_suppress_log_ms = now;
        tracing::debug!(
            session_round_id = %round_id,
            rms,
            reason,
            confirmed_speech_ms = self.confirmed_speech_ms,
            "worker suppressed mic input during bot turn"
        );
    }

    fn push_prefix(&mut self, frame: PcmFrame) {
        let frame_ms = frame_duration_ms(&frame);
        self.prefix_duration_ms = self.prefix_duration_ms.saturating_add(frame_ms);
        self.prefix_frames.push_back(frame);
        while self.prefix_duration_ms > self.max_prefix_ms {
            let Some(removed) = self.prefix_frames.pop_front() else {
                self.prefix_duration_ms = 0;
                break;
            };
            self.prefix_duration_ms = self
                .prefix_duration_ms
                .saturating_sub(frame_duration_ms(&removed));
        }
    }
}

#[derive(Default)]
struct PlaybackReference {
    samples: VecDeque<i16>,
    max_samples: usize,
    sample_rate_hz: u32,
}

impl PlaybackReference {
    fn push_frame(&mut self, frame: &PcmFrame) {
        if frame.sample_rate == 0 || frame.num_channels == 0 {
            return;
        }
        if self.sample_rate_hz != frame.sample_rate {
            self.samples.clear();
            self.sample_rate_hz = frame.sample_rate;
            self.max_samples = ((u64::from(frame.sample_rate) * 2) as usize).max(1);
        }
        for sample in mono_samples(frame) {
            self.samples.push_back(sample);
        }
        while self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
    }

    fn is_echo_like(&self, mic_frame: &PcmFrame) -> bool {
        if self.sample_rate_hz == 0 || self.samples.is_empty() || mic_frame.sample_rate == 0 {
            return false;
        }
        let mic = mono_samples(mic_frame);
        if mic.is_empty() {
            return false;
        }
        let reference = self.samples.iter().copied().collect::<Vec<_>>();
        let frame_len = mic.len();
        if reference.len() <= frame_len {
            return false;
        }

        let min_delay = delay_samples(self.sample_rate_hz, ECHO_MIN_DELAY_MS);
        let max_delay = delay_samples(self.sample_rate_hz, ECHO_MAX_DELAY_MS);
        let step = delay_samples(self.sample_rate_hz, ECHO_DELAY_STEP_MS).max(1);
        let mut delay = min_delay;
        while delay <= max_delay {
            if reference.len() > delay + frame_len {
                let end = reference.len() - delay;
                let start = end - frame_len;
                if normalized_correlation(&mic, &reference[start..end])
                    >= ECHO_CORRELATION_THRESHOLD
                {
                    return true;
                }
            }
            delay = delay.saturating_add(step);
        }
        false
    }
}

pub struct SpeechPipeline {
    task: Option<JoinHandle<Result<()>>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    drain_task: Option<JoinHandle<()>>,
}

impl SpeechPipeline {
    pub async fn start(
        mut input: StreamingUserAudioInput,
        playback: BotPlayback,
        handler: Arc<dyn SpeechHandler>,
        config: SpeechPipelineConfig,
    ) -> Result<Self> {
        handler
            .start_stream(config.sample_rate_hz, config.num_channels)
            .await?;
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        // 解耦音频拉取:独立 drain 任务持续从 libwebrtc 排空用户音频到有界队列。这样即使主
        // select 循环因某个控制面调用短暂阻塞,也不会停止排空原生音频队列,从而避免
        // "native audio stream queue overflow; dropped frames" 与随之而来的 ASR 空闲断开。
        let (frame_tx, mut frame_rx) =
            tokio::sync::mpsc::channel::<PcmFrame>(INBOUND_AUDIO_QUEUE_CAP);
        let mut drain_shutdown_rx = shutdown_rx.clone();
        let drain_task = tokio::spawn(async move {
            let mut dropped: u64 = 0;
            loop {
                tokio::select! {
                    changed = drain_shutdown_rx.changed() => {
                        match changed {
                            Ok(()) if *drain_shutdown_rx.borrow() => break,
                            Ok(()) => {}
                            Err(_) => break,
                        }
                    }
                    frame = input.next_frame() => {
                        match frame {
                            Ok(Some(frame)) => match frame_tx.try_send(frame) {
                                Ok(()) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                    dropped = dropped.saturating_add(1);
                                    if dropped % INBOUND_DROP_LOG_INTERVAL == 1 {
                                        tracing::warn!(
                                            dropped_total = dropped,
                                            queue_cap = INBOUND_AUDIO_QUEUE_CAP,
                                            "worker inbound audio queue full; dropping newest frame (downstream stalled)"
                                        );
                                    }
                                }
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                            },
                            Ok(None) => break,
                            Err(error) => {
                                tracing::warn!(error = %error, "worker user audio input stream ended");
                                break;
                            }
                        }
                    }
                }
            }
            let _ = input.close();
        });

        let task = tokio::spawn(async move {
            let mut preprocessor = config.preprocessor;
            let sample_rate_hz = config.sample_rate_hz;
            let num_channels = config.num_channels;
            let mut poll_interval = tokio::time::interval(std::time::Duration::from_millis(
                config.speech_poll_interval_ms.max(1),
            ));
            let mut pending_reply: Option<PendingReply> = None;
            let mut pending_non_text_rounds = HashSet::new();
            let playback_lock = Arc::new(Mutex::new(()));
            let active_playback = Arc::new(Mutex::new(None::<ActivePlayback>));
            let input_gate = Arc::new(Mutex::new(InputGate::default()));
            let playback_reference = Arc::new(Mutex::new(PlaybackReference::default()));
            let mut barge_in_detector = BargeInDetector::new(BARGE_IN_PREFIX_MS);
            let mut playback_jobs: JoinSet<Result<()>> = JoinSet::new();
            let mut pending_interrupt: Option<PendingInterruptTransition> = None;
            let mut interrupted_outputs = InterruptedOutputTracker::default();
            let mut input_closed_by_runtime = false;
            let http = HttpClient::new();

            loop {
                if let Some(pending) = pending_interrupt.as_mut()
                    && let Some(completion_rx) = pending.completion_rx.as_mut()
                {
                    match completion_rx.try_recv() {
                        Ok(Ok(interrupt_events)) => {
                            tracing::debug!(
                                buffered_frames = pending.buffered_frames.len(),
                                "worker barge-in transition completed remote interrupt"
                            );
                            publish_round_failed_events(&interrupt_events, &handler).await?;
                            handler
                                .publish_worker_event(
                                    None,
                                    RuntimeFactKind::WorkerBargeInDetected { rms: 0.0 },
                                )
                                .await?;
                            interrupted_outputs.remember(&pending.interrupted_round_id);
                            input_gate.lock().await.open_after_interrupt();
                            for frame in pending.buffered_frames.drain(..) {
                                match push_input_frame(&handler, frame, SpeechInputMediaState::Open)
                                    .await?
                                {
                                    PushInputOutcome::Accepted => {}
                                    PushInputOutcome::InputClosed => {
                                        input_closed_by_runtime = true;
                                        input_gate.lock().await.clear_all();
                                        break;
                                    }
                                }
                            }
                            pending_interrupt = None;
                        }
                        Ok(Err(error)) => return Err(error),
                        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                        Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                            return Err(anyhow!(
                                "worker barge-in transition task dropped before completion"
                            ));
                        }
                    }
                }

                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        match changed {
                            Ok(()) if *shutdown_rx.borrow() => break,
                            Ok(()) => {}
                            Err(_) => break,
                        }
                    }
                    _ = poll_interval.tick() => {
                        if pending_interrupt
                            .as_ref()
                            .is_some_and(PendingInterruptTransition::is_waiting_for_remote)
                        {
                            continue;
                        }
                        let events = match handler.poll_output_events().await {
                            Ok(events) => events,
                            Err(error) => {
                                // poll 失败不再杀死整通(此前 `?` 会让一次瞬时控制面错误终结通话);
                                // 重试已有 deadline 兜底,跳过本轮、下一 tick 再试,通话可自愈。
                                tracing::warn!(
                                    error = %error,
                                    "worker speech poll failed; skipping this tick and continuing"
                                );
                                continue;
                            }
                        };
                        handle_output_events(
                            events,
                            &mut pending_reply,
                            &mut pending_non_text_rounds,
                            &mut interrupted_outputs,
                            playback.clone(),
                            handler.clone(),
                            playback_lock.clone(),
                            active_playback.clone(),
                            input_gate.clone(),
                            playback_reference.clone(),
                            &mut playback_jobs,
                            http.clone(),
                            sample_rate_hz,
                            num_channels,
                        )
                        .await?;
                    }
                    job_result = playback_jobs.join_next(), if !playback_jobs.is_empty() => {
                        if let Some(result) = job_result {
                            handle_playback_job_result(
                                result,
                                &active_playback,
                                &input_gate,
                                &playback_reference,
                            )
                            .await?;
                        }
                    }
                    maybe_frame = frame_rx.recv() => {
                        let Some(frame) = maybe_frame else {
                            break;
                        };
                        let processed = preprocessor.process(frame);

                        if input_closed_by_runtime {
                            barge_in_detector.reset();
                            continue;
                        }

                        if let Some(pending) = pending_interrupt.as_mut() {
                            pending.buffered_frames.push(processed.frame);
                            continue;
                        }

                        // 注意:必须用 let 绑定先取出 owned mode,让锁在此语句结束即释放。
                        // 若写成 `match input_gate.lock().await.mode() { .. }`,scrutinee 的
                        // 临时 guard 会延长到整个 match 块,块内 begin_interrupt_transition /
                        // open_after_interrupt 再次 lock 同一 tokio Mutex 会自死锁(打断后卡死)。
                        let gate_mode = input_gate.lock().await.mode();
                        match gate_mode {
                            InputGateMode::Closed => {
                                barge_in_detector.reset();
                                continue;
                            }
                            InputGateMode::Open => {
                                barge_in_detector.reset();
                                if matches!(
                                    push_input_frame(
                                        &handler,
                                        processed.frame,
                                        SpeechInputMediaState::Open,
                                    )
                                    .await?,
                                    PushInputOutcome::InputClosed
                                ) {
                                    input_closed_by_runtime = true;
                                    input_gate.lock().await.clear_all();
                                }
                            }
                            InputGateMode::OutputTurnPending { .. } => {
                                barge_in_detector.reset();
                                continue;
                            }
                            InputGateMode::BotTurn {
                                round_id,
                                interruptible,
                            } => {
                            if !interruptible {
                                barge_in_detector.reset();
                                barge_in_detector.log_suppressed(
                                    "not_interruptible",
                                    processed.rms,
                                    round_id.as_str(),
                                );
                                continue;
                            }

                            let echo_like = playback_reference
                                .lock()
                                .await
                                .is_echo_like(&processed.frame);
                            let decision =
                                barge_in_detector.observe_bot_turn_frame(&processed, echo_like);
                            match decision {
                                BargeInDecision::Suppress { reason } => {
                                    barge_in_detector.log_suppressed(
                                        reason,
                                        processed.rms,
                                        round_id.as_str(),
                                    );
                                    continue;
                                }
                                BargeInDecision::Confirm { prefix_frames } => {
                                let interrupting_remote_output_turn =
                                    pending_reply.is_some() || !pending_non_text_rounds.is_empty();
                                tracing::debug!(
                                    session_round_id = %round_id,
                                    rms = processed.rms,
                                    interrupting_remote_output_turn,
                                    prefix_frames = prefix_frames.len(),
                                    "worker interrupting active playback because valid user speech was detected"
                                );
                                    let completion_rx = begin_interrupt_transition(
                                    &mut pending_reply,
                                    &mut pending_non_text_rounds,
                                    &playback,
                                    &active_playback,
                                    &input_gate,
                                    &playback_reference,
                                    &mut playback_jobs,
                                    &handler,
                                    &round_id,
                                    interrupting_remote_output_turn,
                                )
                                .await?;
                                let mut buffered_frames = prefix_frames;
                                if !interrupting_remote_output_turn {
                                    handler
                                        .publish_worker_event(
                                            None,
                                            RuntimeFactKind::WorkerBargeInDetected {
                                                rms: f64::from(processed.rms),
                                            },
                                        )
                                        .await?;
                                    input_gate.lock().await.open_after_interrupt();
                                    for frame in buffered_frames.drain(..) {
                                        match push_input_frame(
                                            &handler,
                                            frame,
                                            SpeechInputMediaState::Open,
                                        )
                                        .await?
                                        {
                                            PushInputOutcome::Accepted => {}
                                            PushInputOutcome::InputClosed => {
                                                input_closed_by_runtime = true;
                                                input_gate.lock().await.clear_all();
                                                break;
                                            }
                                        }
                                    }
                                } else {
                                    let interrupted_round_id = round_id.clone();
                                    pending_interrupt = Some(PendingInterruptTransition {
                                        interrupted_round_id,
                                        buffered_frames,
                                        completion_rx: Some(completion_rx.expect("remote interrupt receiver must exist")),
                                    });
                                }
                                continue;
                                }
                            }
                        }
                        }
                    }
                }
            }

            let close_events = handler.close_stream().await?;
            handle_output_events(
                close_events,
                &mut pending_reply,
                &mut pending_non_text_rounds,
                &mut interrupted_outputs,
                playback.clone(),
                handler.clone(),
                playback_lock.clone(),
                active_playback.clone(),
                input_gate.clone(),
                playback_reference.clone(),
                &mut playback_jobs,
                http,
                sample_rate_hz,
                num_channels,
            )
            .await?;

            while let Some(job_result) = playback_jobs.join_next().await {
                handle_playback_job_result(
                    job_result,
                    &active_playback,
                    &input_gate,
                    &playback_reference,
                )
                .await?;
            }

            // input 现由 drain_task 拥有并在其结束时 close;此处只关 playback。
            playback.close().await?;
            Ok(())
        });

        Ok(Self {
            task: Some(task),
            shutdown_tx: Some(shutdown_tx),
            drain_task: Some(drain_task),
        })
    }

    pub async fn poll_completion(&mut self) -> Result<Option<Result<()>>> {
        let Some(task) = self.task.as_mut() else {
            return Ok(None);
        };

        if !task.is_finished() {
            return Ok(None);
        }

        let Some(task) = self.task.take() else {
            return Ok(None);
        };
        // 主任务已结束(其持有的 frame_rx 被 drop)→ drain_task 的 try_send 得到 Closed 而退出;
        // dropping shutdown_tx 也会让其 changed() 返回 Err 而退出。优雅收尾让 drain 跑完 input.close()。
        self.shutdown_tx.take();
        if let Some(drain_task) = self.drain_task.take() {
            shutdown_drain_task(drain_task).await;
        }
        let result = match task.await {
            Ok(result) => result,
            Err(join_error) => Err(anyhow!("speech pipeline task failed: {join_error}")),
        };
        Ok(Some(result))
    }

    pub async fn stop(mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }
        // drain_task 收到 shutdown 会 break 并执行 input.close();优雅 await,超时才 abort 兜底。
        if let Some(drain_task) = self.drain_task.take() {
            shutdown_drain_task(drain_task).await;
        }

        let Some(task) = self.task.take() else {
            return Ok(());
        };
        let mut task = Box::pin(task);
        tokio::select! {
            result = &mut task => {
                match result {
                    Ok(result) => result,
                    Err(join_error) => Err(anyhow!(
                        "speech pipeline task failed while stopping: {join_error}"
                    )),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(PIPELINE_STOP_GRACE_MS)) => {
                tracing::warn!(
                    stop_grace_ms = PIPELINE_STOP_GRACE_MS,
                    "speech pipeline stop exceeded grace period; aborting task"
                );
                task.as_ref().get_ref().abort();
                match task.await {
                    Ok(result) => result,
                    Err(join_error) if join_error.is_cancelled() => Ok(()),
                    Err(join_error) => Err(anyhow!(
                        "speech pipeline task failed after forced abort: {join_error}"
                    )),
                }
            }
        }
    }
}

/// 优雅停止音频 drain 任务:它收到 shutdown(或 sender/receiver 被 drop)后会 break 并执行
/// `input.close()` 释放底层 NativeAudioStream;给一段宽限优雅 await,仅超时才 abort 兜底。
async fn shutdown_drain_task(handle: JoinHandle<()>) {
    let mut handle = Box::pin(handle);
    tokio::select! {
        _ = &mut handle => {}
        _ = tokio::time::sleep(std::time::Duration::from_millis(PIPELINE_STOP_GRACE_MS)) => {
            tracing::warn!(
                stop_grace_ms = PIPELINE_STOP_GRACE_MS,
                "worker audio drain task did not stop within grace period; aborting"
            );
            handle.as_ref().get_ref().abort();
            let _ = handle.await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_output_events(
    events: Vec<SpeechRuntimeEvent>,
    pending_reply: &mut Option<PendingReply>,
    pending_non_text_rounds: &mut HashSet<String>,
    interrupted_outputs: &mut InterruptedOutputTracker,
    playback: BotPlayback,
    handler: Arc<dyn SpeechHandler>,
    playback_lock: Arc<Mutex<()>>,
    active_playback: Arc<Mutex<Option<ActivePlayback>>>,
    input_gate: Arc<Mutex<InputGate>>,
    playback_reference: Arc<Mutex<PlaybackReference>>,
    playback_jobs: &mut JoinSet<Result<()>>,
    http: HttpClient,
    sample_rate_hz: u32,
    num_channels: u16,
) -> Result<()> {
    publish_round_failed_events(&events, &handler).await?;

    if let Some(message) = events.iter().find_map(|event| match event {
        SpeechRuntimeEvent::SessionFailed { message } => Some(message.clone()),
        _ => None,
    }) {
        interrupted_outputs.clear();
        input_gate.lock().await.clear_all();
        stop_all_pending_playback(
            pending_reply,
            pending_non_text_rounds,
            &playback,
            &active_playback,
            &input_gate,
            &playback_reference,
            playback_jobs,
        )
        .await;
        return Err(anyhow!("speech runtime session failed: {message}"));
    }
    if events
        .iter()
        .any(|event| matches!(event, SpeechRuntimeEvent::SessionClosing))
    {
        tracing::debug!("worker received speech runtime session closing");
        interrupted_outputs.clear();
        input_gate.lock().await.clear_all();
        stop_all_pending_playback(
            pending_reply,
            pending_non_text_rounds,
            &playback,
            &active_playback,
            &input_gate,
            &playback_reference,
            playback_jobs,
        )
        .await;
        return Ok(());
    }

    for event in events {
        input_gate.lock().await.apply_event(&event);
        match event {
            SpeechRuntimeEvent::ListeningStarted => {
                tracing::debug!("worker received speech runtime listening started");
            }
            SpeechRuntimeEvent::SpeechDetected => {
                tracing::debug!("worker received speech runtime speech detected");
            }
            SpeechRuntimeEvent::UtteranceFlushing => {
                tracing::debug!("worker received speech runtime utterance flushing");
            }
            SpeechRuntimeEvent::RespondingStarted => {
                tracing::debug!("worker received speech runtime responding started");
            }
            SpeechRuntimeEvent::SessionClosing => {
                tracing::debug!("worker received speech runtime session closing");
                interrupted_outputs.clear();
                stop_all_pending_playback(
                    pending_reply,
                    pending_non_text_rounds,
                    &playback,
                    &active_playback,
                    &input_gate,
                    &playback_reference,
                    playback_jobs,
                )
                .await;
            }
            SpeechRuntimeEvent::SessionFailed { message } => {
                interrupted_outputs.clear();
                stop_all_pending_playback(
                    pending_reply,
                    pending_non_text_rounds,
                    &playback,
                    &active_playback,
                    &input_gate,
                    &playback_reference,
                    playback_jobs,
                )
                .await;
                return Err(anyhow!("speech runtime session failed: {message}"));
            }
            SpeechRuntimeEvent::Warning { message } => {
                tracing::debug!(message, "worker received speech runtime warning");
            }
            SpeechRuntimeEvent::RoundFailed { round_id, .. } => {
                interrupted_outputs.finish(&round_id);
            }
            SpeechRuntimeEvent::ReplyStarted {
                round_id,
                reply_text,
                interruptible,
            } => {
                if interrupted_outputs.contains(&round_id) {
                    tracing::debug!(
                        session_round_id = %round_id,
                        "worker ignored reply_started for interrupted output"
                    );
                    continue;
                }
                if pending_reply.is_some() {
                    return Err(anyhow!(
                        "speech runtime emitted nested reply_started before reply_finished"
                    ));
                }
                *pending_reply = Some(PendingReply {
                    round_id,
                    reply_text,
                    interruptible,
                    frames_tx: None,
                    playback_started: false,
                });
            }
            SpeechRuntimeEvent::AudioChunk {
                pcm_s16le,
                sample_rate_hz,
                channels,
            } => {
                let Some(pending) = pending_reply.as_mut() else {
                    if interrupted_outputs.should_ignore_orphan_audio() {
                        tracing::debug!(
                            interrupted_round_count = interrupted_outputs.len(),
                            "worker ignored orphan audio_chunk after interrupted output"
                        );
                        continue;
                    }
                    return Err(anyhow!("speech runtime emitted audio_chunk without reply"));
                };
                let channels = usize::from(channels.max(1));
                if pcm_s16le.len() % channels != 0 {
                    return Err(anyhow!(
                        "speech runtime returned malformed pcm length {} for {} channels",
                        pcm_s16le.len(),
                        channels
                    ));
                }
                let samples_per_channel = (pcm_s16le.len() / channels) as u32;
                let frame = PcmFrame::new(
                    pcm_s16le,
                    sample_rate_hz,
                    channels as u32,
                    samples_per_channel,
                );
                if pending.frames_tx.is_none() {
                    let (frames_tx, frames_rx) = unbounded_channel();
                    playback_jobs.spawn(run_streaming_text_playback_job(
                        playback.clone(),
                        handler.clone(),
                        playback_lock.clone(),
                        active_playback.clone(),
                        input_gate.clone(),
                        playback_reference.clone(),
                        pending.round_id.clone(),
                        pending.reply_text.clone(),
                        pending.interruptible,
                        frames_rx,
                    ));
                    pending.frames_tx = Some(frames_tx);
                    pending.playback_started = true;
                }
                let Some(frames_tx) = pending.frames_tx.as_ref() else {
                    return Err(anyhow!("speech runtime audio stream was not initialized"));
                };
                frames_tx
                    .send(frame)
                    .map_err(|_| anyhow!("worker streaming playback task closed early"))?;
            }
            SpeechRuntimeEvent::ReplyFinished { round_id } => {
                if let Some(pending) = pending_reply.take() {
                    if pending.round_id != round_id {
                        return Err(anyhow!(
                            "speech runtime reply_finished round mismatch: expected {}, got {}",
                            pending.round_id,
                            round_id
                        ));
                    }
                    if !pending.playback_started {
                        return Err(anyhow!("speech runtime reply finished without audio_chunk"));
                    }
                    handler
                        .publish_worker_event(
                            Some(&round_id),
                            RuntimeFactKind::RuntimeReplyFinished,
                        )
                        .await?;
                    continue;
                }
                if pending_non_text_rounds.remove(&round_id) {
                    handler
                        .publish_worker_event(
                            Some(&round_id),
                            RuntimeFactKind::RuntimeReplyFinished,
                        )
                        .await?;
                } else if interrupted_outputs.finish(&round_id) {
                    tracing::debug!(
                        session_round_id = %round_id,
                        "worker ignored reply_finished for interrupted output"
                    );
                } else {
                    return Err(anyhow!(
                        "speech runtime emitted reply_finished without matching turn state"
                    ));
                }
            }
            SpeechRuntimeEvent::PreRecordedAsset {
                round_id,
                audio_url,
                band,
                interruptible,
            } => {
                playback_jobs.spawn(run_pre_recorded_playback_job(
                    playback.clone(),
                    handler.clone(),
                    playback_lock.clone(),
                    active_playback.clone(),
                    http.clone(),
                    input_gate.clone(),
                    playback_reference.clone(),
                    round_id,
                    audio_url,
                    band,
                    interruptible,
                    sample_rate_hz,
                    num_channels,
                ));
            }
        }
    }
    Ok(())
}

async fn publish_round_failed_events(
    events: &[SpeechRuntimeEvent],
    handler: &Arc<dyn SpeechHandler>,
) -> Result<()> {
    for event in events {
        let SpeechRuntimeEvent::RoundFailed { round_id, message } = event else {
            continue;
        };
        tracing::warn!(
            session_round_id = %round_id,
            reason = %message,
            "worker received speech runtime input round failure"
        );
        handler
            .publish_worker_event(
                Some(round_id),
                RuntimeFactKind::RuntimeInputRoundFailed {
                    reason: message.clone(),
                },
            )
            .await?;
    }
    Ok(())
}

async fn handle_playback_job_result(
    job_result: Result<Result<()>, JoinError>,
    active_playback: &Arc<Mutex<Option<ActivePlayback>>>,
    input_gate: &Arc<Mutex<InputGate>>,
    playback_reference: &Arc<Mutex<PlaybackReference>>,
) -> Result<()> {
    match job_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error),
        Err(error) if error.is_cancelled() => {
            tracing::warn!(
                error = %error,
                "worker observed cancelled playback job; clearing local playback gate"
            );
            clear_any_active_playback(active_playback, playback_reference).await;
            input_gate.lock().await.open_after_interrupt();
            Ok(())
        }
        Err(error) => Err(anyhow!("worker playback job failed: {error}")),
    }
}

async fn stop_all_pending_playback(
    pending_reply: &mut Option<PendingReply>,
    pending_non_text_rounds: &mut HashSet<String>,
    playback: &BotPlayback,
    active_playback: &Arc<Mutex<Option<ActivePlayback>>>,
    input_gate: &Arc<Mutex<InputGate>>,
    playback_reference: &Arc<Mutex<PlaybackReference>>,
    playback_jobs: &mut JoinSet<Result<()>>,
) {
    pending_reply.take();
    pending_non_text_rounds.clear();
    playback.interrupt_speech();
    playback_jobs.abort_all();
    if let Err(error) = drain_playback_jobs_after_abort(playback_jobs, "speech session stop").await
    {
        tracing::warn!(
            error = %error,
            "worker failed to drain playback jobs after stop"
        );
    }
    clear_any_active_playback(active_playback, playback_reference).await;
    input_gate.lock().await.clear_all();
}

async fn drain_playback_jobs_after_abort(
    playback_jobs: &mut JoinSet<Result<()>>,
    context: &'static str,
) -> Result<()> {
    while let Some(job_result) = playback_jobs.join_next().await {
        match job_result {
            Ok(Ok(())) => {
                tracing::debug!(
                    context,
                    "worker observed playback job completion after abort"
                );
            }
            Ok(Err(error)) => {
                return Err(anyhow!(
                    "worker playback job failed after abort ({context}): {error}"
                ));
            }
            Err(error) if error.is_cancelled() => {
                tracing::debug!(
                    context,
                    "worker observed expected playback job cancellation after abort"
                );
            }
            Err(error) => {
                return Err(anyhow!(
                    "worker playback job join failed after abort ({context}): {error}"
                ));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn begin_interrupt_transition(
    pending_reply: &mut Option<PendingReply>,
    pending_non_text_rounds: &mut HashSet<String>,
    playback: &BotPlayback,
    active_playback: &Arc<Mutex<Option<ActivePlayback>>>,
    input_gate: &Arc<Mutex<InputGate>>,
    playback_reference: &Arc<Mutex<PlaybackReference>>,
    playback_jobs: &mut JoinSet<Result<()>>,
    handler: &Arc<dyn SpeechHandler>,
    round_id: &str,
    interrupt_remote_output_turn: bool,
) -> Result<Option<oneshot::Receiver<Result<Vec<SpeechRuntimeEvent>>>>> {
    tracing::debug!(
        session_round_id = %round_id,
        interrupt_remote_output_turn,
        pending_reply_present = pending_reply.is_some(),
        pending_non_text_round_count = pending_non_text_rounds.len(),
        "worker barge-in transition: begin"
    );

    pending_reply.take();
    pending_non_text_rounds.clear();

    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: invalidate current playback epoch"
    );
    playback.interrupt_speech();

    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: abort playback jobs"
    );
    playback_jobs.abort_all();
    drain_playback_jobs_after_abort(playback_jobs, "barge-in interrupt").await?;

    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: clear active playback"
    );
    clear_any_active_playback(active_playback, playback_reference).await;
    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: clear input gate output turn"
    );
    input_gate.lock().await.clear_output_turn(round_id);
    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: clear native output buffer after abort"
    );
    playback.interrupt_speech();

    if interrupt_remote_output_turn {
        let handler = handler.clone();
        let round_id = round_id.to_owned();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            tracing::debug!(
                session_round_id = %round_id,
                "worker barge-in transition: interrupt remote speech stream"
            );
            let result = handler.interrupt_stream().await;
            match &result {
                Ok(interrupt_events) => {
                    tracing::debug!(
                        session_round_id = %round_id,
                        interrupt_event_count = interrupt_events.len(),
                        "worker barge-in transition: remote speech stream interrupted"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        session_round_id = %round_id,
                        error = %error,
                        "worker barge-in transition: remote speech stream interrupt failed"
                    );
                }
            }
            let _ = tx.send(result);
        });
        return Ok(Some(rx));
    }

    tracing::debug!(
        session_round_id = %round_id,
        "worker barge-in transition: completed"
    );
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
async fn run_streaming_text_playback_job(
    playback: BotPlayback,
    handler: Arc<dyn SpeechHandler>,
    playback_lock: Arc<Mutex<()>>,
    active_playback: Arc<Mutex<Option<ActivePlayback>>>,
    input_gate: Arc<Mutex<InputGate>>,
    playback_reference: Arc<Mutex<PlaybackReference>>,
    round_id: String,
    reply_text: String,
    interruptible: bool,
    mut frames_rx: tokio::sync::mpsc::UnboundedReceiver<PcmFrame>,
) -> Result<()> {
    let _guard = playback_lock.lock().await;
    let playback_epoch = playback.current_epoch();

    let mut started = false;
    while let Some(frame) = frames_rx.recv().await {
        if !playback.is_current_epoch(playback_epoch) {
            clear_active_playback(&active_playback, &playback_reference, &round_id).await;
            input_gate.lock().await.clear_output_turn(&round_id);
            return Ok(());
        }
        if let Err(error) = playback.play_speech_frame(frame.clone()).await {
            clear_active_playback(&active_playback, &playback_reference, &round_id).await;
            input_gate.lock().await.clear_output_turn(&round_id);
            return Err(error);
        }
        playback_reference.lock().await.push_frame(&frame);
        if !started {
            set_active_playback(&active_playback, &round_id).await;
            input_gate
                .lock()
                .await
                .start_bot_playback(&round_id, interruptible);
            handler
                .publish_worker_event(
                    Some(&round_id),
                    RuntimeFactKind::RuntimeReplyStarted {
                        reply_text: reply_text.clone(),
                    },
                )
                .await?;
            started = true;
        }
    }
    if !started {
        clear_active_playback(&active_playback, &playback_reference, &round_id).await;
        input_gate.lock().await.clear_output_turn(&round_id);
        return Err(anyhow!("speech runtime reply produced no playable frames"));
    }
    playback.wait_until_drained().await?;

    let completed_current_epoch = playback.is_current_epoch(playback_epoch);
    clear_active_playback(&active_playback, &playback_reference, &round_id).await;
    input_gate.lock().await.clear_output_turn(&round_id);

    if completed_current_epoch {
        handler
            .publish_worker_event(Some(&round_id), RuntimeFactKind::RuntimePlaybackCompleted)
            .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_pre_recorded_playback_job(
    playback: BotPlayback,
    handler: Arc<dyn SpeechHandler>,
    playback_lock: Arc<Mutex<()>>,
    active_playback: Arc<Mutex<Option<ActivePlayback>>>,
    http: HttpClient,
    input_gate: Arc<Mutex<InputGate>>,
    playback_reference: Arc<Mutex<PlaybackReference>>,
    round_id: String,
    audio_url: String,
    band: String,
    interruptible: bool,
    sample_rate_hz: u32,
    num_channels: u16,
) -> Result<()> {
    let frames = load_pre_recorded_frames(&http, &audio_url, sample_rate_hz, num_channels).await?;
    let _guard = playback_lock.lock().await;
    let playback_epoch = playback.current_epoch();
    let mut started = false;
    for frame in frames {
        if !playback.is_current_epoch(playback_epoch) {
            clear_active_playback(&active_playback, &playback_reference, &round_id).await;
            input_gate.lock().await.clear_output_turn(&round_id);
            return Ok(());
        }
        if let Err(error) = playback.play_pre_recorded_frame(frame.clone()).await {
            clear_active_playback(&active_playback, &playback_reference, &round_id).await;
            input_gate.lock().await.clear_output_turn(&round_id);
            return Err(error);
        }
        playback_reference.lock().await.push_frame(&frame);
        if !started {
            set_active_playback(&active_playback, &round_id).await;
            input_gate
                .lock()
                .await
                .start_bot_playback(&round_id, interruptible);
            handler
                .publish_worker_event(
                    Some(&round_id),
                    RuntimeFactKind::ExternalAudioStarted {
                        audio_url: audio_url.clone(),
                        band: band.clone(),
                    },
                )
                .await?;
            started = true;
        }
    }
    if !started {
        clear_active_playback(&active_playback, &playback_reference, &round_id).await;
        input_gate.lock().await.clear_output_turn(&round_id);
        return Err(anyhow!("pre-recorded audio produced no playable frames"));
    }
    playback.wait_until_drained().await?;

    let completed_current_epoch = playback.is_current_epoch(playback_epoch);
    clear_active_playback(&active_playback, &playback_reference, &round_id).await;
    input_gate.lock().await.clear_output_turn(&round_id);

    if completed_current_epoch {
        handler
            .publish_worker_event(
                Some(&round_id),
                RuntimeFactKind::ExternalAudioFinished { audio_url, band },
            )
            .await?;
    }
    Ok(())
}

async fn set_active_playback(active_playback: &Arc<Mutex<Option<ActivePlayback>>>, round_id: &str) {
    let mut active = active_playback.lock().await;
    *active = Some(ActivePlayback {
        round_id: round_id.to_owned(),
    });
}

async fn clear_active_playback(
    active_playback: &Arc<Mutex<Option<ActivePlayback>>>,
    playback_reference: &Arc<Mutex<PlaybackReference>>,
    round_id: &str,
) {
    let cleared = {
        let mut active = active_playback.lock().await;
        if active
            .as_ref()
            .is_some_and(|current| current.round_id == round_id)
        {
            *active = None;
            true
        } else {
            false
        }
    };
    if cleared {
        playback_reference.lock().await.clear();
    }
}

async fn clear_any_active_playback(
    active_playback: &Arc<Mutex<Option<ActivePlayback>>>,
    playback_reference: &Arc<Mutex<PlaybackReference>>,
) {
    *active_playback.lock().await = None;
    playback_reference.lock().await.clear();
}

fn frame_duration_ms(frame: &PcmFrame) -> u32 {
    if frame.sample_rate == 0 {
        return 0;
    }
    ((u64::from(frame.samples_per_channel) * 1_000) / u64::from(frame.sample_rate)) as u32
}

fn delay_samples(sample_rate_hz: u32, delay_ms: u32) -> usize {
    ((u64::from(sample_rate_hz) * u64::from(delay_ms)) / 1_000) as usize
}

fn mono_samples(frame: &PcmFrame) -> Vec<i16> {
    let channels = frame.num_channels.max(1) as usize;
    if channels == 1 {
        return frame.data.clone();
    }
    frame
        .data
        .chunks(channels)
        .map(|samples| {
            let sum = samples.iter().map(|sample| i32::from(*sample)).sum::<i32>();
            (sum / samples.len() as i32).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        })
        .collect()
}

fn normalized_correlation(left: &[i16], right: &[i16]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut left_energy = 0.0_f64;
    let mut right_energy = 0.0_f64;
    for (left, right) in left.iter().zip(right.iter()) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot += left * right;
        left_energy += left * left;
        right_energy += right * right;
    }
    if left_energy <= f64::EPSILON || right_energy <= f64::EPSILON {
        return 0.0;
    }
    (dot.abs() / (left_energy.sqrt() * right_energy.sqrt())) as f32
}

fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(error) => -(error.duration().as_millis() as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_gate_starts_open_and_listening_started_keeps_it_open() {
        let mut gate = InputGate::default();

        assert_eq!(gate.mode(), InputGateMode::Open);

        gate.apply_event(&SpeechRuntimeEvent::ListeningStarted);

        assert_eq!(gate.mode(), InputGateMode::Open);
    }

    #[test]
    fn input_gate_responding_started_blocks_input_until_output_identity_arrives() {
        let mut gate = InputGate::default();

        gate.apply_event(&SpeechRuntimeEvent::RespondingStarted);

        assert_eq!(
            gate.mode(),
            InputGateMode::OutputTurnPending { round_id: None }
        );
    }

    #[test]
    fn input_gate_reply_started_enters_pending_output_turn() {
        let mut gate = InputGate::default();

        gate.apply_event(&SpeechRuntimeEvent::ReplyStarted {
            round_id: "round-1".to_owned(),
            reply_text: "hello".to_owned(),
            interruptible: true,
        });

        assert_eq!(
            gate.mode(),
            InputGateMode::OutputTurnPending {
                round_id: Some("round-1".to_owned()),
            }
        );
    }

    #[test]
    fn input_gate_keeps_bot_playback_closed_until_local_playback_clears_it() {
        let mut gate = InputGate::default();
        gate.apply_event(&SpeechRuntimeEvent::ReplyStarted {
            round_id: "round-1".to_owned(),
            reply_text: "hello".to_owned(),
            interruptible: true,
        });
        gate.start_bot_playback("round-1", true);

        gate.apply_event(&SpeechRuntimeEvent::ReplyFinished {
            round_id: "round-1".to_owned(),
        });
        gate.apply_event(&SpeechRuntimeEvent::ListeningStarted);
        assert_eq!(
            gate.mode(),
            InputGateMode::BotTurn {
                round_id: "round-1".to_owned(),
                interruptible: true,
            }
        );

        gate.clear_bot_playback("round-1");

        assert_eq!(gate.mode(), InputGateMode::Open);
    }

    #[test]
    fn input_gate_does_not_clear_newer_bot_turn_with_stale_round_id() {
        let mut gate = InputGate::default();
        gate.apply_event(&SpeechRuntimeEvent::ListeningStarted);
        gate.start_bot_playback("round-2", false);

        gate.clear_bot_playback("round-1");

        assert_eq!(
            gate.mode(),
            InputGateMode::BotTurn {
                round_id: "round-2".to_owned(),
                interruptible: false,
            }
        );
    }

    #[test]
    fn input_gate_uses_pre_recorded_asset_as_bot_turn() {
        let mut gate = InputGate::default();

        gate.apply_event(&SpeechRuntimeEvent::RespondingStarted);

        gate.apply_event(&SpeechRuntimeEvent::PreRecordedAsset {
            round_id: "asset-1".to_owned(),
            audio_url: "http://example.test/audio.pcm".to_owned(),
            band: "voice".to_owned(),
            interruptible: true,
        });

        assert_eq!(
            gate.mode(),
            InputGateMode::OutputTurnPending {
                round_id: Some("asset-1".to_owned()),
            }
        );

        gate.start_bot_playback("asset-1", true);

        assert_eq!(
            gate.mode(),
            InputGateMode::BotTurn {
                round_id: "asset-1".to_owned(),
                interruptible: true,
            }
        );

        gate.clear_bot_playback("asset-1");

        assert_eq!(gate.mode(), InputGateMode::Open);
    }

    #[test]
    fn input_gate_closes_terminal_session_events() {
        let mut gate = InputGate {
            pending_output: Some(PendingOutputTurn {
                round_id: Some("round-1".to_owned()),
            }),
            bot_playback: Some(BotTurnGate {
                round_id: "round-1".to_owned(),
                interruptible: true,
            }),
            terminal: false,
        };

        gate.apply_event(&SpeechRuntimeEvent::SessionFailed {
            message: "failed".to_owned(),
        });

        assert_eq!(gate.mode(), InputGateMode::Closed);
    }

    #[test]
    fn input_gate_round_failed_clears_pending_output_turn() {
        let mut gate = InputGate::default();
        gate.apply_event(&SpeechRuntimeEvent::ReplyStarted {
            round_id: "round-1".to_owned(),
            reply_text: "hello".to_owned(),
            interruptible: true,
        });

        gate.apply_event(&SpeechRuntimeEvent::RoundFailed {
            round_id: "round-1".to_owned(),
            message: "failed".to_owned(),
        });

        assert_eq!(gate.mode(), InputGateMode::Open);
    }

    #[test]
    fn input_gate_reply_finished_does_not_open_bot_playback() {
        let mut gate = InputGate::default();
        gate.apply_event(&SpeechRuntimeEvent::ReplyStarted {
            round_id: "round-1".to_owned(),
            reply_text: "hello".to_owned(),
            interruptible: true,
        });
        gate.start_bot_playback("round-1", true);

        gate.apply_event(&SpeechRuntimeEvent::ReplyFinished {
            round_id: "round-1".to_owned(),
        });

        assert_eq!(
            gate.mode(),
            InputGateMode::BotTurn {
                round_id: "round-1".to_owned(),
                interruptible: true,
            }
        );
    }

    #[test]
    fn interrupted_output_tracker_ignores_orphan_audio_until_matching_finish() {
        let mut tracker = InterruptedOutputTracker::default();

        assert!(!tracker.should_ignore_orphan_audio());

        tracker.remember("round-1");

        assert!(tracker.contains("round-1"));
        assert!(tracker.should_ignore_orphan_audio());
        assert!(!tracker.finish("round-2"));
        assert!(tracker.should_ignore_orphan_audio());
        assert!(tracker.finish("round-1"));
        assert!(!tracker.should_ignore_orphan_audio());
    }

    #[test]
    fn interrupted_output_tracker_clear_restores_strict_mode() {
        let mut tracker = InterruptedOutputTracker::default();

        tracker.remember("round-1");
        tracker.remember("round-2");
        tracker.clear();

        assert_eq!(tracker.len(), 0);
        assert!(!tracker.should_ignore_orphan_audio());
    }

    #[tokio::test]
    async fn drain_playback_jobs_after_abort_ignores_cancelled_jobs() -> Result<()> {
        let mut playback_jobs = JoinSet::new();
        playback_jobs.spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok(())
        });

        playback_jobs.abort_all();
        drain_playback_jobs_after_abort(&mut playback_jobs, "test").await?;

        assert!(playback_jobs.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn drain_playback_jobs_after_abort_keeps_real_job_errors() {
        let mut playback_jobs = JoinSet::new();
        playback_jobs.spawn(async { Err(anyhow!("playback failed")) });

        let error = drain_playback_jobs_after_abort(&mut playback_jobs, "test")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("playback failed"));
        assert!(playback_jobs.is_empty());
    }

    #[tokio::test]
    async fn playback_job_result_ignores_cancelled_jobs_and_reopens_gate() -> Result<()> {
        let mut playback_jobs = JoinSet::new();
        playback_jobs.spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok(())
        });

        let active_playback = Arc::new(Mutex::new(Some(ActivePlayback {
            round_id: "round-1".to_owned(),
        })));
        let input_gate = Arc::new(Mutex::new(InputGate::default()));
        input_gate.lock().await.start_bot_playback("round-1", true);
        let playback_reference = Arc::new(Mutex::new(PlaybackReference::default()));

        playback_jobs.abort_all();
        let job_result = playback_jobs
            .join_next()
            .await
            .expect("playback job result");

        handle_playback_job_result(
            job_result,
            &active_playback,
            &input_gate,
            &playback_reference,
        )
        .await?;

        assert!(active_playback.lock().await.is_none());
        assert_eq!(input_gate.lock().await.mode(), InputGateMode::Open);
        assert!(playback_jobs.is_empty());
        Ok(())
    }
}
