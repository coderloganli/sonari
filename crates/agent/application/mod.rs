use async_trait::async_trait;
use character_context::{CharacterPromptContext, CharacterPromptContextReadPort};
use serde::Serialize;
use shared_kernel::{AppError, AppResult};

use crate::domain::{
    AgentCallerIdentity, AgentMessage, AgentSession, LlmProviderConfig, LlmUsageLog, MessageRole,
    PartnerConversationPromptOverride, PromptTemplateKey, ProviderKey,
};
use crate::ports::{
    AgentCallControlPort, AgentMessageRepository, AgentSessionRepository, AgentSettingsRepository,
    Clock, CreateAgentSessionRequest, CreateAgentSessionResult, IdGenerator, LlmCompletionRequest,
    LlmGateway, LlmProviderConfigRepository, LlmRequestMessage,
    PartnerConversationPromptOverrideRepository, PromptTemplateRepository, UsageLogRepository,
};

const DEFAULT_RECENT_TURNS: i32 = 6;

#[derive(Debug, Clone)]
pub struct UpdateAdminConfigCommand {
    pub provider_key: String,
    pub endpoint_url: String,
    pub api_key: Option<String>,
    pub model_name: String,
    pub temperature: f64,
    pub frequency_penalty: f64,
    pub system_prompt: Option<String>,
    pub system_prompt_1: Option<String>,
    pub system_prompt_2: Option<String>,
    pub system_prompt_3: Option<String>,
    pub welcome_user_prompt: Option<String>,
    pub recent_turns: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionCommand {
    pub caller: AgentCallerIdentity,
    pub character_id: i64,
    pub timezone: String,
    pub scene_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ChatCommand {
    pub session_id: String,
    pub user_message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminConfigView {
    pub provider_key: String,
    pub endpoint_url: String,
    pub api_key_prefix: String,
    pub model_name: String,
    pub temperature: f64,
    pub frequency_penalty: f64,
    pub system_prompt: String,
    pub system_prompt_1: String,
    pub system_prompt_2: String,
    pub system_prompt_3: String,
    pub welcome_user_prompt: String,
    pub recent_turns: i32,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub struct UpdatePartnerConversationPromptConfigCommand {
    pub partner_id: i64,
    pub enabled: bool,
    pub system_prompt_1: String,
    pub system_prompt_2: String,
    pub system_prompt_3: String,
    pub welcome_user_prompt: String,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartnerConversationPromptConfigView {
    pub partner_id: i64,
    pub enabled: bool,
    pub system_prompt_1: String,
    pub system_prompt_2: String,
    pub system_prompt_3: String,
    pub welcome_user_prompt: String,
    pub updated_by: Option<String>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub inherited_from_global: bool,
}

pub struct AgentDependencies<P, T, PP, S, M, U, G, C, I, K> {
    pub providers: P,
    pub templates: T,
    pub partner_prompt_overrides: PP,
    pub sessions: S,
    pub messages: M,
    pub usage_logs: U,
    pub gateway: G,
    pub characters: C,
    pub ids: I,
    pub clock: K,
    pub settings: Box<dyn AgentSettingsRepository>,
}

pub struct AgentService<P, T, PP, S, M, U, G, C, I, K> {
    providers: P,
    templates: T,
    partner_prompt_overrides: PP,
    sessions: S,
    messages: M,
    usage_logs: U,
    gateway: G,
    characters: C,
    ids: I,
    clock: K,
    settings: Box<dyn AgentSettingsRepository>,
}

impl<P, T, PP, S, M, U, G, C, I, K> AgentService<P, T, PP, S, M, U, G, C, I, K> {
    pub fn new(deps: AgentDependencies<P, T, PP, S, M, U, G, C, I, K>) -> Self {
        Self {
            providers: deps.providers,
            templates: deps.templates,
            partner_prompt_overrides: deps.partner_prompt_overrides,
            sessions: deps.sessions,
            messages: deps.messages,
            usage_logs: deps.usage_logs,
            gateway: deps.gateway,
            characters: deps.characters,
            ids: deps.ids,
            clock: deps.clock,
            settings: deps.settings,
        }
    }
}

#[async_trait]
trait AgentUseCases: Send + Sync {
    async fn create_session(&self, command: CreateSessionCommand) -> AppResult<AgentSession>;
    async fn get_session(&self, session_id: &str) -> AppResult<AgentSession>;
    async fn generate_welcome_message(&self, session_id: &str) -> AppResult<String>;
    async fn chat_once(&self, command: ChatCommand) -> AppResult<String>;
}

#[async_trait]
pub trait AgentRuntimeUseCases: Send + Sync {
    async fn chat_once(&self, command: ChatCommand) -> AppResult<String>;
    /// 生成开场欢迎语(server-initiated turn);进程内编排时由 worker 起会话后调用。
    async fn generate_welcome_message(&self, agent_session_id: &str) -> AppResult<String>;
}

#[async_trait]
impl<P, T, PP, S, M, U, G, C, I, K> AgentUseCases for AgentService<P, T, PP, S, M, U, G, C, I, K>
where
    P: LlmProviderConfigRepository + Send + Sync,
    T: PromptTemplateRepository + Send + Sync,
    PP: PartnerConversationPromptOverrideRepository + Send + Sync,
    S: AgentSessionRepository + Send + Sync,
    M: AgentMessageRepository + Send + Sync,
    U: UsageLogRepository + Send + Sync,
    G: LlmGateway + Send + Sync,
    C: CharacterPromptContextReadPort + Send + Sync,
    I: IdGenerator + Send + Sync,
    K: Clock + Send + Sync,
{
    async fn create_session(&self, command: CreateSessionCommand) -> AppResult<AgentSession> {
        if !valid_agent_caller(&command.caller) || command.character_id <= 0 {
            return Err(AppError::invalid_input(
                "caller and character_id are required",
            ));
        }
        self.sessions
            .create(&AgentSession {
                id: self.ids.next_session_id(),
                caller: command.caller,
                character_id: command.character_id,
                timezone: command.timezone,
                scene_id: command.scene_id,
                started_at: self.clock.now(),
                ended_at: None,
            })
            .await
    }

    async fn get_session(&self, session_id: &str) -> AppResult<AgentSession> {
        self.sessions
            .get_by_id(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent session not found"))
    }

    async fn generate_welcome_message(&self, session_id: &str) -> AppResult<String> {
        let session = self.get_session(session_id).await?;
        let prompt_context = self
            .characters
            .get_prompt_context(session.character_id, session.scene_id)
            .await?;
        let provider = self
            .require_provider_config(ProviderKey::Conversation)
            .await?;
        let prompts = self.resolve_conversation_prompt_bundle(None).await?;
        let user_prompt = self.build_prompt_from_template(
            &prompts.welcome_user_prompt,
            &prompt_context,
            &session.timezone,
        )?;
        let turn_number = self.messages.next_turn_number(session_id).await?;
        self.append_message(&session.id, MessageRole::User, &user_prompt, turn_number)
            .await?;

        let mut messages = self
            .build_system_messages(&prompt_context, &session.timezone, None)
            .await?;
        messages.push(LlmRequestMessage {
            role: MessageRole::User.as_str().to_owned(),
            content: user_prompt,
        });
        let response = match self
            .gateway
            .stream(self.build_request(&provider, messages, Some(120))?)
            .await
        {
            Ok(stream) => collect_reply(stream).await,
            Err(error) => Err(error),
        };
        self.log_usage_completion(
            &session.id,
            provider.provider_key,
            &provider.model_name,
            &response,
        )
        .await?;
        let response = response?;
        let content = response.content.trim().to_owned();
        if content.is_empty() {
            return Err(AppError::unavailable("welcome message is empty"));
        }
        self.append_message(&session.id, MessageRole::Assistant, &content, turn_number)
            .await?;
        Ok(content)
    }

    async fn chat_once(&self, command: ChatCommand) -> AppResult<String> {
        let session = self.get_session(&command.session_id).await?;
        let provider = self
            .require_provider_config(ProviderKey::Conversation)
            .await?;
        let request_messages = self
            .build_chat_messages(&session, &command.user_message)
            .await?;
        let turn_number = self.messages.next_turn_number(&session.id).await?;
        self.append_message(
            &session.id,
            MessageRole::User,
            &command.user_message,
            turn_number,
        )
        .await?;
        let response = match self
            .gateway
            .stream(self.build_request(&provider, request_messages, None)?)
            .await
        {
            Ok(stream) => collect_reply(stream).await,
            Err(error) => Err(error),
        };
        self.log_usage_completion(
            &session.id,
            provider.provider_key,
            &provider.model_name,
            &response,
        )
        .await?;
        let response = response?;
        self.append_message(
            &session.id,
            MessageRole::Assistant,
            &response.content,
            turn_number,
        )
        .await?;
        Ok(response.content)
    }
}

/// Drains a reply stream into the finished result.
///
/// Callers that need audio as it is produced consume the stream directly; this
/// is for the paths that only want the completed text, and it is where usage
/// and tool calls are gathered.
async fn collect_reply(
    mut stream: crate::ports::LlmStream,
) -> AppResult<crate::ports::LlmCompletionResponse> {
    use futures::StreamExt;

    let mut content = String::new();
    let mut usage = crate::ports::LlmUsage::default();
    let mut tool_calls = Vec::new();
    while let Some(delta) = stream.next().await {
        match delta? {
            crate::ports::LlmDelta::Token(token) => content.push_str(&token),
            crate::ports::LlmDelta::ToolCall(call) => tool_calls.push(call),
            crate::ports::LlmDelta::Done(reported) => usage = reported,
        }
    }
    if !tool_calls.is_empty() {
        // Dispatch lands with the persona work that declares the tools; until
        // then a call would be answered with nothing, which is worse than
        // saying so.
        tracing::warn!(
            count = tool_calls.len(),
            "model requested tools, but none are dispatchable yet"
        );
    }
    Ok(crate::ports::LlmCompletionResponse {
        content,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
    })
}

/// Retained for the validation it encodes, which its test pins down; the admin
/// surface that called it is gone.
#[cfg(test)]
fn required_prompt(value: Option<&str>, field: &'static str) -> AppResult<String> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() {
        return Err(AppError::invalid_input(format!("{field} is required")));
    }
    reject_legacy_prompt_placeholders(value)?;
    Ok(value.to_owned())
}

fn valid_agent_caller(caller: &AgentCallerIdentity) -> bool {
    match caller {
        AgentCallerIdentity::PlatformUser { user_id } => *user_id > 0,
    }
}

#[async_trait]
#[async_trait]
#[async_trait]
impl<P, T, PP, S, M, U, G, C, I, K> AgentRuntimeUseCases
    for AgentService<P, T, PP, S, M, U, G, C, I, K>
where
    P: LlmProviderConfigRepository + Send + Sync,
    T: PromptTemplateRepository + Send + Sync,
    PP: PartnerConversationPromptOverrideRepository + Send + Sync,
    S: AgentSessionRepository + Send + Sync,
    M: AgentMessageRepository + Send + Sync,
    U: UsageLogRepository + Send + Sync,
    G: LlmGateway + Send + Sync,
    C: CharacterPromptContextReadPort + Send + Sync,
    I: IdGenerator + Send + Sync,
    K: Clock + Send + Sync,
{
    async fn chat_once(&self, command: ChatCommand) -> AppResult<String> {
        AgentUseCases::chat_once(self, command).await
    }

    async fn generate_welcome_message(&self, agent_session_id: &str) -> AppResult<String> {
        AgentUseCases::generate_welcome_message(self, agent_session_id).await
    }
}

#[async_trait]
impl<P, T, PP, S, M, U, G, C, I, K> AgentCallControlPort
    for AgentService<P, T, PP, S, M, U, G, C, I, K>
where
    P: LlmProviderConfigRepository + Send + Sync,
    T: PromptTemplateRepository + Send + Sync,
    PP: PartnerConversationPromptOverrideRepository + Send + Sync,
    S: AgentSessionRepository + Send + Sync,
    M: AgentMessageRepository + Send + Sync,
    U: UsageLogRepository + Send + Sync,
    G: LlmGateway + Send + Sync,
    C: CharacterPromptContextReadPort + Send + Sync,
    I: IdGenerator + Send + Sync,
    K: Clock + Send + Sync,
{
    async fn create_call_session(
        &self,
        request: CreateAgentSessionRequest,
    ) -> AppResult<CreateAgentSessionResult> {
        let session = AgentUseCases::create_session(
            self,
            CreateSessionCommand {
                caller: request.caller,
                character_id: request.character_id,
                timezone: request.timezone,
                scene_id: request.scene_id,
            },
        )
        .await?;
        Ok(CreateAgentSessionResult {
            session_id: session.id,
        })
    }

    async fn generate_welcome_message(&self, agent_session_id: &str) -> AppResult<String> {
        AgentUseCases::generate_welcome_message(self, agent_session_id).await
    }
}

impl<P, T, PP, S, M, U, G, C, I, K> AgentService<P, T, PP, S, M, U, G, C, I, K>
where
    P: LlmProviderConfigRepository + Send + Sync,
    T: PromptTemplateRepository + Send + Sync,
    PP: PartnerConversationPromptOverrideRepository + Send + Sync,
    S: AgentSessionRepository + Send + Sync,
    M: AgentMessageRepository + Send + Sync,
    U: UsageLogRepository + Send + Sync,
    G: LlmGateway + Send + Sync,
    C: CharacterPromptContextReadPort + Send + Sync,
    I: IdGenerator + Send + Sync,
    K: Clock + Send + Sync,
{
    async fn require_provider_config(
        &self,
        provider_key: ProviderKey,
    ) -> AppResult<LlmProviderConfig> {
        self.providers
            .get_by_key(provider_key)
            .await?
            .ok_or_else(|| AppError::not_found("agent provider config not found"))
    }

    async fn get_template_text(&self, key: PromptTemplateKey) -> AppResult<String> {
        Ok(self
            .templates
            .get_by_key(key)
            .await?
            .map(|item| item.template_text)
            .unwrap_or_default())
    }

    async fn get_partner_prompt_override(
        &self,
        partner_id: i64,
    ) -> AppResult<Option<PartnerConversationPromptOverride>> {
        if partner_id <= 0 {
            return Ok(None);
        }
        self.partner_prompt_overrides
            .get_by_partner_id(partner_id)
            .await
    }

    async fn resolve_conversation_prompt_bundle(
        &self,
        partner_id: Option<i64>,
    ) -> AppResult<PartnerConversationPromptConfigView> {
        let system_prompt_1 = self
            .get_template_text(PromptTemplateKey::ConversationSystem1)
            .await?;
        let system_prompt_2 = self
            .get_template_text(PromptTemplateKey::ConversationSystem2)
            .await?;
        let system_prompt_3 = self
            .get_template_text(PromptTemplateKey::ConversationSystem3)
            .await?;
        let welcome_user_prompt = self
            .get_template_text(PromptTemplateKey::ConversationWelcomeUser)
            .await?;

        if let Some(partner_id) = partner_id {
            if let Some(override_config) = self.get_partner_prompt_override(partner_id).await? {
                return Ok(PartnerConversationPromptConfigView {
                    partner_id,
                    enabled: true,
                    system_prompt_1: override_config.system_prompt_1,
                    system_prompt_2: override_config.system_prompt_2,
                    system_prompt_3: override_config.system_prompt_3,
                    welcome_user_prompt: override_config.welcome_user_prompt,
                    updated_by: Some(override_config.updated_by),
                    updated_at: Some(override_config.updated_at),
                    created_at: Some(override_config.created_at),
                    inherited_from_global: false,
                });
            }
            return Ok(PartnerConversationPromptConfigView {
                partner_id,
                enabled: false,
                system_prompt_1,
                system_prompt_2,
                system_prompt_3,
                welcome_user_prompt,
                updated_by: None,
                updated_at: None,
                created_at: None,
                inherited_from_global: true,
            });
        }

        Ok(PartnerConversationPromptConfigView {
            partner_id: 0,
            enabled: false,
            system_prompt_1,
            system_prompt_2,
            system_prompt_3,
            welcome_user_prompt,
            updated_by: None,
            updated_at: None,
            created_at: None,
            inherited_from_global: true,
        })
    }

    async fn append_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        turn_number: i32,
    ) -> AppResult<AgentMessage> {
        self.messages
            .append(&AgentMessage {
                id: 0,
                session_id: session_id.to_owned(),
                role,
                content: content.to_owned(),
                turn_number,
                created_at: self.clock.now(),
            })
            .await
    }

    async fn build_chat_messages(
        &self,
        session: &AgentSession,
        user_message: &str,
    ) -> AppResult<Vec<LlmRequestMessage>> {
        if user_message.trim().is_empty() {
            return Err(AppError::invalid_input("user_message is required"));
        }
        let prompt_context = self
            .characters
            .get_prompt_context(session.character_id, session.scene_id)
            .await?;
        let recent_turns = self
            .settings
            .get_recent_turns()
            .await
            .unwrap_or(DEFAULT_RECENT_TURNS);
        let mut messages = self
            .build_system_messages(&prompt_context, &session.timezone, None)
            .await?;
        messages.extend(
            self.messages
                .list_recent(&session.id, recent_turns)
                .await?
                .into_iter()
                .map(|message| LlmRequestMessage {
                    role: message.role.as_str().to_owned(),
                    content: message.content,
                }),
        );
        messages.push(LlmRequestMessage {
            role: MessageRole::User.as_str().to_owned(),
            content: user_message.to_owned(),
        });
        Ok(messages)
    }

    async fn build_system_messages(
        &self,
        prompt_context: &CharacterPromptContext,
        timezone: &str,
        partner_id: Option<i64>,
    ) -> AppResult<Vec<LlmRequestMessage>> {
        let prompts = self.resolve_conversation_prompt_bundle(partner_id).await?;
        [
            prompts.system_prompt_1,
            prompts.system_prompt_2,
            prompts.system_prompt_3,
        ]
        .into_iter()
        .map(|template_text| {
            Ok(LlmRequestMessage {
                role: MessageRole::System.as_str().to_owned(),
                content: self.build_prompt_from_template(
                    &template_text,
                    prompt_context,
                    timezone,
                )?,
            })
        })
        .collect::<AppResult<Vec<_>>>()
    }

    fn build_prompt_from_template(
        &self,
        template_text: &str,
        prompt_context: &CharacterPromptContext,
        timezone: &str,
    ) -> AppResult<String> {
        let mut prompt = normalize_template_text(template_text);
        reject_legacy_prompt_placeholders(&prompt)?;
        let scene = prompt_context.scene.as_ref();
        let scene_location = scene
            .map(|scene| scene.location.clone())
            .unwrap_or_default();
        let scene_user_role = scene
            .map(|scene| scene.user_role.clone())
            .unwrap_or_default();
        let scene_relationship = scene
            .map(|scene| scene.relationship.clone())
            .unwrap_or_default();
        let scene_environment = scene
            .map(|scene| scene.environment.clone())
            .unwrap_or_default();
        let scene_goal = scene.map(|scene| scene.goal.clone()).unwrap_or_default();
        let scene_opening_event = scene
            .map(|scene| scene.opening_event.clone())
            .unwrap_or_default();
        let replacements = [
            ("name", prompt_context.character.name.clone()),
            ("persona", prompt_context.character.persona.clone()),
            (
                "private_interests",
                prompt_context.character.private_interests.join(", "),
            ),
            (
                "personality_traits",
                prompt_context.character.personality_traits.clone(),
            ),
            (
                "speaking_style",
                prompt_context.character.speaking_style.clone(),
            ),
            ("occupation", prompt_context.character.occupation.clone()),
            (
                "marital_status",
                prompt_context.character.marital_status.clone(),
            ),
            ("language", prompt_context.character.language.clone()),
            (
                "relationship_stance",
                prompt_context.character.relationship_stance.clone(),
            ),
            ("age", prompt_context.character.age.to_string()),
            ("location", scene_location),
            ("user_role", scene_user_role),
            ("relationship", scene_relationship),
            ("environment", scene_environment),
            ("goal", scene_goal),
            ("opening_event", scene_opening_event),
            ("time", resolve_time_label(timezone, scene)?),
        ];
        for (key, value) in replacements {
            prompt = prompt.replace(&format!("{{{{{key}}}}}"), &value);
        }
        Ok(prompt)
    }

    fn build_request(
        &self,
        provider: &LlmProviderConfig,
        messages: Vec<LlmRequestMessage>,
        max_tokens: Option<i32>,
    ) -> AppResult<LlmCompletionRequest> {
        Ok(LlmCompletionRequest {
            // Tools are declared per persona; none are offered yet.
            tools: Vec::new(),
            endpoint_url: provider.endpoint_url.clone(),
            api_key: provider.api_key.clone(),
            model_name: provider.model_name.clone(),
            temperature: provider.temperature,
            frequency_penalty: provider.frequency_penalty,
            messages,
            max_tokens,
        })
    }

    async fn log_usage_completion(
        &self,
        session_id: &str,
        provider_key: ProviderKey,
        model_name: &str,
        result: &AppResult<crate::ports::LlmCompletionResponse>,
    ) -> AppResult<()> {
        let (prompt_tokens, completion_tokens, is_error, error_message) = match result {
            Ok(response) => (
                response.prompt_tokens,
                response.completion_tokens,
                false,
                String::new(),
            ),
            Err(err) => (0, 0, true, err.message.to_string()),
        };
        self.usage_logs
            .create(&LlmUsageLog {
                id: 0,
                session_id: session_id.to_owned(),
                provider_key,
                model_name: model_name.to_owned(),
                prompt_tokens,
                completion_tokens,
                is_error,
                error_message,
                created_at: self.clock.now(),
            })
            .await?;
        Ok(())
    }
}

fn reject_legacy_prompt_placeholders(template_text: &str) -> AppResult<()> {
    const LEGACY_PLACEHOLDERS: &[&str] = &[
        "{{name_zh}}",
        "{{name_en}}",
        "{{marital_status_zh}}",
        "{{marital_status_en}}",
        "{{occupation_zh}}",
        "{{occupation_en}}",
        "{{personality_zh}}",
        "{{personality_en}}",
        "{{personality}}",
        "{{description_zh}}",
        "{{description_en}}",
        "{{description}}",
        "{{interests_zh}}",
        "{{interests_en}}",
        "{{interests}}",
        "{{traits_zh}}",
        "{{traits_en}}",
        "{{traits}}",
        "{{sexual_preference}}",
        "{{user_sexual_preference}}",
    ];
    if let Some(placeholder) = LEGACY_PLACEHOLDERS
        .iter()
        .find(|placeholder| template_text.contains(**placeholder))
    {
        return Err(AppError::invalid_input(format!(
            "legacy prompt placeholder {placeholder} is not allowed"
        )));
    }
    Ok(())
}

fn normalize_template_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn resolve_time_label(
    timezone: &str,
    scene: Option<&character_context::ScenePromptProfile>,
) -> AppResult<String> {
    let Some(scene) = scene else {
        return Ok("未知".to_owned());
    };
    match scene.time_period_mode.as_str() {
        "fixed" => {
            return Ok(scene
                .time_period
                .clone()
                .unwrap_or_else(|| "未知".to_owned()));
        }
        "disabled" => return Ok("未知".to_owned()),
        _ => {}
    }
    let timezone = timezone.trim();
    if timezone.is_empty() {
        return Err(AppError::invalid_input("timezone is required"));
    }
    let tz: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| AppError::invalid_input("invalid timezone"))?;
    let hour = chrono::Utc::now().with_timezone(&tz).hour();
    Ok(match hour {
        0..=5 => "凌晨".to_owned(),
        6..=11 => "早上".to_owned(),
        12..=17 => "下午".to_owned(),
        _ => "夜里".to_owned(),
    })
}

use chrono::Timelike;

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use shared_kernel::AppResult;

    use super::*;
    use crate::domain::PromptTemplate;

    #[derive(Default)]
    struct StubProviders;
    #[derive(Default)]
    struct StubTemplates;
    #[derive(Default)]
    struct StubSessions;
    #[derive(Default)]
    struct StubMessages;
    #[derive(Default)]
    struct StubUsage;
    #[derive(Default)]
    struct StubGateway;
    #[derive(Default)]
    struct StubCharacters;
    #[derive(Default)]
    struct StubIds;
    #[derive(Default)]
    struct StubClock;
    #[derive(Default)]
    struct StubSettings;
    #[derive(Default)]
    struct StubPartnerPromptOverrides;

    #[async_trait]
    impl LlmProviderConfigRepository for StubProviders {
        async fn get_by_key(
            &self,
            provider_key: ProviderKey,
        ) -> AppResult<Option<LlmProviderConfig>> {
            Ok(Some(LlmProviderConfig {
                provider_key,
                endpoint_url: "https://example.com".into(),
                api_key: "key".into(),
                model_name: "model".into(),
                temperature: 0.7,
                frequency_penalty: 0.0,
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn list_all(&self) -> AppResult<Vec<LlmProviderConfig>> {
            Ok(Vec::new())
        }
        async fn upsert(&self, config: &LlmProviderConfig) -> AppResult<LlmProviderConfig> {
            Ok(config.clone())
        }
    }

    #[async_trait]
    impl PromptTemplateRepository for StubTemplates {
        async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>> {
            Ok(Some(PromptTemplate {
                id: 1,
                template_key: key,
                template_text: "你好 {{name}}".into(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn list_all(&self) -> AppResult<Vec<PromptTemplate>> {
            Ok(Vec::new())
        }
        async fn upsert(&self, template: &PromptTemplate) -> AppResult<PromptTemplate> {
            Ok(template.clone())
        }
    }

    #[async_trait]
    impl PartnerConversationPromptOverrideRepository for StubPartnerPromptOverrides {
        async fn get_by_partner_id(
            &self,
            _partner_id: i64,
        ) -> AppResult<Option<PartnerConversationPromptOverride>> {
            Ok(None)
        }

        async fn upsert(
            &self,
            config: &PartnerConversationPromptOverride,
        ) -> AppResult<PartnerConversationPromptOverride> {
            Ok(config.clone())
        }

        async fn delete_by_partner_id(&self, _partner_id: i64) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AgentSessionRepository for StubSessions {
        async fn create(&self, session: &AgentSession) -> AppResult<AgentSession> {
            Ok(session.clone())
        }
        async fn get_by_id(&self, session_id: &str) -> AppResult<Option<AgentSession>> {
            Ok(Some(AgentSession {
                id: session_id.to_owned(),
                caller: AgentCallerIdentity::PlatformUser { user_id: 1 },
                character_id: 1,
                timezone: "Asia/Shanghai".into(),
                scene_id: Some(1),
                started_at: chrono::Utc::now(),
                ended_at: None,
            }))
        }
        async fn end(&self, _session_id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl AgentMessageRepository for StubMessages {
        async fn append(&self, message: &AgentMessage) -> AppResult<AgentMessage> {
            Ok(message.clone())
        }
        async fn list_recent(
            &self,
            _session_id: &str,
            _recent_turns: i32,
        ) -> AppResult<Vec<AgentMessage>> {
            Ok(Vec::new())
        }
        async fn list_all(&self, _session_id: &str) -> AppResult<Vec<AgentMessage>> {
            Ok(Vec::new())
        }
        async fn next_turn_number(&self, _session_id: &str) -> AppResult<i32> {
            Ok(1)
        }
    }

    #[async_trait]
    impl UsageLogRepository for StubUsage {
        async fn get_usage_stats(&self) -> AppResult<crate::domain::LlmUsageStats> {
            Ok(crate::domain::LlmUsageStats::default())
        }

        async fn create(&self, log: &LlmUsageLog) -> AppResult<LlmUsageLog> {
            Ok(log.clone())
        }
        async fn summarize_session(
            &self,
            _session_id: &str,
        ) -> AppResult<crate::domain::SessionUsageSummary> {
            Ok(crate::domain::SessionUsageSummary::default())
        }
    }

    #[async_trait]
    impl LlmGateway for StubGateway {
        async fn stream(
            &self,
            _request: LlmCompletionRequest,
        ) -> AppResult<crate::ports::LlmStream> {
            use crate::ports::{LlmDelta, LlmUsage};
            // Two tokens, so a consumer that assumes one chunk per reply fails
            // here rather than in a call.
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(LlmDelta::Token("hel".into())),
                Ok(LlmDelta::Token("lo".into())),
                Ok(LlmDelta::Done(LlmUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                })),
            ])))
        }
    }

    #[async_trait]
    impl CharacterPromptContextReadPort for StubCharacters {
        async fn get_prompt_context(
            &self,
            character_id: i64,
            selected_scene_id: Option<i64>,
        ) -> AppResult<CharacterPromptContext> {
            Ok(CharacterPromptContext {
                character: character_context::CharacterPromptProfile {
                    character_id,
                    language: "zh".into(),
                    relationship_stance: "a warm, steady presence".into(),
                    name: "阿明".into(),
                    age: 20,
                    marital_status: "未婚".into(),
                    occupation: "学生".into(),
                    persona: "描述".into(),
                    private_interests: vec!["聊天".into()],
                    personality_traits: "温柔".into(),
                    speaking_style: "轻声细语".into(),
                },
                scene: Some(character_context::ScenePromptProfile {
                    scene_id: selected_scene_id.unwrap_or(1),
                    location: "客厅".into(),
                    user_role: "朋友".into(),
                    relationship: "亲密".into(),
                    environment: "安静".into(),
                    goal: "聊天".into(),
                    opening_event: "刚见面".into(),
                    time_period_mode: "auto".into(),
                    time_period: None,
                }),
            })
        }
    }

    impl IdGenerator for StubIds {
        fn next_session_id(&self) -> String {
            "session-1".into()
        }
    }

    impl Clock for StubClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::Utc::now()
        }
    }

    #[async_trait]
    impl AgentSettingsRepository for StubSettings {
        async fn get_recent_turns(&self) -> AppResult<i32> {
            Ok(6)
        }
        async fn set_recent_turns(&self, _recent_turns: i32) -> AppResult<()> {
            Ok(())
        }
    }

    #[test]
    fn prompt_write_rejects_legacy_character_placeholders() {
        let result = required_prompt(Some("你好 {{name_zh}}"), "system_prompt_1");

        assert!(result.is_err());
    }

    #[test]
    fn prompt_render_rejects_legacy_character_placeholders() {
        let service = AgentService::new(AgentDependencies {
            providers: StubProviders,
            templates: StubTemplates,
            partner_prompt_overrides: StubPartnerPromptOverrides,
            sessions: StubSessions,
            messages: StubMessages,
            usage_logs: StubUsage,
            gateway: StubGateway,
            characters: StubCharacters,
            ids: StubIds,
            clock: StubClock,
            settings: Box::new(StubSettings),
        });
        let prompt_context = CharacterPromptContext {
            character: character_context::CharacterPromptProfile {
                character_id: 1,
                language: "zh".into(),
                relationship_stance: "a warm, steady presence".into(),
                name: "阿明".into(),
                age: 20,
                marital_status: "未婚".into(),
                occupation: "学生".into(),
                persona: "描述".into(),
                private_interests: vec!["聊天".into()],
                personality_traits: "温柔".into(),
                speaking_style: "轻声细语".into(),
            },
            scene: Some(character_context::ScenePromptProfile {
                scene_id: 1,
                location: "客厅".into(),
                user_role: "朋友".into(),
                relationship: "亲密".into(),
                environment: "安静".into(),
                goal: "聊天".into(),
                opening_event: "刚见面".into(),
                time_period_mode: "auto".into(),
                time_period: None,
            }),
        };

        let result = service.build_prompt_from_template(
            "你好 {{name_zh}}",
            &prompt_context,
            "Asia/Shanghai",
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_session_requires_ids() {
        let service = AgentService::new(AgentDependencies {
            providers: StubProviders,
            templates: StubTemplates,
            partner_prompt_overrides: StubPartnerPromptOverrides,
            sessions: StubSessions,
            messages: StubMessages,
            usage_logs: StubUsage,
            gateway: StubGateway,
            characters: StubCharacters,
            ids: StubIds,
            clock: StubClock,
            settings: Box::new(StubSettings),
        });

        let result = service
            .create_session(CreateSessionCommand {
                caller: AgentCallerIdentity::PlatformUser { user_id: 0 },
                character_id: 1,
                timezone: "Asia/Shanghai".into(),
                scene_id: Some(1),
            })
            .await;

        assert!(result.is_err());
    }
}
