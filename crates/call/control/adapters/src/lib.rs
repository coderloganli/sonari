pub mod adapters;

pub use adapters::{
    agent::AgentSessionAdapter,
    agent_prompt_log::AgentPromptLogEventAdapter,
    call_event_log::PostgresCallEventLogSink,
    observability_client_event::ClientDebugEventCallSinkAdapter,
    postgres_bot_speech_state::PostgresBotSpeechStateStoreAdapter,
    postgres_call_logs::PostgresCallEventLogRepository,
    postgres_event_outbox::{
        CallEventOutboxPublisher, CallEventOutboxPublisherHandle, PostgresCallEventOutboxRepository,
    },
    postgres_event_sink::PostgresCallEventOutboxSink,
    postgres_sessions::PostgresCallSessionRepository,
    server_initiated_turn::{ServerInitiatedBotSpeechAdapter, ServerInitiatedTurnTextAdapter},
    user::UserContextAdapter,
    voice::InputLanguageAdapter,
};
