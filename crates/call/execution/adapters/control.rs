use async_trait::async_trait;
use call_runtime_context::{
    RuntimeAsrLanguage, RuntimeSessionContext, RuntimeSessionContextPort, RuntimeSpeechReadiness,
};

use call::{CallStatus, RuntimeSessionState, RuntimeSessionStatePort, RuntimeWorkStatus};
use shared_kernel::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct ControlRuntimeContextAdapter<S> {
    state: S,
}

impl<S> ControlRuntimeContextAdapter<S> {
    pub fn new(state: S) -> Self {
        Self { state }
    }
}

#[async_trait]
impl<S> RuntimeSessionContextPort for ControlRuntimeContextAdapter<S>
where
    S: RuntimeSessionStatePort + Send + Sync,
{
    async fn get_runtime_session_context(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionContext>> {
        self.state
            .get_runtime_session_state(session_id)
            .await?
            .map(map_runtime_context)
            .transpose()
    }
}

fn map_runtime_context(state: RuntimeSessionState) -> AppResult<RuntimeSessionContext> {
    Ok(RuntimeSessionContext {
        session_id: state.session_id,
        agent_session_id: state.agent_session_id,
        voice: state.voice,
        asr_language: match state.asr_language {
            call::SpeechInputLanguage::Zh => RuntimeAsrLanguage::Zh,
            call::SpeechInputLanguage::En => RuntimeAsrLanguage::En,
        },
        runtime_owner_id: state.runtime_owner_id,
        speech_readiness: execution_speech_readiness(state.status, state.runtime_status)?,
    })
}

fn execution_speech_readiness(
    status: CallStatus,
    runtime_status: RuntimeWorkStatus,
) -> AppResult<RuntimeSpeechReadiness> {
    match (status, runtime_status) {
        (CallStatus::Starting, RuntimeWorkStatus::StartClaimed)
        | (CallStatus::Active, RuntimeWorkStatus::Ready) => {
            Ok(RuntimeSpeechReadiness::AcceptingInput)
        }
        (CallStatus::Ending, _)
        | (CallStatus::Ended, _)
        | (CallStatus::Failed, _)
        | (_, RuntimeWorkStatus::Failed)
        | (_, RuntimeWorkStatus::StopRequested)
        | (_, RuntimeWorkStatus::StopClaimed)
        | (_, RuntimeWorkStatus::Stopped) => Ok(RuntimeSpeechReadiness::RejectingInput),
        _ => Err(AppError::invalid_input(
            "runtime speech is not available for the current call state",
        )),
    }
}
