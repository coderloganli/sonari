use axum::response::IntoResponse;
use serde::Serialize;

use crate::response::ok;

#[derive(Debug, Serialize)]
pub struct HealthPayload {
    pub status: &'static str,
    pub service: &'static str,
}

pub async fn healthz() -> impl IntoResponse {
    ok(HealthPayload {
        status: "ok",
        service: "backend",
    })
}
