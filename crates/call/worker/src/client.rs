//! In-process control-plane access.
//!
//! The orchestration loop runs inside the server process, so every call here is
//! a direct call into `call_execution` or `speech_runtime`. The type is kept as
//! a client so that the pipeline, which is written against it, is unchanged.

use std::{error::Error as StdError, fmt, sync::Arc};

use anyhow::{Result, anyhow};
use call_execution::{
    CallExecutionUseCases, PollRuntimeWorkCommand, PublishRuntimeEventCommand,
    ReportRuntimeStatusCommand, RuntimeStatusKind,
};
use call_runtime_control::{
    RuntimeEventFact, RuntimeLaunchSpec, RuntimeWorkItem, SpeechInputMediaState,
};
use shared_kernel::{AppError, AppErrorCode};
use speech_runtime::{
    CloseSpeechSessionCommand, CreateSpeechSessionCommand, FailOwnerRouteCommand,
    InterruptSpeechSessionCommand, PollSpeechEventsCommand, PushSpeechInputCommand,
    SpeechRuntimeUseCases,
};

pub type WorkerRuntimeLaunch = RuntimeLaunchSpec;

/// The event type is the runtime's own — there is no serialization boundary to
/// mirror it across.
pub use speech_runtime::SpeechRuntimeEvent;

#[derive(Debug)]
pub struct RuntimeControlError {
    operation: &'static str,
    error: AppError,
}

impl RuntimeControlError {
    fn new(operation: &'static str, error: AppError) -> Self {
        Self { operation, error }
    }

    /// Transient conditions are worth retrying; a rejected command is not.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.error.code,
            AppErrorCode::Unavailable | AppErrorCode::Internal
        )
    }

    pub fn is_not_found_for(&self, operation: &'static str) -> bool {
        self.operation == operation && self.error.code == AppErrorCode::NotFound
    }

    pub fn is_bad_request_containing(&self, operation: &'static str, needle: &str) -> bool {
        self.operation == operation
            && self.error.code == AppErrorCode::InvalidInput
            && self.error.message.contains(needle)
    }
}

impl fmt::Display for RuntimeControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed ({:?}): {}",
            self.operation, self.error.code, self.error.message
        )
    }
}

impl StdError for RuntimeControlError {}

fn wrap(operation: &'static str) -> impl FnOnce(AppError) -> anyhow::Error {
    move |error| anyhow!(RuntimeControlError::new(operation, error))
}

pub fn is_retryable_control_plane_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|source| source.downcast_ref::<RuntimeControlError>())
        .is_some_and(RuntimeControlError::is_retryable)
}

pub fn is_not_found_control_plane_error(error: &anyhow::Error, operation: &'static str) -> bool {
    error
        .chain()
        .find_map(|source| source.downcast_ref::<RuntimeControlError>())
        .is_some_and(|error| error.is_not_found_for(operation))
}

pub fn is_push_input_not_accepting_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|source| source.downcast_ref::<RuntimeControlError>())
        .is_some_and(|error| {
            error.is_bad_request_containing(
                "push speech input",
                "runtime speech session is not currently accepting input",
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusRequest {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub status: String,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSpeechSessionRequest {
    pub session_id: i64,
    pub runtime_owner_id: String,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
}

pub type CreateSpeechSessionResponse = speech_runtime::CreateSpeechSessionResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailOwnerRouteRequest {
    pub runtime_owner_id: String,
    pub owner_backend_url: String,
    pub owner_instance_id: String,
    pub owner_instance_epoch: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSpeechInputRequest {
    pub runtime_owner_id: String,
    pub pcm_s16le: Vec<i16>,
    pub sample_rate_hz: u32,
    pub num_channels: u16,
    pub media_state: SpeechInputMediaState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechEventsResponse {
    pub events: Vec<SpeechRuntimeEvent>,
}

#[derive(Clone)]
pub struct RuntimeControlClient {
    execution: Arc<dyn CallExecutionUseCases>,
    speech: Arc<dyn SpeechRuntimeUseCases>,
}

impl RuntimeControlClient {
    pub fn new(
        execution: Arc<dyn CallExecutionUseCases>,
        speech: Arc<dyn SpeechRuntimeUseCases>,
    ) -> Self {
        Self { execution, speech }
    }

    /// Retained for the pipeline's owner-route handling. With one process there
    /// is one owner, so re-targeting is a no-op.
    pub fn with_base_url(&self, _base_url: String) -> Self {
        self.clone()
    }

    pub async fn poll_work(&self, runtime_owner_id: &str) -> Result<Option<RuntimeWorkItem>> {
        self.execution
            .poll_runtime_work(PollRuntimeWorkCommand {
                runtime_owner_id: runtime_owner_id.to_owned(),
            })
            .await
            .map_err(wrap("poll runtime work"))
    }

    pub async fn report_status(&self, request: RuntimeStatusRequest) -> Result<()> {
        let status = RuntimeStatusKind::parse(&request.status).ok_or_else(|| {
            anyhow!(RuntimeControlError::new(
                "report runtime status",
                AppError::invalid_input(format!("unknown runtime status: {}", request.status)),
            ))
        })?;
        self.execution
            .report_runtime_status(ReportRuntimeStatusCommand {
                session_id: request.session_id,
                runtime_owner_id: request.runtime_owner_id,
                status,
                failure_reason: request.failure_reason,
            })
            .await
            .map_err(wrap("report runtime status"))
    }

    pub async fn publish_event(&self, request: RuntimeEventFact) -> Result<()> {
        self.execution
            .publish_runtime_event(PublishRuntimeEventCommand { fact: request })
            .await
            .map_err(wrap("publish runtime event"))
    }

    pub async fn create_speech_session(
        &self,
        request: CreateSpeechSessionRequest,
    ) -> Result<CreateSpeechSessionResponse> {
        self.speech
            .create_session(CreateSpeechSessionCommand {
                session_id: request.session_id,
                runtime_owner_id: request.runtime_owner_id,
                sample_rate_hz: request.sample_rate_hz,
                num_channels: request.num_channels,
            })
            .await
            .map_err(wrap("create speech session"))
    }

    pub async fn push_speech_input(
        &self,
        speech_session_id: &str,
        request: PushSpeechInputRequest,
    ) -> Result<()> {
        self.speech
            .push_input_audio(PushSpeechInputCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: request.runtime_owner_id,
                pcm_s16le: request.pcm_s16le,
                sample_rate_hz: request.sample_rate_hz,
                num_channels: request.num_channels,
                media_state: request.media_state,
            })
            .await
            .map_err(wrap("push speech input"))
    }

    pub async fn fail_owner_route(
        &self,
        speech_session_id: &str,
        request: FailOwnerRouteRequest,
    ) -> Result<()> {
        self.speech
            .fail_owner_route(FailOwnerRouteCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: request.runtime_owner_id,
                owner_backend_url: request.owner_backend_url,
                owner_instance_id: request.owner_instance_id,
                owner_instance_epoch: request.owner_instance_epoch,
                reason: request.reason,
            })
            .await
            .map_err(wrap("fail owner route"))
    }

    pub async fn poll_speech_events(
        &self,
        speech_session_id: &str,
        runtime_owner_id: &str,
        max_events: usize,
    ) -> Result<SpeechEventsResponse> {
        let result = self
            .speech
            .poll_events(PollSpeechEventsCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: runtime_owner_id.to_owned(),
                max_events,
            })
            .await
            .map_err(wrap("poll speech events"))?;
        Ok(SpeechEventsResponse {
            events: result.events,
        })
    }

    pub async fn close_speech_session(
        &self,
        speech_session_id: &str,
        runtime_owner_id: &str,
    ) -> Result<SpeechEventsResponse> {
        let result = self
            .speech
            .close_session(CloseSpeechSessionCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: runtime_owner_id.to_owned(),
            })
            .await
            .map_err(wrap("close speech session"))?;
        Ok(SpeechEventsResponse {
            events: result.events,
        })
    }

    pub async fn interrupt_speech_session(
        &self,
        speech_session_id: &str,
        runtime_owner_id: &str,
    ) -> Result<SpeechEventsResponse> {
        let result = self
            .speech
            .interrupt_session(InterruptSpeechSessionCommand {
                speech_session_id: speech_session_id.to_owned(),
                runtime_owner_id: runtime_owner_id.to_owned(),
            })
            .await
            .map_err(wrap("interrupt speech session"))?;
        Ok(SpeechEventsResponse {
            events: result.events,
        })
    }
}
