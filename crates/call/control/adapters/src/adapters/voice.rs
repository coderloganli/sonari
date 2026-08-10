use std::sync::Arc;

use async_trait::async_trait;
use call::{SpeechInputLanguage, ports::CallInputLanguagePort};

#[derive(Clone)]
pub struct InputLanguageAdapter {
    voice: Arc<dyn voice::VoiceCallConfigUseCases>,
}

impl InputLanguageAdapter {
    pub fn new(voice: Arc<dyn voice::VoiceCallConfigUseCases>) -> Self {
        Self { voice }
    }
}

#[async_trait]
impl CallInputLanguagePort for InputLanguageAdapter {
    async fn get_input_language(&self) -> shared_kernel::AppResult<SpeechInputLanguage> {
        let language = self.voice.get_asr_input_language().await?;
        Ok(match language {
            voice::AsrLanguage::Zh => SpeechInputLanguage::Zh,
            voice::AsrLanguage::En => SpeechInputLanguage::En,
        })
    }
}
