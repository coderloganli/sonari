use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use shared_kernel::{AppError, AppErrorCode};

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: i32,
    message: String,
}

#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            AppErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
            AppErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            AppErrorCode::Forbidden => StatusCode::FORBIDDEN,
            AppErrorCode::NotFound => StatusCode::NOT_FOUND,
            AppErrorCode::Conflict => StatusCode::CONFLICT,
            AppErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            AppErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (
            status,
            Json(ErrorResponse {
                code: 1,
                message: self.0.message.into_owned(),
            }),
        )
            .into_response()
    }
}
