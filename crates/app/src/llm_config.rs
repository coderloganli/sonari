//! Where the LLM lives and how it should behave.
//!
//! The split follows what each value is. The endpoint is deployment topology —
//! it points at a container in one environment and a host in another — and the
//! key is a secret, so both come from the environment. The model and its
//! sampling parameters are behaviour, tuned alongside the prompt, so they live
//! in `sonari.toml` where they can be reviewed and rolled back.

use agent::ports::LlmProviderConfigRepository;
use agent::{LlmProviderConfig, ProviderKey};
use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use sonari_config::SettingsHandle;

/// Reads the endpoint and key once at startup: they are deployment facts, not
/// something an operator edits between calls.
#[derive(Debug, Clone)]
pub struct LlmEndpoint {
    pub base_url: String,
    pub api_key: String,
}

impl LlmEndpoint {
    pub fn from_env() -> AppResult<Self> {
        let base_url = std::env::var("LLM_BASE_URL").unwrap_or_default();
        if base_url.trim().is_empty() {
            return Err(AppError::invalid_input("LLM_BASE_URL must be configured"));
        }
        Ok(Self {
            base_url: base_url.trim().to_owned(),
            // A self-hosted endpoint usually wants no key. An empty one is sent
            // as an empty bearer token, which such endpoints ignore.
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
        })
    }
}

pub struct ConfigLlmProviders {
    settings: SettingsHandle,
    endpoint: LlmEndpoint,
}

impl ConfigLlmProviders {
    pub fn new(settings: SettingsHandle, endpoint: LlmEndpoint) -> Self {
        Self { settings, endpoint }
    }

    fn current(&self, provider_key: ProviderKey) -> AppResult<LlmProviderConfig> {
        let settings = self.settings.get();
        if settings.llm.model.trim().is_empty() {
            return Err(AppError::invalid_input(
                "llm.model must be set in the configuration file",
            ));
        }
        Ok(LlmProviderConfig {
            provider_key,
            endpoint_url: self.endpoint.base_url.clone(),
            api_key: self.endpoint.api_key.clone(),
            model_name: settings.llm.model.clone(),
            temperature: settings.llm.temperature,
            frequency_penalty: settings.llm.frequency_penalty,
            updated_at: chrono::Utc::now(),
        })
    }
}

#[async_trait]
impl LlmProviderConfigRepository for ConfigLlmProviders {
    async fn get_by_key(&self, provider_key: ProviderKey) -> AppResult<Option<LlmProviderConfig>> {
        self.current(provider_key).map(Some)
    }

    async fn list_all(&self) -> AppResult<Vec<LlmProviderConfig>> {
        Ok(vec![self.current(ProviderKey::Conversation)?])
    }

    async fn upsert(&self, _config: &LlmProviderConfig) -> AppResult<LlmProviderConfig> {
        Err(AppError::invalid_input(
            "llm settings are configuration; edit sonari.toml instead",
        ))
    }
}
