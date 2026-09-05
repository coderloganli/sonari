use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub use shared_kernel::CallerIdentity as AgentCallerIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// 区分 agent 当前支持的 LLM 提供方配置槽位。
pub enum ProviderKey {
    Conversation,
    Assistant,
}

impl ProviderKey {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "conversation" => Some(Self::Conversation),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
// 标识由 agent 模块统一维护的 prompt 模板键。
pub enum PromptTemplateKey {
    ConversationSystem1,
    ConversationSystem2,
    ConversationSystem3,
    ConversationWelcomeUser,
    MemoryExtraction,
}

impl PromptTemplateKey {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "conversation_system_1" => Some(Self::ConversationSystem1),
            "conversation_system_2" => Some(Self::ConversationSystem2),
            "conversation_system_3" => Some(Self::ConversationSystem3),
            "conversation_welcome_user" => Some(Self::ConversationWelcomeUser),
            "memory_extraction" => Some(Self::MemoryExtraction),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConversationSystem1 => "conversation_system_1",
            Self::ConversationSystem2 => "conversation_system_2",
            Self::ConversationSystem3 => "conversation_system_3",
            Self::ConversationWelcomeUser => "conversation_welcome_user",
            Self::MemoryExtraction => "memory_extraction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
// 统一消息角色，保证存储和 LLM 请求映射一致。
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl MessageRole {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// 保存某个 provider 槽位的可执行模型配置。
pub struct LlmProviderConfig {
    pub provider_key: ProviderKey,
    pub endpoint_url: String,
    /// Read from the environment at startup and never written down, so there is
    /// nothing to encrypt.
    pub api_key: String,
    pub model_name: String,
    pub temperature: f64,
    pub frequency_penalty: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 表示一条可由 application 组装的 prompt 模板。
pub struct PromptTemplate {
    pub id: i64,
    pub template_key: PromptTemplateKey,
    pub template_text: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartnerConversationPromptOverride {
    pub partner_id: i64,
    pub system_prompt_1: String,
    pub system_prompt_2: String,
    pub system_prompt_3: String,
    pub welcome_user_prompt: String,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 表示一段角色对话生命周期内的会话实体。
pub struct AgentSession {
    pub id: String,
    pub caller: AgentCallerIdentity,
    pub character_id: i64,
    pub timezone: String,
    pub scene_id: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 表示会话内按轮次落库的原始消息记录。
pub struct AgentMessage {
    pub id: i64,
    pub session_id: String,
    pub role: MessageRole,
    pub content: String,
    pub turn_number: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 归档时只保留重建历史所需的最小消息字段。
pub struct AgentArchiveMessage {
    pub role: MessageRole,
    pub content: String,
    pub turn_number: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 会话归档聚合了消息快照与 token 用量摘要。
pub struct AgentMessageArchive {
    pub id: i64,
    pub character_id: i64,
    pub messages: Vec<AgentArchiveMessage>,
    pub total_turns: i32,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub archived_at: DateTime<Utc>,
}

/// What kind of thing is remembered. A closed list, because eviction needs a
/// quota to be fair about and the injected text is grouped by it (ADR-0021).
///
/// The order is the order facts are rendered in: what is stable first, what is
/// passing last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// Who the caller is: name, age, where they live, what they do.
    Identity,
    /// Who is around them: family, friends, pets, colleagues.
    Relationship,
    /// What they like, dislike, or will not talk about.
    Preference,
    /// What is going on right now. This is the category that expires.
    Situation,
    /// What was agreed between them and the agent.
    Commitment,
}

impl MemoryCategory {
    /// Every category, in rendering order.
    pub const ALL: [Self; 5] = [
        Self::Identity,
        Self::Relationship,
        Self::Preference,
        Self::Situation,
        Self::Commitment,
    ];

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "identity" => Some(Self::Identity),
            "relationship" => Some(Self::Relationship),
            "preference" => Some(Self::Preference),
            "situation" => Some(Self::Situation),
            "commitment" => Some(Self::Commitment),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Relationship => "relationship",
            Self::Preference => "preference",
            Self::Situation => "situation",
            Self::Commitment => "commitment",
        }
    }
}

/// One thing the agent knows about a caller, as stored.
///
/// The row is structured; the sentence is not. What is worth remembering about a
/// person is an open set, so `content` stays natural language (ADR-0021).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFact {
    pub user_id: i64,
    pub character_id: i64,
    pub category: MemoryCategory,
    pub content: String,
    /// When this fact was first learned. Survives a rewrite that keeps it.
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The session whose extraction last confirmed it.
    pub source_session_id: String,
}

/// A fact as the model produced it, before it is anyone's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedFact {
    pub category: MemoryCategory,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 记录一次 LLM 调用的用量和错误结果。
pub struct LlmUsageLog {
    pub id: i64,
    pub session_id: String,
    pub provider_key: ProviderKey,
    pub model_name: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub is_error: bool,
    pub error_message: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
// 汇总单个会话的 prompt 和 completion token。
pub struct SessionUsageSummary {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
// 汇总管理端查看的整体 LLM 调用统计。
pub struct LlmUsageStats {
    pub total_calls: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_errors: i64,
    pub error_rate: f64,
}
