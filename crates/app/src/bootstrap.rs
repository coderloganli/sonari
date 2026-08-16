use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::serve;
use call::{
    BotSpeechDependencies, BotSpeechService, CallLogEngine, CallLogUseCases,
    ConsumeBotSpeechRuntimeFactUseCases,
};
use call_control_adapters::{
    AgentSessionAdapter, CallEventOutboxPublisher, InputLanguageAdapter,
    PostgresBotSpeechStateStoreAdapter, PostgresCallEventLogRepository, PostgresCallEventLogSink,
    PostgresCallEventOutboxRepository, PostgresCallEventOutboxSink, PostgresCallSessionRepository,
    ServerInitiatedBotSpeechAdapter, ServerInitiatedTurnTextAdapter, UserContextAdapter,
};
use rtc::livekit::join::LiveKitRealtimeJoinAdapter;
use rtc::livekit::launch::LiveKitRuntimeLaunchAdapter;
use rtc::livekit::token::{LiveKitTokenConfig, LiveKitTokenIssuer};
use speech_runtime::{BotSpeechRuntimeTriggerAdapter, SpeechRuntimeEventAdapter};
use tokio::{net::TcpListener, signal};

use crate::config::AppConfig;

/// Breaks the assembly cycle: call execution needs somewhere to send runtime
/// facts, and the service that consumes them is built afterwards. The handle is
/// attached once that service exists; facts arriving before then are rejected as
/// unavailable rather than dropped silently.
#[derive(Default)]
struct DeferredFactConsumer {
    inner: std::sync::OnceLock<Arc<dyn ConsumeBotSpeechRuntimeFactUseCases>>,
}

impl DeferredFactConsumer {
    fn attach(&self, consumer: Arc<dyn ConsumeBotSpeechRuntimeFactUseCases>) {
        if self.inner.set(consumer).is_err() {
            tracing::warn!("runtime fact consumer attached more than once");
        }
    }
}

#[async_trait::async_trait]
impl call_execution::RuntimeFactConsumer for DeferredFactConsumer {
    async fn consume(
        &self,
        fact: call_runtime_control::RuntimeEventFact,
    ) -> shared_kernel::AppResult<()> {
        let Some(consumer) = self.inner.get() else {
            return Err(shared_kernel::AppError::unavailable(
                "runtime fact consumer is not attached yet",
            ));
        };
        consumer
            .consume_bot_speech_runtime_fact(call::ConsumeBotSpeechRuntimeFactCommand { fact })
            .await
    }
}

pub async fn run() -> Result<()> {
    observability::init_tracing("backend")?;
    let _service_span = observability::service_span("backend").entered();

    let config = AppConfig::from_env()?;
    validate_instance_id(&config)?;
    validate_livekit_config(&config)?;
    tracing::info!("connecting postgres");
    let pool = platform_postgres::connect(&config.postgres).await?;
    tracing::info!("postgres connected");
    tracing::info!("running migrations");
    platform_postgres::run_migrations(&pool).await?;
    tracing::info!("migrations completed");
    let token_service = Arc::new(auth::adapters::jwt::JwtTokenService::new(
        auth::adapters::jwt::JwtTokenConfig {
            secret: config.auth.jwt.secret.clone(),
            access_token_ttl: config.auth.jwt.access_token_ttl(),
            refresh_token_ttl: config.auth.jwt.refresh_token_ttl(),
        },
    ));
    let settings_path = sonari_config::config_path();
    let settings = sonari_config::load_and_watch(&settings_path)?;
    tracing::info!(path = %settings_path.display(), "configuration loaded");

    tracing::info!("assembling services");

    // Where the model lives comes from the environment; how it behaves comes
    // from the configuration file.
    let llm_providers = Arc::new(crate::llm_config::ConfigLlmProviders::new(
        settings.clone(),
        crate::llm_config::LlmEndpoint::from_env()?,
    ));
    let voice_service = Arc::new(voice::VoiceCallConfigService::new(
        voice::PostgresVoiceConfigRepository::new(pool.clone()),
    ));
    let voice_runtime = build_voice_runtime(&settings)?;
    // Personas come from configuration, not the database: a clean clone must be
    // able to hold a conversation without seeding anything.
    let personas = Arc::new(crate::persona::ConfigPersonas::new(settings.clone()));
    let character_call_context: Arc<dyn character_context::CharacterCallContextReadPort> =
        personas.clone();
    let character_prompt_context: Arc<dyn character_context::CharacterPromptContextReadPort> =
        personas.clone();
    // The same object the call path resolves ids through, so the list the API
    // publishes and the ids a call accepts cannot drift.
    let persona_catalog: Arc<dyn character_context::CharacterCatalogReadPort> = personas;
    let user_call_context: Arc<dyn user_context::UserCallContextReadPort> =
        Arc::new(crate::persona::AnonymousCallers);

    let call_sessions = PostgresCallSessionRepository::new(pool.clone());
    let call_log_repo = PostgresCallEventLogRepository::new(pool.clone());
    let call_event_sink = PostgresCallEventLogSink::new(pool.clone());
    let call_event_outbox_sink = PostgresCallEventOutboxSink::new(pool.clone());
    let call_event_outbox_sink_port: Arc<dyn call::ports::EventSinkPort> =
        Arc::new(call_event_outbox_sink.clone());
    let call_event_outbox = PostgresCallEventOutboxRepository::new(pool.clone());
    let call_event_publisher = CallEventOutboxPublisher::new(call_event_outbox, call_event_sink)
        .start()
        .await
        .context("failed to start call event outbox publisher")?;
    let agent_service = Arc::new(agent::AgentService::new(agent::AgentDependencies {
        providers: llm_providers.clone(),
        templates: Arc::new(crate::prompts::ConfigPromptTemplates::new(settings.clone())),
        partner_prompt_overrides: agent::PostgresPartnerConversationPromptOverrideRepository::new(
            pool.clone(),
        ),
        sessions: agent::PostgresAgentSessionRepository::new(pool.clone()),
        messages: agent::PostgresAgentMessageRepository::new(pool.clone()),
        usage_logs: agent::PostgresAgentUsageLogRepository::new(pool.clone()),
        gateway: agent::adapters::llm::ReqwestLlmGateway::default(),
        characters: character_prompt_context,
        ids: StaticIdGenerator,
        clock: SystemClock,
        settings: Box::new(agent::PostgresAgentSettingsRepository::new(pool.clone())),
    }));
    let agent_call_control: Arc<dyn agent::AgentCallControlPort> = agent_service.clone();
    let agent_runtime: Arc<dyn agent::AgentRuntimeUseCases> = agent_service.clone();
    let call_log_service: Arc<dyn CallLogUseCases> = Arc::new(CallLogEngine::new(call_log_repo));
    let instance_id = resolve_instance_id(&config)?;
    let instance_epoch = process_instance_epoch();
    let runtime_owner_id = resolve_runtime_owner_id(&config)?;
    let livekit_token_issuer = LiveKitTokenIssuer::new(LiveKitTokenConfig {
        internal_livekit_url: config.livekit.url.clone(),
        public_livekit_url: config.livekit.public_url.clone(),
        api_key: config.livekit.api_key.clone(),
        api_secret: config.livekit.api_secret.clone(),
    });
    let server_initiated_turn = ServerInitiatedBotSpeechAdapter::new(
        call_sessions.clone(),
        ServerInitiatedTurnTextAdapter::new(agent_call_control.clone()),
    );
    let bot_speech_state_store: Arc<dyn call::BotSpeechStateStorePort> =
        Arc::new(PostgresBotSpeechStateStoreAdapter::new(pool.clone()));
    let call_service = Arc::new(call::CallService::new(call::CallDependencies {
        sessions: call_sessions.clone(),
        transactions: call_sessions.clone(),
        characters: character_call_context,
        agent: AgentSessionAdapter::new(agent_call_control.clone()),
        input_language_port: InputLanguageAdapter::new(voice_service.clone()),
        user_context: UserContextAdapter::new(user_call_context),
        instance_identity: StaticInstanceIdentity {
            id: instance_id.clone(),
        },
        runtime_owner: StaticRuntimeOwner {
            id: runtime_owner_id.clone(),
        },
        realtime_join: LiveKitRealtimeJoinAdapter::new(livekit_token_issuer.clone()),
        bot_speech_state_store: bot_speech_state_store.clone(),
    }));
    let fact_consumer = Arc::new(DeferredFactConsumer::default());
    let call_execution_service = Arc::new(call_execution::CallExecutionService::new(
        call_execution::CallExecutionDependencies {
            control: call_service.clone(),
            launch: crate::livekit_launch::LiveKitRuntimeLaunchProvision::new(
                Arc::new(LiveKitRuntimeLaunchAdapter::new(
                    livekit_token_issuer.clone(),
                )),
                // Assembles the per-session configuration dispatch hands to the
                // media plane. In-process orchestration is the only path.
                Arc::new(crate::speech_bootstrap::DbSpeechBootstrapComposer::new(
                    Arc::new(crate::endpointing::ConfigEndpointing::new(settings.clone())),
                    llm_providers.clone(),
                )),
            ),
            runtime_context: call_execution::ControlRuntimeContextAdapter::new(
                call_sessions.clone(),
            ),
            events: call_execution::CallExecutionEventAdapter::new(call_event_outbox_sink.clone()),
            fact_log: call_execution::RuntimeFactLogAdapter::new(fact_consumer.clone()),
        },
    ));
    let call_execution_use_cases: Arc<dyn call_execution::CallExecutionUseCases> =
        call_execution_service.clone();
    let speech_segmentation_config: Arc<dyn speech_runtime::SpeechSegmentationConfigPort> =
        Arc::new(crate::endpointing::ConfigEndpointing::new(settings.clone()));
    let speech_runtime_service = Arc::new(speech_runtime::SpeechRuntimeService::new(
        speech_runtime::SpeechRuntimeDependencies {
            runtime_context: Arc::new(speech_runtime::ExecutionRuntimeContextAdapter::new(
                call_execution_service.clone(),
            )),
            segmentation_config: speech_segmentation_config,
            session_store: speech_runtime::InMemorySpeechSessionStore::new(),
            events: SpeechRuntimeEventAdapter::new(call_event_outbox_sink.clone()),
            instance_id: instance_id.clone(),
            instance_epoch,
            internal_runtime_advertise_url: resolve_internal_runtime_advertise_url(&config)?,
            segmentation_policy: Arc::new(speech_runtime::ThresholdSpeechSegmentationPolicy),
            voice_runtime: voice_runtime.clone(),
            agent_turn: Arc::new(speech_runtime::AgentRuntimeAdapter::new(agent_runtime)),
        },
    ));
    let runtime_speech_trigger: Arc<dyn call::BotSpeechRuntimeTriggerPort> = Arc::new(
        BotSpeechRuntimeTriggerAdapter::new(speech_runtime_service.clone()),
    );
    let bot_speech_service: Arc<dyn ConsumeBotSpeechRuntimeFactUseCases> =
        Arc::new(BotSpeechService::new(BotSpeechDependencies {
            runtime_sessions: call_sessions.clone(),
            state_store: bot_speech_state_store.clone(),
            event_sink: call_event_outbox_sink_port,
            runtime_trigger: runtime_speech_trigger.clone(),
            server_initiated_turn: server_initiated_turn.clone(),
        }));
    fact_consumer.attach(bot_speech_service);
    // The media plane runs as a task in this process: dispatch and the per-turn
    // agent call are function calls, and audio never leaves the process.
    let worker_config =
        worker::WorkerConfig::from_env(runtime_owner_id.clone(), config.secrets.voice.clone())?;
    // Supervised, because an unwatched media plane is a service that answers
    // /healthz while no call can be answered at all: every `?` in the worker
    // loop returns from the task, which drops every active runtime with it.
    let media_plane = tokio::spawn(supervise_media_plane(worker::run(
        worker_config,
        call_execution_use_cases.clone(),
        speech_runtime_service.clone(),
        agent_service.clone(),
        Arc::new(call_event_outbox_sink.clone()),
        voice_runtime,
    )));
    tracing::info!("media plane started");

    let router = api::build_router_with_modules(api::ModuleServices {
        token_service,
        call_service: call_service.clone(),
        call_log_service,
        persona_catalog,
    });
    tracing::info!("services assembled");

    let host: IpAddr = config
        .server
        .host
        .parse()
        .with_context(|| format!("invalid SERVER_HOST: {}", config.server.host))?;
    let addr = SocketAddr::new(host, config.server.port);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;

    tracing::info!(%addr, "server starting");

    let serve_result = serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server exited with error");

    media_plane.abort();
    let publisher_stop_result = call_event_publisher.stop().await;

    serve_result?;
    publisher_stop_result.context("failed to stop call event publisher")?;
    Ok(())
}

/// Loads the speech models named in configuration.
///
/// Models load once at startup: they are large and shared by every call. With
/// none configured the process still serves HTTP and reports voice as
/// unavailable, rather than refusing to start.
fn build_voice_runtime(
    settings: &sonari_config::SettingsHandle,
) -> Result<Arc<dyn voice::VoiceRuntimeExecutionPort>> {
    let settings = settings.get();
    let Some(models) = &settings.models else {
        tracing::warn!("no speech models configured; voice is unavailable");
        return Ok(Arc::new(voice::UnavailableVoiceRuntime));
    };

    // One credential for both hosted stages, read once and held by the adapters.
    let api_key = std::env::var("ELEVENLABS_API_KEY").unwrap_or_default();
    let asr = providers::ElevenLabsAsrEngine::new(models.asr.clone(), api_key.clone())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let tts = providers::ElevenLabsTtsEngine::new(models.tts.clone(), api_key)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    tracing::info!(
        asr_model = %models.asr.model,
        tts_model = %models.tts.model,
        tts_sample_rate_hz = tts.sample_rate_hz(),
        "speech providers configured"
    );

    Ok(Arc::new(voice::LocalVoiceRuntime::new(
        Arc::new(asr),
        Arc::new(tts),
    )))
}

fn validate_livekit_config(config: &AppConfig) -> Result<()> {
    if config.livekit.url.trim().is_empty() {
        anyhow::bail!("LIVEKIT_URL must be configured");
    }

    if config.livekit.public_url.trim().is_empty() {
        anyhow::bail!("LIVEKIT_PUBLIC_URL must be configured");
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct SystemClock;

impl auth::ports::Clock for SystemClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

impl agent::ports::Clock for SystemClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
}

#[derive(Debug, Clone, Copy)]
struct StaticIdGenerator;

impl agent::ports::IdGenerator for StaticIdGenerator {
    fn next_session_id(&self) -> String {
        format!(
            "sess-{}-{:016x}",
            chrono::Utc::now().timestamp_millis(),
            rand::random::<u64>()
        )
    }
}

#[derive(Debug, Clone)]
struct StaticInstanceIdentity {
    id: String,
}

impl call::ports::InstanceIdentityPort for StaticInstanceIdentity {
    fn current_instance(&self) -> call::InstanceIdentity {
        call::InstanceIdentity {
            id: self.id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct StaticRuntimeOwner {
    id: String,
}

impl call::ports::RuntimeOwnerPort for StaticRuntimeOwner {
    fn current_runtime_owner(&self) -> String {
        self.id.clone()
    }
}

fn resolve_instance_id(config: &AppConfig) -> Result<String> {
    config
        .server
        .instance_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("SERVER_INSTANCE_ID must be configured"))
}

fn resolve_runtime_owner_id(config: &AppConfig) -> Result<String> {
    if let Some(runtime_owner_id) = config.runtime_owner_id.as_ref()
        && !runtime_owner_id.trim().is_empty()
    {
        return Ok(runtime_owner_id.clone());
    }

    Err(anyhow::anyhow!("RUNTIME_OWNER_ID must be configured"))
}

fn resolve_internal_runtime_advertise_url(config: &AppConfig) -> Result<String> {
    let url = config.internal_runtime_advertise_url.trim();
    if url.is_empty() {
        return Err(anyhow::anyhow!(
            "INTERNAL_RUNTIME_ADVERTISE_URL must be configured"
        ));
    }
    let host = internal_runtime_advertise_host(url)?;
    let instance_id = resolve_instance_id(config)?;
    if host != instance_id && !host.starts_with(&format!("{instance_id}.")) {
        return Err(anyhow::anyhow!(
            "INTERNAL_RUNTIME_ADVERTISE_URL host must identify SERVER_INSTANCE_ID"
        ));
    }
    Ok(url.to_owned())
}

fn internal_runtime_advertise_host(url: &str) -> Result<&str> {
    let (_, rest) = url
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("INTERNAL_RUNTIME_ADVERTISE_URL must include scheme"))?;
    let authority = rest
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("INTERNAL_RUNTIME_ADVERTISE_URL must include host"))?;
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        return Err(anyhow::anyhow!(
            "INTERNAL_RUNTIME_ADVERTISE_URL host is empty"
        ));
    }
    Ok(host)
}

fn process_instance_epoch() -> String {
    format!(
        "{}-{:016x}",
        chrono::Utc::now().timestamp_millis(),
        rand::random::<u64>()
    )
}

fn validate_instance_id(config: &AppConfig) -> Result<()> {
    if config
        .server
        .instance_id
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Ok(());
    }
    Err(anyhow::anyhow!("SERVER_INSTANCE_ID must be configured"))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            let _ = stream.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

/// Reports what became of the media plane.
///
/// Its task returning is not a normal event: the worker loop only leaves on an
/// error, and when it does every runtime it was holding is dropped, so calls in
/// progress end and new ones are never claimed. Saying so loudly is the least
/// this can do.
async fn supervise_media_plane(
    plane: impl std::future::Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    match plane.await {
        Ok(()) => {
            tracing::error!("media plane stopped; no further call will be served");
            Ok(())
        }
        Err(error) => {
            tracing::error!(%error, "media plane failed; no further call will be served");
            Err(error)
        }
    }
}
