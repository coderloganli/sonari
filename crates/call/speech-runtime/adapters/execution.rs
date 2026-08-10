use async_trait::async_trait;
use call_runtime_context::{RuntimeAsrLanguage, RuntimeSessionContextPort, RuntimeSpeechReadiness};
use shared_kernel::{AppError, AppResult};

use crate::application::{SpeechRuntimeContext, SpeechRuntimeContextPort};

pub struct ExecutionRuntimeContextAdapter<P> {
    inner: P,
}

impl<P> ExecutionRuntimeContextAdapter<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<P> SpeechRuntimeContextPort for ExecutionRuntimeContextAdapter<P>
where
    P: RuntimeSessionContextPort + Send + Sync,
{
    async fn resolve_accepting_runtime_context(
        &self,
        session_id: i64,
        runtime_owner_id: &str,
    ) -> AppResult<Option<SpeechRuntimeContext>> {
        let Some(context) = self.inner.get_runtime_session_context(session_id).await? else {
            return Ok(None);
        };
        if context.runtime_owner_id.as_deref() != Some(runtime_owner_id) {
            return Err(AppError::forbidden("runtime owner mismatch"));
        }
        if !matches!(
            context.speech_readiness,
            RuntimeSpeechReadiness::AcceptingInput
        ) {
            return Err(AppError::invalid_input(
                "runtime speech session is not currently accepting input",
            ));
        }
        Ok(Some(SpeechRuntimeContext {
            session_id: context.session_id,
            agent_session_id: context.agent_session_id,
            voice: context.voice,
            runtime_owner_id: runtime_owner_id.to_owned(),
            language: match context.asr_language {
                RuntimeAsrLanguage::Zh => voice::AsrLanguage::Zh,
                RuntimeAsrLanguage::En => voice::AsrLanguage::En,
            },
        }))
    }
}
