//! Prompt templates, read from `sonari.toml`.
//!
//! These are the instructions wrapped around a persona — how long to speak, that
//! this is a voice call, how to use the scene. They are edited together with the
//! persona and belong in the same file.
//!
//! A missing template used to resolve to an empty string, which produced a
//! working conversation with no system prompt at all: the agent answered, but as
//! nobody in particular. Startup now refuses that.

use agent::ports::PromptTemplateRepository;
use agent::{PromptTemplate, PromptTemplateKey};
use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use sonari_config::SettingsHandle;

pub struct ConfigPromptTemplates {
    settings: SettingsHandle,
}

impl ConfigPromptTemplates {
    pub fn new(settings: SettingsHandle) -> Self {
        Self { settings }
    }

    fn text(&self, key: PromptTemplateKey) -> Option<String> {
        let settings = self.settings.get();
        let prompts = &settings.prompts;
        let text = match key {
            PromptTemplateKey::ConversationSystem1 => &prompts.conversation_system,
            PromptTemplateKey::ConversationSystem2 => &prompts.character,
            PromptTemplateKey::ConversationSystem3 => &prompts.scene,
            PromptTemplateKey::ConversationWelcomeUser => &prompts.welcome,
            // Only the conversation path is served; nothing else calls this.
            PromptTemplateKey::AssistantSystem => return None,
        };
        Some(text.clone())
    }
}

#[async_trait]
impl PromptTemplateRepository for ConfigPromptTemplates {
    async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>> {
        Ok(self.text(key).map(|template_text| PromptTemplate {
            id: 0,
            template_key: key,
            template_text,
            updated_at: chrono::Utc::now(),
        }))
    }

    async fn list_all(&self) -> AppResult<Vec<PromptTemplate>> {
        Ok(Vec::new())
    }

    async fn upsert(&self, _template: &PromptTemplate) -> AppResult<PromptTemplate> {
        Err(AppError::invalid_input(
            "prompts are configuration; edit sonari.toml instead",
        ))
    }
}
