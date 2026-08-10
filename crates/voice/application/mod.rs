//! What the call path asks of the voice module.
//!
//! Inference itself is reached through `VoiceRuntimeExecutionPort`; this is only
//! the configuration the call path needs before a session opens. It shrinks
//! further once personas move into configuration.

use async_trait::async_trait;
use shared_kernel::AppResult;

use crate::domain::AsrLanguage;
use crate::ports::VoiceConfigRepository;

#[async_trait]
pub trait VoiceCallConfigUseCases: Send + Sync {
    async fn get_asr_input_language(&self) -> AppResult<AsrLanguage>;
}

pub struct VoiceCallConfigService<C> {
    config: C,
}

impl<C> VoiceCallConfigService<C> {
    pub fn new(config: C) -> Self {
        Self { config }
    }
}

#[async_trait]
impl<C> VoiceCallConfigUseCases for VoiceCallConfigService<C>
where
    C: VoiceConfigRepository + Send + Sync,
{
    async fn get_asr_input_language(&self) -> AppResult<AsrLanguage> {
        self.config.get_asr_input_language().await
    }
}
