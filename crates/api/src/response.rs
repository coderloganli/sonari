use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    code: i32,
    message: &'static str,
    data: T,
}

pub fn ok<T: Serialize>(data: T) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(ApiResponse {
            code: 0,
            message: "ok",
            data,
        }),
    )
}
