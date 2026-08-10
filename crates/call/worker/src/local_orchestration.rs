//! 进程内编排装配:worker 进程内运行 speech-runtime,消除每帧音频 HTTP(卡顿根因)。
//!
//! `LocalSpeechHandler` 实现 worker 的 `SpeechHandler`,把每个方法映射到进程内
//! `SpeechRuntimeUseCases`(微秒级,无 HTTP/轮询争用)。媒体(VAD/ASR/TTS)在 worker 本地;
//! agent(LLM+历史+prompt+usage,重度 DB)经 per-turn HTTP 留 backend(决策 F)。
//! `launch.speech` 为 Some 时启用此路径,否则回退旧的 RemoteSpeechHandler。

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use call_runtime_control::SpeechRuntimeBootstrap;
use shared_kernel::AppResult;
use speech_runtime::{
    CloseSpeechSessionCommand, CreateSpeechSessionCommand, InMemorySpeechSessionStore,
    InterruptSpeechSessionCommand, PollSpeechEventsCommand, PushSpeechInputCommand,
    SpeechRuntimeContext, SpeechRuntimeContextPort, SpeechRuntimeDependencies,
    SpeechRuntimeService, SpeechRuntimeUseCases, SpeechSegmentationConfig,
    SpeechSegmentationConfigPort, SubmitServerTurnCommand, ThresholdSpeechSegmentationPolicy,
};
use tokio::sync::Mutex;

use crate::agent_client::LocalAgentTurnAdapter;
use crate::client::{RuntimeControlClient, SpeechRuntimeEvent};
use crate::config::WorkerConfig;
use crate::pipeline::SpeechHandler;
use crate::speech_events::LocalSpeechEventsAdapter;
use call_runtime_control::{RuntimeEventFact, RuntimeFactKind, SpeechInputMediaState};
use rtc::livekit::pcm::PcmFrame;

const POLL_MAX_EVENTS: usize = 32;

/// 进程内固定上下文:由 dispatch 下发的 bootstrap 提供,无 DB 解析。
#[derive(Clone)]
struct StaticRuntimeContext {
    context: SpeechRuntimeContext,
}

#[async_trait]
impl SpeechRuntimeContextPort for StaticRuntimeContext {
    async fn resolve_accepting_runtime_context(
        &self,
        _session_id: i64,
        _runtime_owner_id: &str,
    ) -> AppResult<Option<SpeechRuntimeContext>> {
        Ok(Some(self.context.clone()))
    }
}

/// 进程内固定分段配置:由 bootstrap 下发。
#[derive(Clone)]
struct StaticSegmentationConfig {
    config: SpeechSegmentationConfig,
}

#[async_trait]
impl SpeechSegmentationConfigPort for StaticSegmentationConfig {
    async fn get_speech_segmentation_config(&self) -> AppResult<SpeechSegmentationConfig> {
        Ok(self.config.clone())
    }
}

/// worker 进程内 SpeechHandler:把 SpeechHandler 各方法映射到进程内 SpeechRuntimeUseCases。
pub struct LocalSpeechHandler {
    service: Arc<dyn SpeechRuntimeUseCases>,
    client: RuntimeControlClient,
    session_id: i64,
    runtime_owner_id: String,
    speech_session_id: Mutex<Option<String>>,
    // 进程内编排:起会话后自取开场欢迎语并提交 server-initiated turn(LiveKit 标准:agent 先问候)。
    agent_session_id: String,
    welcome_agent: LocalAgentTurnAdapter,
}

impl LocalSpeechHandler {
    async fn require_speech_session_id(&self) -> Result<String> {
        self.speech_session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("speech session not started for session {}", self.session_id))
    }

    /// 起会话后触发开场欢迎语(server-initiated turn)。best-effort:失败仅告警,不阻断通话。
    async fn trigger_welcome(&self, speech_session_id: &str) {
        let text = match self
            .welcome_agent
            .generate_welcome(&self.agent_session_id)
            .await
        {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!(reason = %error, "进程内编排:取开场欢迎语失败,跳过");
                return;
            }
        };
        if text.trim().is_empty() {
            return;
        }
        if let Err(error) = self
            .service
            .submit_server_turn(SubmitServerTurnCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: self.runtime_owner_id.clone(),
                round_id: format!("welcome-{}", self.session_id),
                reply_text: text,
                interruptible: true,
            })
            .await
        {
            tracing::warn!(reason = %error, "进程内编排:提交开场欢迎语 server turn 失败");
        }
    }
}

#[async_trait]
impl SpeechHandler for LocalSpeechHandler {
    async fn ensure_available(&self) -> Result<()> {
        Ok(())
    }

    async fn start_stream(&self, sample_rate_hz: u32, num_channels: u16) -> Result<()> {
        let result = self
            .service
            .create_session(CreateSpeechSessionCommand {
                session_id: self.session_id,
                runtime_owner_id: self.runtime_owner_id.clone(),
                sample_rate_hz,
                num_channels,
            })
            .await
            .map_err(to_anyhow)?;
        let speech_session_id = result.speech_session_id;
        *self.speech_session_id.lock().await = Some(speech_session_id.clone());
        // 起会话后即触发开场欢迎语(server-initiated turn);best-effort,不阻断通话建立。
        self.trigger_welcome(&speech_session_id).await;
        Ok(())
    }

    async fn push_frame(&self, frame: PcmFrame, media_state: SpeechInputMediaState) -> Result<()> {
        let speech_session_id = self.require_speech_session_id().await?;
        self.service
            .push_input_audio(PushSpeechInputCommand {
                speech_session_id,
                runtime_owner_id: self.runtime_owner_id.clone(),
                pcm_s16le: frame.data,
                sample_rate_hz: frame.sample_rate,
                num_channels: frame.num_channels as u16,
                media_state,
            })
            .await
            .map_err(to_anyhow)
    }

    async fn poll_output_events(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let speech_session_id = self.require_speech_session_id().await?;
        let result = self
            .service
            .poll_events(PollSpeechEventsCommand {
                speech_session_id,
                runtime_owner_id: self.runtime_owner_id.clone(),
                max_events: POLL_MAX_EVENTS,
            })
            .await
            .map_err(to_anyhow)?;
        Ok(result.events.into_iter().map(convert_event).collect())
    }

    async fn interrupt_stream(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let speech_session_id = self.require_speech_session_id().await?;
        let result = self
            .service
            .interrupt_session(InterruptSpeechSessionCommand {
                speech_session_id,
                runtime_owner_id: self.runtime_owner_id.clone(),
            })
            .await
            .map_err(to_anyhow)?;
        Ok(result.events.into_iter().map(convert_event).collect())
    }

    async fn publish_worker_event(
        &self,
        round_id: Option<&str>,
        kind: RuntimeFactKind,
    ) -> Result<()> {
        // worker→backend fact 上报(barge-in 等),与 RemoteSpeechHandler 一致经控制面客户端。
        let fact = RuntimeEventFact {
            session_id: self.session_id,
            runtime_owner_id: self.runtime_owner_id.clone(),
            round_id: round_id.map(ToOwned::to_owned),
            source: "worker".to_owned(),
            ts_ms: chrono::Utc::now().timestamp_millis(),
            kind,
        };
        self.client.publish_event(fact).await
    }

    async fn close_stream(&self) -> Result<Vec<SpeechRuntimeEvent>> {
        let speech_session_id = self.require_speech_session_id().await?;
        let result = self
            .service
            .close_session(CloseSpeechSessionCommand {
                speech_session_id,
                runtime_owner_id: self.runtime_owner_id.clone(),
            })
            .await
            .map_err(to_anyhow)?;
        Ok(result.events.into_iter().map(convert_event).collect())
    }
}

/// 装配进程内 SpeechRuntimeService + LocalSpeechHandler。
#[allow(clippy::too_many_arguments)]
pub fn build_local_speech_handler(
    bootstrap: &SpeechRuntimeBootstrap,
    session_id: i64,
    runtime_owner_id: String,
    _config: &WorkerConfig,
    client: RuntimeControlClient,
    agent: Arc<dyn agent::AgentRuntimeUseCases>,
    call_events: Arc<dyn call_log_contract::CallEventSinkPort>,
    voice_runtime: Arc<dyn voice::ports::VoiceRuntimeExecutionPort>,
) -> Result<Arc<dyn SpeechHandler>> {
    let language = voice::AsrLanguage::parse(&bootstrap.language)
        .ok_or_else(|| anyhow!("unknown asr language: {}", bootstrap.language))?;
    let context = SpeechRuntimeContext {
        session_id,
        agent_session_id: bootstrap.agent_session_id.clone(),
        voice: bootstrap.voice.clone(),
        runtime_owner_id: runtime_owner_id.clone(),
        language,
    };
    let segmentation = SpeechSegmentationConfig {
        min_utterance_ms: bootstrap.segmentation.min_utterance_ms,
        silence_flush_ms: bootstrap.segmentation.silence_flush_ms,
        silence_force_agent_ms: bootstrap.segmentation.silence_force_agent_ms,
        voice_activity_threshold: bootstrap.segmentation.voice_activity_threshold,
        min_speech_confirm_ms: bootstrap.segmentation.min_speech_confirm_ms,
    };
    let agent_turn = LocalAgentTurnAdapter::new(agent.clone());

    let service = SpeechRuntimeService::new(SpeechRuntimeDependencies {
        runtime_context: StaticRuntimeContext { context },
        segmentation_config: StaticSegmentationConfig {
            config: segmentation,
        },
        session_store: InMemorySpeechSessionStore::new(),
        events: LocalSpeechEventsAdapter::new(call_events),
        // 进程内单进程:owner 即本 worker;owner 路由字段不参与进程内路径。
        instance_id: runtime_owner_id.clone(),
        instance_epoch: "local".to_owned(),
        internal_runtime_advertise_url: String::new(),
        segmentation_policy: Arc::new(ThresholdSpeechSegmentationPolicy),
        voice_runtime,
        agent_turn: Arc::new(agent_turn),
    });

    let welcome_agent = LocalAgentTurnAdapter::new(agent);
    Ok(Arc::new(LocalSpeechHandler {
        service: Arc::new(service),
        client,
        session_id,
        runtime_owner_id,
        speech_session_id: Mutex::new(None),
        agent_session_id: bootstrap.agent_session_id.clone(),
        welcome_agent,
    }))
}

fn to_anyhow(error: shared_kernel::AppError) -> anyhow::Error {
    anyhow!(error.to_string())
}

/// speech_runtime::SpeechRuntimeEvent → client::SpeechRuntimeEvent(结构一一对应)。
fn convert_event(event: speech_runtime::SpeechRuntimeEvent) -> SpeechRuntimeEvent {
    use speech_runtime::SpeechRuntimeEvent as S;
    match event {
        S::ListeningStarted => SpeechRuntimeEvent::ListeningStarted,
        S::SpeechDetected => SpeechRuntimeEvent::SpeechDetected,
        S::UtteranceFlushing => SpeechRuntimeEvent::UtteranceFlushing,
        S::RespondingStarted => SpeechRuntimeEvent::RespondingStarted,
        S::SessionClosing => SpeechRuntimeEvent::SessionClosing,
        S::SessionFailed { message } => SpeechRuntimeEvent::SessionFailed { message },
        S::ReplyStarted {
            round_id,
            reply_text,
            interruptible,
        } => SpeechRuntimeEvent::ReplyStarted {
            round_id,
            reply_text,
            interruptible,
        },
        S::AudioChunk {
            pcm_s16le,
            sample_rate_hz,
            channels,
        } => SpeechRuntimeEvent::AudioChunk {
            pcm_s16le,
            sample_rate_hz,
            channels,
        },
        S::PreRecordedAsset {
            round_id,
            audio_url,
            band,
            interruptible,
        } => SpeechRuntimeEvent::PreRecordedAsset {
            round_id,
            audio_url,
            band,
            interruptible,
        },
        S::ReplyFinished { round_id } => SpeechRuntimeEvent::ReplyFinished { round_id },
        S::RoundFailed { round_id, message } => {
            SpeechRuntimeEvent::RoundFailed { round_id, message }
        }
        S::Warning { message } => SpeechRuntimeEvent::Warning { message },
    }
}
