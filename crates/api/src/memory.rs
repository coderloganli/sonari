//! What the agent remembers about the caller, to the caller.
//!
//! A `uid` identifies but does not authenticate, so what is held about a person
//! has to be visible to them and removable by them — product.md §4 says so
//! plainly rather than leaving it to be discovered. Read and delete only:
//! authoring memory is not a surface anybody asked for.

use std::sync::Arc;

use agent::{MemoryFact, MemoryUseCases};
use auth::ports::TokenService;
use axum::{
    Router,
    extract::{Extension, Query, State},
    middleware,
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use shared_kernel::Claims;

use crate::{admin_auth::require_user_auth, error::ApiError, response::ok};

pub fn build_memory_router(
    memory_service: Arc<dyn MemoryUseCases>,
    token_service: Arc<dyn TokenService>,
) -> Router {
    Router::new()
        .route("/api/memory", get(list_memory))
        .route("/api/memory", delete(forget_memory))
        .route_layer(middleware::from_fn_with_state(
            token_service,
            require_user_auth,
        ))
        .with_state(memory_service)
}

#[derive(Debug, Deserialize, Default)]
struct ForgetQuery {
    /// Narrows the deletion to one persona. Absent means forget everything.
    character_id: Option<i64>,
}

/// One fact, as the person it is about sees it.
#[derive(Debug, Serialize)]
struct MemoryFactView {
    character_id: i64,
    category: &'static str,
    content: String,
    first_seen_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<MemoryFact> for MemoryFactView {
    fn from(fact: MemoryFact) -> Self {
        Self {
            character_id: fact.character_id,
            category: fact.category.as_str(),
            content: fact.content,
            first_seen_at: fact.first_seen_at,
            updated_at: fact.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct MemoryListData {
    facts: Vec<MemoryFactView>,
}

#[derive(Debug, Serialize)]
struct ForgottenData {
    deleted: u64,
}

async fn list_memory(
    State(memory_service): State<Arc<dyn MemoryUseCases>>,
    Extension(claims): Extension<Claims>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    // The caller is the token's subject. Nothing in the request says whose
    // memory this is, so nothing in the request can ask for someone else's.
    let facts = memory_service.list(claims.subject_id).await?;
    Ok(ok(MemoryListData {
        facts: facts.into_iter().map(MemoryFactView::from).collect(),
    }))
}

async fn forget_memory(
    State(memory_service): State<Arc<dyn MemoryUseCases>>,
    Extension(claims): Extension<Claims>,
    Query(query): Query<ForgetQuery>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let deleted = memory_service
        .forget(claims.subject_id, query.character_id)
        .await?;
    Ok(ok(ForgottenData { deleted }))
}

#[cfg(test)]
mod tests {
    //! Test cases 19-25 of task.md.

    use std::sync::{Arc, Mutex};

    use agent::{MemoryCategory, MemoryFact, MemoryUseCases};
    use async_trait::async_trait;
    use auth::ports::{TokenPairView, TokenService};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use shared_kernel::{AppResult, Claims, Role};
    use tower::ServiceExt;

    use super::build_memory_router;

    /// Two callers, so a handler that ignores the token is visible.
    #[derive(Default)]
    struct FakeMemory {
        listed: Mutex<Vec<i64>>,
        forgotten: Mutex<Vec<(i64, Option<i64>)>>,
    }

    fn fact(user_id: i64, character_id: i64, content: &str) -> MemoryFact {
        MemoryFact {
            user_id,
            character_id,
            category: MemoryCategory::Relationship,
            content: content.to_owned(),
            first_seen_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_session_id: "session-a".into(),
        }
    }

    fn everything() -> Vec<MemoryFact> {
        vec![
            fact(7, 11, "The caller has a cat called Coal."),
            fact(7, 12, "The caller mentioned a brother."),
            fact(8, 11, "Someone else entirely."),
        ]
    }

    #[async_trait]
    impl MemoryUseCases for FakeMemory {
        async fn list(&self, user_id: i64) -> AppResult<Vec<MemoryFact>> {
            self.listed.lock().unwrap().push(user_id);
            Ok(everything()
                .into_iter()
                .filter(|fact| fact.user_id == user_id)
                .collect())
        }

        async fn forget(&self, user_id: i64, character_id: Option<i64>) -> AppResult<u64> {
            self.forgotten.lock().unwrap().push((user_id, character_id));
            Ok(everything()
                .into_iter()
                .filter(|fact| {
                    fact.user_id == user_id
                        && character_id
                            .map(|id| fact.character_id == id)
                            .unwrap_or(true)
                })
                .count() as u64)
        }
    }

    struct Tokens;

    #[async_trait]
    impl TokenService for Tokens {
        async fn issue_token_pair(
            &self,
            _subject_id: i64,
            _role: &str,
            _permissions: &[String],
        ) -> AppResult<TokenPairView> {
            unreachable!()
        }

        async fn refresh_token_pair(&self, _refresh_token: &str) -> AppResult<TokenPairView> {
            unreachable!()
        }

        async fn validate_access_token(&self, access_token: &str) -> AppResult<Claims> {
            match access_token {
                "caller-7" => Ok(Claims {
                    subject_id: 7,
                    role: Role::User,
                }),
                _ => Err(shared_kernel::AppError::unauthorized("bad token")),
            }
        }
    }

    fn request(method: &str, uri: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri).method(method);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        builder.body(Body::empty()).expect("build request")
    }

    async fn body_of(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read the body");
        serde_json::from_slice(&bytes).expect("the body is JSON")
    }

    /// Test case 19 — reading needs a token.
    #[tokio::test]
    async fn reading_memory_needs_a_token() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory.clone(), Arc::new(Tokens));

        let response = router
            .oneshot(request("GET", "/api/memory", None))
            .await
            .expect("route the request");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(memory.listed.lock().unwrap().is_empty());
    }

    /// Test case 20 — deleting needs a token, and an unauthenticated request
    /// does not reach the store.
    #[tokio::test]
    async fn deleting_memory_needs_a_token() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory.clone(), Arc::new(Tokens));

        let response = router
            .oneshot(request("DELETE", "/api/memory", None))
            .await
            .expect("route the request");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(memory.forgotten.lock().unwrap().is_empty());
    }

    /// Test case 22 — reading returns the caller's facts, across personas.
    #[tokio::test]
    async fn reading_memory_returns_the_callers_facts() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory, Arc::new(Tokens));

        let response = router
            .oneshot(request("GET", "/api/memory", Some("caller-7")))
            .await
            .expect("route the request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_of(response).await;
        let facts = body["data"]["facts"]
            .as_array()
            .expect("a facts array")
            .clone();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0]["character_id"], 11);
        assert_eq!(facts[0]["category"], "relationship");
        assert_eq!(facts[1]["character_id"], 12);
    }

    /// Test case 23 — reading returns only the token's caller, and asks the
    /// store for that subject.
    #[tokio::test]
    async fn reading_memory_returns_only_the_tokens_caller() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory.clone(), Arc::new(Tokens));

        let response = router
            .oneshot(request("GET", "/api/memory", Some("caller-7")))
            .await
            .expect("route the request");

        let body = body_of(response).await;
        let serialised = body.to_string();
        assert!(
            !serialised.contains("Someone else entirely"),
            "another caller's facts must not be returned"
        );
        assert_eq!(memory.listed.lock().unwrap().as_slice(), [7]);
    }

    /// Test case 24 — deleting forgets everything for the caller, for the
    /// token's caller rather than one named in the request.
    #[tokio::test]
    async fn deleting_memory_forgets_everything_for_the_caller() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory.clone(), Arc::new(Tokens));

        let response = router
            .oneshot(request("DELETE", "/api/memory?user_id=8", Some("caller-7")))
            .await
            .expect("route the request");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_of(response).await["data"]["deleted"], 2);
        assert_eq!(memory.forgotten.lock().unwrap().as_slice(), [(7, None)]);
    }

    /// Test case 25 — deleting one persona's facts leaves the others.
    #[tokio::test]
    async fn deleting_one_persona_leaves_the_others() {
        let memory = Arc::new(FakeMemory::default());
        let router = build_memory_router(memory.clone(), Arc::new(Tokens));

        let response = router
            .oneshot(request(
                "DELETE",
                "/api/memory?character_id=11",
                Some("caller-7"),
            ))
            .await
            .expect("route the request");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_of(response).await["data"]["deleted"], 1);
        assert_eq!(memory.forgotten.lock().unwrap().as_slice(), [(7, Some(11))]);
    }
}
