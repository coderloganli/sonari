use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAsrLanguage {
    Zh,
    En,
}

impl RuntimeAsrLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSpeechReadiness {
    AcceptingInput,
    RejectingInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionContext {
    pub session_id: i64,
    pub agent_session_id: String,
    /// The synthesis voice this session speaks with.
    pub voice: String,
    pub asr_language: RuntimeAsrLanguage,
    pub runtime_owner_id: Option<String>,
    pub speech_readiness: RuntimeSpeechReadiness,
}

#[async_trait]
pub trait RuntimeSessionContextPort: Send + Sync {
    async fn get_runtime_session_context(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionContext>>;
}

#[async_trait]
impl<T> RuntimeSessionContextPort for std::sync::Arc<T>
where
    T: RuntimeSessionContextPort + Send + Sync + ?Sized,
{
    async fn get_runtime_session_context(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionContext>> {
        (**self).get_runtime_session_context(session_id).await
    }
}
