use std::sync::Arc;

use auth::ports::TokenService;
use axum::{Router, middleware, routing::get};
use call::{CallLogUseCases, CallUseCases};

use crate::call::build_call_router;
use crate::error::ApiError;
use crate::health::healthz;
use crate::session::build_session_router;

pub struct ModuleServices {
    pub token_service: Arc<dyn TokenService>,
    pub call_service: Arc<dyn CallUseCases>,
    pub call_log_service: Arc<dyn CallLogUseCases>,
}

pub fn build_router() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics))
}

async fn metrics() -> Result<impl axum::response::IntoResponse, ApiError> {
    observability::render_prometheus_metrics()
        .map_err(|message| ApiError(shared_kernel::AppError::internal(message)))
}

pub fn build_router_with_modules(services: ModuleServices) -> Router {
    build_router()
        .merge(build_session_router(services.token_service.clone()))
        .merge(build_call_router(
            services.call_service,
            services.call_log_service,
            services.token_service,
        ))
        .layer(middleware::from_fn(
            observability::attach_http_request_context,
        ))
}
