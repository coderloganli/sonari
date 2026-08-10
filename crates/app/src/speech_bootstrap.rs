//! Assembles the per-session configuration dispatch hands to the media plane.
//!
//! Inference runs on models loaded by this process, so nothing about providers,
//! endpoints or credentials travels with a session. What remains is the session
//! context and the endpointing parameters.

use std::sync::Arc;

use agent::ports::LlmProviderConfigRepository;
use async_trait::async_trait;
use call_execution::SpeechBootstrapComposerPort;
use call_runtime_control::{LlmSelectionSpec, SegmentationConfigSpec, SpeechRuntimeBootstrap};
use shared_kernel::{AppError, AppResult};
use speech_runtime::SpeechSegmentationConfigPort;

pub struct DbSpeechBootstrapComposer {
    segmentation: Arc<dyn SpeechSegmentationConfigPort>,
    llm_providers: Arc<dyn LlmProviderConfigRepository>,
}

impl DbSpeechBootstrapComposer {
    pub fn new(
        segmentation: Arc<dyn SpeechSegmentationConfigPort>,
        llm_providers: Arc<dyn LlmProviderConfigRepository>,
    ) -> Self {
        Self {
            segmentation,
            llm_providers,
        }
    }
}

#[async_trait]
impl SpeechBootstrapComposerPort for DbSpeechBootstrapComposer {
    async fn compose(
        &self,
        voice: String,
        agent_session_id: &str,
        language: &str,
    ) -> AppResult<Option<SpeechRuntimeBootstrap>> {
        let segmentation = self.segmentation.get_speech_segmentation_config().await?;

        let llm = self
            .llm_providers
            .get_by_key(agent::ProviderKey::Conversation)
            .await?
            .ok_or_else(|| AppError::internal("conversation llm provider config not found"))?;

        Ok(Some(SpeechRuntimeBootstrap {
            agent_session_id: agent_session_id.to_owned(),
            voice,
            language: language.to_owned(),
            segmentation: SegmentationConfigSpec {
                min_utterance_ms: segmentation.min_utterance_ms,
                silence_flush_ms: segmentation.silence_flush_ms,
                silence_force_agent_ms: segmentation.silence_force_agent_ms,
                voice_activity_threshold: segmentation.voice_activity_threshold,
                min_speech_confirm_ms: segmentation.min_speech_confirm_ms,
            },
            llm: LlmSelectionSpec {
                provider_key: llm.provider_key.as_str().to_owned(),
                endpoint: llm.endpoint_url,
                model: llm.model_name,
                temperature: llm.temperature,
                frequency_penalty: llm.frequency_penalty,
            },
        }))
    }
}
