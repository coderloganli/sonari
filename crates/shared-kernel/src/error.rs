use std::borrow::Cow;

use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppErrorCode {
    InvalidInput,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct AppError {
    pub code: AppErrorCode,
    pub message: Cow<'static, str>,
}

impl AppError {
    pub fn new(code: AppErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_input(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::InvalidInput, message)
    }

    pub fn unauthorized(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::Unauthorized, message)
    }

    pub fn forbidden(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::Forbidden, message)
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::Conflict, message)
    }

    pub fn unavailable(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::Unavailable, message)
    }

    pub fn internal(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(AppErrorCode::Internal, message)
    }
}
