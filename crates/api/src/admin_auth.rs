use std::sync::Arc;

use auth::ports::TokenService;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use shared_kernel::{AppError, Role};

use crate::error::ApiError;

fn extract_bearer_token(request: &Request) -> Result<&str, ApiError> {
    let header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError(AppError::unauthorized(
                "missing or invalid authorization header",
            ))
        })?;
    header.strip_prefix("Bearer ").ok_or_else(|| {
        ApiError(AppError::unauthorized(
            "missing or invalid authorization header",
        ))
    })
}

async fn authorize(
    token_service: Arc<dyn TokenService>,
    mut request: Request,
    next: Next,
    predicate: impl FnOnce(Role) -> bool,
) -> Result<Response, ApiError> {
    let token = extract_bearer_token(&request)?;
    let claims = token_service.validate_access_token(token).await?;
    if !predicate(claims.role) {
        return Err(ApiError(AppError::forbidden("forbidden")));
    }
    request.extensions_mut().insert(claims);
    Ok(next.run(request).await)
}

pub async fn require_user_auth(
    State(token_service): State<Arc<dyn TokenService>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authorize(token_service, request, next, |role| {
        matches!(role, Role::User)
    })
    .await
}

pub async fn require_admin_auth(
    State(token_service): State<Arc<dyn TokenService>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authorize(token_service, request, next, |role| {
        matches!(role, Role::Admin)
    })
    .await
}
