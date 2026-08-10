use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservabilityErrorKind {
    InvalidInput,
    Internal,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ObservabilityError {
    pub kind: ObservabilityErrorKind,
    pub message: String,
}

impl ObservabilityError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            kind: ObservabilityErrorKind::InvalidInput,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ObservabilityErrorKind::Internal,
            message: message.into(),
        }
    }
}

pub type ObservabilityResult<T> = Result<T, ObservabilityError>;
