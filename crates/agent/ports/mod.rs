use async_trait::async_trait;
use shared_kernel::AppResult;

use crate::domain::{
    AgentCallerIdentity, AgentMessage, AgentMessageArchive, AgentSession, ExtractedFact,
    LlmProviderConfig, LlmUsageLog, LlmUsageStats, MemoryFact, PartnerConversationPromptOverride,
    PromptTemplate, PromptTemplateKey, ProviderKey, SessionUsageSummary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAgentSessionRequest {
    pub caller: AgentCallerIdentity,
    pub character_id: i64,
    pub timezone: String,
    pub scene_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAgentSessionResult {
    pub session_id: String,
}

#[async_trait]
pub trait LlmProviderConfigRepository: Send + Sync {
    async fn get_by_key(&self, provider_key: ProviderKey) -> AppResult<Option<LlmProviderConfig>>;
    async fn list_all(&self) -> AppResult<Vec<LlmProviderConfig>>;
    async fn upsert(&self, config: &LlmProviderConfig) -> AppResult<LlmProviderConfig>;
}

#[async_trait]
impl<T> LlmProviderConfigRepository for std::sync::Arc<T>
where
    T: LlmProviderConfigRepository + ?Sized,
{
    async fn get_by_key(&self, provider_key: ProviderKey) -> AppResult<Option<LlmProviderConfig>> {
        (**self).get_by_key(provider_key).await
    }
    async fn list_all(&self) -> AppResult<Vec<LlmProviderConfig>> {
        (**self).list_all().await
    }
    async fn upsert(&self, config: &LlmProviderConfig) -> AppResult<LlmProviderConfig> {
        (**self).upsert(config).await
    }
}

#[async_trait]
pub trait PromptTemplateRepository: Send + Sync {
    async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>>;
    async fn list_all(&self) -> AppResult<Vec<PromptTemplate>>;
    async fn upsert(&self, template: &PromptTemplate) -> AppResult<PromptTemplate>;
}

#[async_trait]
impl<T> PromptTemplateRepository for std::sync::Arc<T>
where
    T: PromptTemplateRepository + ?Sized,
{
    async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>> {
        (**self).get_by_key(key).await
    }
    async fn list_all(&self) -> AppResult<Vec<PromptTemplate>> {
        (**self).list_all().await
    }
    async fn upsert(&self, template: &PromptTemplate) -> AppResult<PromptTemplate> {
        (**self).upsert(template).await
    }
}

#[async_trait]
pub trait PartnerConversationPromptOverrideRepository: Send + Sync {
    async fn get_by_partner_id(
        &self,
        partner_id: i64,
    ) -> AppResult<Option<PartnerConversationPromptOverride>>;
    async fn upsert(
        &self,
        config: &PartnerConversationPromptOverride,
    ) -> AppResult<PartnerConversationPromptOverride>;
    async fn delete_by_partner_id(&self, partner_id: i64) -> AppResult<()>;
}

#[async_trait]
pub trait AgentSessionRepository: Send + Sync {
    async fn create(&self, session: &AgentSession) -> AppResult<AgentSession>;
    async fn get_by_id(&self, session_id: &str) -> AppResult<Option<AgentSession>>;
    async fn end(&self, session_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait AgentMessageRepository: Send + Sync {
    async fn append(&self, message: &AgentMessage) -> AppResult<AgentMessage>;
    async fn list_recent(
        &self,
        session_id: &str,
        recent_turns: i32,
    ) -> AppResult<Vec<AgentMessage>>;
    async fn list_all(&self, session_id: &str) -> AppResult<Vec<AgentMessage>>;
    async fn next_turn_number(&self, session_id: &str) -> AppResult<i32>;
}

#[async_trait]
pub trait AgentArchiveRepository: Send + Sync {
    async fn create(&self, archive: &AgentMessageArchive) -> AppResult<AgentMessageArchive>;
}

#[async_trait]
pub trait UsageLogRepository: Send + Sync {
    async fn create(&self, log: &LlmUsageLog) -> AppResult<LlmUsageLog>;
    async fn get_usage_stats(&self) -> AppResult<LlmUsageStats>;
    async fn summarize_session(&self, session_id: &str) -> AppResult<SessionUsageSummary>;
}

/// Where the fact set lives. Whole sets in, whole sets out: ADR-0021 made the
/// set the unit, so nothing here reads or writes a single fact.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// The facts one persona knows about one caller.
    async fn load(&self, user_id: i64, character_id: i64) -> AppResult<Vec<MemoryFact>>;
    /// Everything held about a caller, across personas. For the caller's own
    /// reading, not for a prompt.
    async fn load_all(&self, user_id: i64) -> AppResult<Vec<MemoryFact>>;
    /// Replaces the set in one transaction. A fact whose content is unchanged
    /// keeps its `first_seen_at`; one that is absent is deleted.
    async fn replace(
        &self,
        user_id: i64,
        character_id: i64,
        source_session_id: &str,
        facts: &[ExtractedFact],
    ) -> AppResult<()>;
    /// Forgets one persona's facts, or all of them when `character_id` is
    /// `None`. Returns how many rows went.
    async fn delete(&self, user_id: i64, character_id: Option<i64>) -> AppResult<u64>;
}

#[async_trait]
impl<T> MemoryStore for std::sync::Arc<T>
where
    T: MemoryStore + ?Sized,
{
    async fn load(&self, user_id: i64, character_id: i64) -> AppResult<Vec<MemoryFact>> {
        (**self).load(user_id, character_id).await
    }
    async fn load_all(&self, user_id: i64) -> AppResult<Vec<MemoryFact>> {
        (**self).load_all(user_id).await
    }
    async fn replace(
        &self,
        user_id: i64,
        character_id: i64,
        source_session_id: &str,
        facts: &[ExtractedFact],
    ) -> AppResult<()> {
        (**self)
            .replace(user_id, character_id, source_session_id, facts)
            .await
    }
    async fn delete(&self, user_id: i64, character_id: Option<i64>) -> AppResult<u64> {
        (**self).delete(user_id, character_id).await
    }
}

/// Starts an extraction without waiting for it (ADR-0022).
///
/// Deliberately not `async`: the turn path calls this and moves on. How the work
/// actually leaves the current task is the composition root's business, and the
/// only place that knows about spawning.
pub trait MemoryExtractionScheduler: Send + Sync {
    fn schedule(&self, session_id: &str);
}

impl<T> MemoryExtractionScheduler for std::sync::Arc<T>
where
    T: MemoryExtractionScheduler + ?Sized,
{
    fn schedule(&self, session_id: &str) {
        (**self).schedule(session_id)
    }
}

#[async_trait]
pub trait AgentSettingsRepository: Send + Sync {
    async fn get_recent_turns(&self) -> AppResult<i32>;
    async fn set_recent_turns(&self, recent_turns: i32) -> AppResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmRequestMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmCompletionRequest {
    pub endpoint_url: String,
    pub api_key: String,
    pub model_name: String,
    pub temperature: f64,
    pub frequency_penalty: f64,
    pub messages: Vec<LlmRequestMessage>,
    pub max_tokens: Option<i32>,
    /// Tools the model may call. Empty means none are offered.
    pub tools: Vec<ToolDefinition>,
}

// No `Eq`: the timings are floating point.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmCompletionResponse {
    pub content: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    /// When the first token arrived, as epoch milliseconds.
    ///
    /// Recorded here because this is the only place that sees the stream, and
    /// absolute because two of ADR-0010's markers are derived from it in another
    /// crate — an offset would need an anchor, and the anchor would be a guess.
    pub first_token_at_ms: Option<i64>,
    /// When the first complete sentence existed — what synthesis can start on,
    /// and therefore the earliest the reply could begin to be spoken.
    pub first_sentence_at_ms: Option<i64>,
}

/// A tool the model may call. Declared per persona and dispatched by the
/// conversation loop; it never touches audio.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON, assembled from the fragments the stream delivers.
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LlmUsage {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
}

/// One piece of a reply as it is produced.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmDelta {
    /// Text to speak, as soon as it exists.
    Token(String),
    /// A completed tool call. Fragments are assembled before this is emitted.
    ToolCall(ToolCall),
    /// The reply is finished. Usage is whatever the endpoint reported.
    Done(LlmUsage),
}

pub type LlmStream = std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<LlmDelta>> + Send>>;

#[async_trait]
pub trait LlmGateway: Send + Sync {
    /// Streams a reply. Tokens arrive as they are generated so that synthesis
    /// can begin on the first sentence rather than the last token.
    async fn stream(&self, request: LlmCompletionRequest) -> AppResult<LlmStream>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

pub trait IdGenerator: Send + Sync {
    fn next_session_id(&self) -> String;
}

#[async_trait]
pub trait AgentCallControlPort: Send + Sync {
    async fn create_call_session(
        &self,
        request: CreateAgentSessionRequest,
    ) -> AppResult<CreateAgentSessionResult>;
    async fn generate_welcome_message(&self, agent_session_id: &str) -> AppResult<String>;
}
