//! Endpointing parameters, read from `sonari.toml`.
//!
//! These decide when a turn starts and when it ends. They are tuned by ear
//! against real calls, so they belong where a change is a diff that can be
//! reviewed and rolled back — not a row somebody edited once.

use async_trait::async_trait;
use shared_kernel::AppResult;
use sonari_config::SettingsHandle;
use speech_runtime::{SpeechSegmentationConfig, SpeechSegmentationConfigPort};

pub struct ConfigEndpointing {
    settings: SettingsHandle,
}

impl ConfigEndpointing {
    pub fn new(settings: SettingsHandle) -> Self {
        Self { settings }
    }
}

#[async_trait]
impl SpeechSegmentationConfigPort for ConfigEndpointing {
    async fn get_speech_segmentation_config(&self) -> AppResult<SpeechSegmentationConfig> {
        let settings = self.settings.get();
        let endpointing = &settings.endpointing;
        Ok(SpeechSegmentationConfig {
            min_utterance_ms: endpointing.min_utterance_ms,
            silence_flush_ms: endpointing.silence_flush_ms,
            silence_force_agent_ms: endpointing.silence_force_agent_ms,
            min_speech_confirm_ms: endpointing.min_speech_confirm_ms,
            // Compares PCM amplitude directly, which means nothing against a
            // neural detector's speech probability. The field survives because
            // the policy still reads it; replacing that policy is what makes it
            // go away.
            voice_activity_threshold: 0,
        })
    }
}
