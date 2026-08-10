use std::sync::Arc;

use async_trait::async_trait;
use shared_kernel::AppResult;

pub fn speech_session_id_for_call(session_id: i64) -> String {
    format!("call-{session_id}")
}

#[async_trait]
pub trait BotSpeechRuntimeTriggerPort: Send + Sync {
    async fn submit_server_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
        reply_text: &str,
        interruptible: bool,
    ) -> AppResult<()>;

    async fn submit_pre_recorded_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
        audio_url: &str,
        band: &str,
        interruptible: bool,
    ) -> AppResult<()>;
}

#[async_trait]
impl<T> BotSpeechRuntimeTriggerPort for Arc<T>
where
    T: BotSpeechRuntimeTriggerPort + Send + Sync + ?Sized,
{
    async fn submit_server_turn(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
        round_id: &str,
        reply_text: &str,
        interruptible: bool,
    ) -> AppResult<()> {
        (**self)
            .submit_server_turn(
                session_id,
                runtime_owner_id,
                round_id,
                reply_text,
                interruptible,
            )
            .await
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
        (**self)
            .submit_pre_recorded_turn(
                session_id,
                runtime_owner_id,
                round_id,
                audio_url,
                band,
                interruptible,
            )
            .await
    }
}
