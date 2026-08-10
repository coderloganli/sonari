//! Starting a session.
//!
//! There is no login. A caller presents a `uid` it chose or was assigned, and
//! receives a token to carry. This identifies, it does not authenticate: anyone
//! who presents a `uid` reaches that history. Adding real authentication later
//! is a new layer in front of this, not a change to it.

use std::sync::Arc;

use auth::ports::TokenService;
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use shared_kernel::AppError;

use crate::{error::ApiError, response::ok};

/// A `uid` names a person's history, so it must be typeable and stable.
const MAX_UID_CHARS: usize = 64;
const MIN_UID_CHARS: usize = 3;

pub fn build_session_router(token_service: Arc<dyn TokenService>) -> Router {
    Router::new()
        .route("/api/session", post(create_session))
        .with_state(token_service)
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    uid: String,
}

#[derive(Debug, Serialize)]
struct CreateSessionData {
    uid: String,
    access_token: String,
    refresh_token: String,
    expires_in_seconds: i64,
}

async fn create_session(
    State(token_service): State<Arc<dyn TokenService>>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let uid = normalise(&request.uid)?;
    // The subject is derived from the uid so the same person resolves to the
    // same identity on any device, without a table to look it up in.
    let subject_id = crate::session::subject_id_for(&uid);
    let tokens = token_service
        .issue_token_pair(subject_id, "user", &[])
        .await?;
    Ok(ok(CreateSessionData {
        uid,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_in_seconds: tokens.expires_in_seconds,
    }))
}

/// Trims and checks a `uid`, rejecting shapes that would be confusing to type
/// back in on another device.
fn normalise(raw: &str) -> Result<String, ApiError> {
    let uid = raw.trim();
    let length = uid.chars().count();
    if length < MIN_UID_CHARS {
        return Err(ApiError(AppError::invalid_input(format!(
            "uid must be at least {MIN_UID_CHARS} characters"
        ))));
    }
    if length > MAX_UID_CHARS {
        return Err(ApiError(AppError::invalid_input(format!(
            "uid must be at most {MAX_UID_CHARS} characters"
        ))));
    }
    if uid.chars().any(char::is_whitespace) {
        return Err(ApiError(AppError::invalid_input(
            "uid must not contain whitespace",
        )));
    }
    Ok(uid.to_owned())
}

/// A stable positive identity for a `uid`.
///
/// Derived rather than allocated: there is no user table, and the same `uid`
/// entered on a new device must reach the same history.
pub fn subject_id_for(uid: &str) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(uid.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    (i64::from_be_bytes(bytes) & i64::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_uid_always_resolves_to_the_same_identity() {
        assert_eq!(
            subject_id_for("brave-otter-4417"),
            subject_id_for("brave-otter-4417")
        );
        assert!(subject_id_for("brave-otter-4417") > 0);
    }

    #[test]
    fn different_uids_are_different_identities() {
        assert_ne!(subject_id_for("one"), subject_id_for("two"));
    }

    #[test]
    fn surrounding_space_does_not_create_a_second_identity() {
        // Typing a uid on a phone tends to add a space.
        assert_eq!(normalise("  brave-otter  ").unwrap(), "brave-otter");
    }

    #[test]
    fn a_uid_with_a_space_inside_is_rejected() {
        assert!(normalise("brave otter").is_err());
    }

    #[test]
    fn a_uid_that_is_too_short_is_rejected() {
        assert!(normalise("ab").is_err());
    }
}
