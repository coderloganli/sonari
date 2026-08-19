use std::sync::Arc;

use agent::MemoryUseCases;
use auth::ports::TokenService;
use axum::{Router, middleware, routing::get};
use call::{CallLogUseCases, CallUseCases};
use character_context::CharacterCatalogReadPort;

use crate::call::build_call_router;
use crate::dev_client::build_dev_client_router;
use crate::error::ApiError;
use crate::health::healthz;
use crate::personas::build_personas_router;
use crate::session::build_session_router;

pub struct ModuleServices {
    pub token_service: Arc<dyn TokenService>,
    pub call_service: Arc<dyn CallUseCases>,
    pub call_log_service: Arc<dyn CallLogUseCases>,
    pub persona_catalog: Arc<dyn CharacterCatalogReadPort>,
    pub memory_service: Arc<dyn MemoryUseCases>,
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
        .merge(build_personas_router(services.persona_catalog))
        .merge(build_dev_client_router())
        .merge(crate::memory::build_memory_router(
            services.memory_service,
            services.token_service.clone(),
        ))
        .merge(build_call_router(
            services.call_service,
            services.call_log_service,
            services.token_service,
        ))
        .layer(middleware::from_fn(
            observability::attach_http_request_context,
        ))
}

#[cfg(test)]
mod tests {
    //! Every other test in this crate builds one router directly, and would
    //! still pass if the application never merged it. These two ask the router
    //! the binary actually serves.

    use std::sync::Arc;

    use agent::{MemoryFact, MemoryUseCases};
    use async_trait::async_trait;
    use auth::ports::{TokenPairView, TokenService};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use call::{
        CallActivityLog, CallDetailView, CallHistoryItemView, CallLogListQuery, CallLogUseCases,
        CallTimelineEvent, CallUseCases, EndCallCommand, PagedCallLogsView, StartCallCommand,
        StartCallResponse,
    };
    use character_context::{CharacterCatalogReadPort, CharacterSummary};
    use shared_kernel::{AppResult, Claims};
    use tower::ServiceExt;

    use super::{ModuleServices, build_router_with_modules};

    struct Unused;

    #[async_trait]
    impl CallUseCases for Unused {
        async fn start_call(&self, _command: StartCallCommand) -> AppResult<StartCallResponse> {
            unreachable!()
        }

        async fn end_call(&self, _command: EndCallCommand) -> AppResult<()> {
            unreachable!()
        }

        async fn list_call_history(&self, _user_id: i64) -> AppResult<Vec<CallHistoryItemView>> {
            unreachable!()
        }
    }

    #[async_trait]
    impl CallLogUseCases for Unused {
        async fn get_timeline(&self, _session_id: i64) -> AppResult<Vec<CallTimelineEvent>> {
            unreachable!()
        }

        async fn get_activity_log(&self, _session_id: i64) -> AppResult<CallActivityLog> {
            unreachable!()
        }

        async fn list_call_logs(&self, _query: CallLogListQuery) -> AppResult<PagedCallLogsView> {
            unreachable!()
        }

        async fn get_call_detail(&self, _session_id: i64) -> AppResult<CallDetailView> {
            unreachable!()
        }
    }

    #[async_trait]
    impl TokenService for Unused {
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

        async fn validate_access_token(&self, _access_token: &str) -> AppResult<Claims> {
            unreachable!()
        }
    }

    #[async_trait]
    impl MemoryUseCases for Unused {
        async fn list(&self, _user_id: i64) -> AppResult<Vec<MemoryFact>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _user_id: i64, _character_id: Option<i64>) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[async_trait]
    impl CharacterCatalogReadPort for Unused {
        async fn list_characters(&self) -> AppResult<Vec<CharacterSummary>> {
            Ok(vec![CharacterSummary {
                character_id: 11,
                name: "companion".to_owned(),
                scene_name: None,
            }])
        }
    }

    async fn status_of(path: &str) -> StatusCode {
        status_of_method("GET", path).await
    }

    async fn status_of_method(method: &str, path: &str) -> StatusCode {
        let router = build_router_with_modules(ModuleServices {
            token_service: Arc::new(Unused),
            call_service: Arc::new(Unused),
            call_log_service: Arc::new(Unused),
            persona_catalog: Arc::new(Unused),
            memory_service: Arc::new(Unused),
        });
        let request = Request::builder()
            .uri(path)
            .method(method)
            .body(Body::empty())
            .expect("build request");
        router
            .oneshot(request)
            .await
            .expect("route the request")
            .status()
    }

    /// Test case 13 — the application serves the page.
    #[tokio::test]
    async fn the_application_serves_the_test_client() {
        assert_eq!(status_of("/dev").await, StatusCode::OK);
    }

    /// Test case 14 — the application serves the persona list, and does not
    /// hide it behind the token the page cannot have yet.
    #[tokio::test]
    async fn the_application_lists_personas_without_a_token() {
        assert_eq!(status_of("/api/personas").await, StatusCode::OK);
    }

    /// Test case 21 — the application serves the memory routes. A route tested
    /// only against its own router can be absent from the one the binary serves.
    #[tokio::test]
    async fn the_application_serves_the_memory_routes() {
        assert_ne!(status_of("/api/memory").await, StatusCode::NOT_FOUND);
        assert_ne!(
            status_of_method("DELETE", "/api/memory").await,
            StatusCode::NOT_FOUND
        );
    }
}
