use std::sync::Arc;

use async_trait::async_trait;
use call_speech_contract::{BotSpeechRuntimeTriggerPort, speech_session_id_for_call};
use shared_kernel::AppResult;

use crate::{SpeechRuntimeUseCases, SubmitPreRecordedTurnCommand, SubmitServerTurnCommand};

#[derive(Clone)]
pub struct BotSpeechRuntimeTriggerAdapter {
    speech_runtime: Arc<dyn SpeechRuntimeUseCases>,
}

impl BotSpeechRuntimeTriggerAdapter {
    pub fn new(speech_runtime: Arc<dyn SpeechRuntimeUseCases>) -> Self {
        Self { speech_runtime }
    }
}

#[async_trait]
impl BotSpeechRuntimeTriggerPort for BotSpeechRuntimeTriggerAdapter {
    async fn submit_server_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
        reply_text: &str,
        interruptible: bool,
    ) -> AppResult<()> {
        self.speech_runtime
            .submit_server_turn(SubmitServerTurnCommand {
                speech_session_id: speech_session_id_for_call(session_id),
                runtime_owner_id: runtime_owner_id.to_owned(),
                round_id: round_id.to_owned(),
                reply_text: reply_text.to_owned(),
                interruptible,
            })
            .await?;
        Ok(())
    }

    async fn submit_pre_recorded_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
        audio_url: &str,
        band: &str,
        interruptible: bool,
    ) -> AppResult<()> {
        self.speech_runtime
            .submit_pre_recorded_turn(SubmitPreRecordedTurnCommand {
                speech_session_id: speech_session_id_for_call(session_id),
                runtime_owner_id: runtime_owner_id.to_owned(),
                round_id: round_id.to_owned(),
                audio_url: audio_url.to_owned(),
                band: band.to_owned(),
                interruptible,
            })
            .await?;
        Ok(())
    }
}
