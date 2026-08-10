use std::sync::Arc;

use auth::ports::TokenService;
use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    middleware,
    routing::{get, post},
};
use call::{CallLogListQuery, CallLogUseCases, CallUseCases, EndCallCommand, StartCallCommand};
use chrono::{DateTime, NaiveDate, Utc};
use observability::HttpRequestContext;
use serde::Deserialize;
use shared_kernel::Claims;
use tracing::Instrument;

use crate::{
    admin_auth::{require_admin_auth, require_user_auth},
    error::ApiError,
    response::ok,
};

pub fn build_call_router(
    call_service: Arc<dyn CallUseCases>,
    call_log_service: Arc<dyn CallLogUseCases>,
    token_service: Arc<dyn TokenService>,
) -> Router {
    let user_router = Router::new()
        .route("/api/call/{character_id}/start", post(start_call))
        .route("/api/call/end", post(end_call))
        .route("/api/call/history", get(list_history))
        .route_layer(middleware::from_fn_with_state(
            token_service.clone(),
            require_user_auth,
        ))
        .with_state(call_service);
    let admin_router = Router::new()
        .route("/api/admin/call-logs", get(list_call_logs))
        .route("/api/admin/call-logs/{session_id}", get(get_call_detail))
        .route(
            "/api/admin/call-logs/{session_id}/timeline",
            get(get_call_timeline),
        )
        .route(
            "/api/admin/call-logs/{session_id}/activity-log",
            get(get_activity_log),
        )
        .route_layer(middleware::from_fn_with_state(
            token_service,
            require_admin_auth,
        ))
        .with_state(call_log_service);
    user_router.merge(admin_router)
}

#[derive(Debug, Deserialize)]
struct EndCallRequest {
    session_id: i64,
}

#[derive(Debug, Deserialize, Default)]
struct StartCallRequest {
    scene_id: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct CallLogsQuery {
    character_id: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
    date_from: Option<String>,
    date_to: Option<String>,
}

async fn start_call(
    State(call_service): State<Arc<dyn CallUseCases>>,
    Extension(claims): Extension<Claims>,
    Extension(request_context): Extension<HttpRequestContext>,
    Path(character_id): Path<i64>,
    request: Option<Json<StartCallRequest>>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let scene_id = request.and_then(|body| body.scene_id);
    let span = tracing::info_span!(
        "start_call",
        request_id = request_context.request_id.as_str(),
        trace_id = request_context
            .trace_id
            .as_ref()
            .map(|value| value.as_str()),
        flow_id = request_context.flow_id.as_deref(),
        user_id = claims.subject_id,
        character_id,
        scene_id,
    );
    Ok(ok(call_service
        .start_call(StartCallCommand {
            user_id: claims.subject_id,
            character_id,
            scene_id,
        })
        .instrument(span)
        .await?))
}

async fn end_call(
    State(call_service): State<Arc<dyn CallUseCases>>,
    Extension(claims): Extension<Claims>,
    Json(request): Json<EndCallRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    call_service
        .end_call(EndCallCommand {
            user_id: claims.subject_id,
            session_id: request.session_id,
        })
        .await?;
    Ok(ok(serde_json::Value::Null))
}

async fn list_history(
    State(call_service): State<Arc<dyn CallUseCases>>,
    Extension(claims): Extension<Claims>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    Ok(ok(call_service
        .list_call_history(claims.subject_id)
        .await?))
}

async fn get_call_timeline(
    State(log_service): State<Arc<dyn CallLogUseCases>>,
    Path(session_id): Path<i64>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    Ok(ok(log_service.get_timeline(session_id).await?))
}

async fn list_call_logs(
    State(log_service): State<Arc<dyn CallLogUseCases>>,
    Query(query): Query<CallLogsQuery>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    Ok(ok(log_service
        .list_call_logs(CallLogListQuery {
            character_id: query.character_id,
            page: query.page.unwrap_or(1),
            page_size: query.page_size.unwrap_or(20),
            date_from: parse_query_date(query.date_from.as_deref())?,
            date_to: parse_query_date(query.date_to.as_deref())?.map(end_of_day),
        })
        .await?))
}

async fn get_call_detail(
    State(log_service): State<Arc<dyn CallLogUseCases>>,
    Path(session_id): Path<i64>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    Ok(ok(log_service.get_call_detail(session_id).await?))
}

async fn get_activity_log(
    State(log_service): State<Arc<dyn CallLogUseCases>>,
    Path(session_id): Path<i64>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    Ok(ok(log_service.get_activity_log(session_id).await?))
}

fn parse_query_date(value: Option<&str>) -> Result<Option<DateTime<Utc>>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        ApiError(shared_kernel::AppError::invalid_input(
            "invalid date, expected YYYY-MM-DD",
        ))
    })?;
    Ok(date
        .and_hms_opt(0, 0, 0)
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)))
}

fn end_of_day(date: DateTime<Utc>) -> DateTime<Utc> {
    date + chrono::TimeDelta::days(1) - chrono::TimeDelta::nanoseconds(1)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use auth::ports::{TokenPairView, TokenService};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use call::{
        CallActivityLog, CallDetailView, CallHistoryItemView, CallLogCallerView,
        CallLogListItemView, CallLogListQuery, CallLogUseCases, CallSessionSummaryView,
        CallTimelineEvent, CallUseCases, EndCallCommand, PagedCallLogsView, StartCallCommand,
        StartCallResponse,
    };
    use chrono::Utc;
    use serde_json::Value;
    use shared_kernel::{AppResult, Claims, Role};
    use tower::ServiceExt;

    use super::build_call_router;

    struct StubCallService;
    struct StubCallLogService;

    #[async_trait]
    impl CallUseCases for StubCallService {
        async fn start_call(&self, _command: StartCallCommand) -> AppResult<StartCallResponse> {
            Ok(StartCallResponse {
                session_id: 11,
                owner_instance: "instance-a".to_owned(),
                realtime: rtc_control_contract::RealtimeJoinArtifacts {
                    provider: "livekit".to_owned(),
                    endpoint: "ws://127.0.0.1:7880".to_owned(),
                    room_name: "call-11".to_owned(),
                    access_token: "token".to_owned(),
                    participant_identity: "platform_user:1".to_owned(),
                    bot_participant_identity: "bot-11".to_owned(),
                },
            })
        }

        async fn end_call(&self, _command: EndCallCommand) -> AppResult<()> {
            Ok(())
        }

        async fn list_call_history(&self, _user_id: i64) -> AppResult<Vec<CallHistoryItemView>> {
            Ok(vec![CallHistoryItemView {
                session_id: 11,
                character_id: 7,
                status: "active".to_owned(),
                owner_instance: "instance-a".to_owned(),
                started_at: Utc::now(),
                ended_at: None,
                duration_seconds: 0,
            }])
        }
    }

    #[async_trait]
    impl CallLogUseCases for StubCallLogService {
        async fn get_timeline(&self, session_id: i64) -> AppResult<Vec<CallTimelineEvent>> {
            Ok(vec![CallTimelineEvent {
                id: 1,
                session_id,
                round_id: Some("r1".to_owned()),
                source: "call-control".to_owned(),
                event: "call_started".to_owned(),
                ts_ms: 1,
                fields: serde_json::json!({}),
                created_at: Utc::now(),
            }])
        }

        async fn get_activity_log(&self, session_id: i64) -> AppResult<CallActivityLog> {
            Ok(CallActivityLog {
                session_id,
                started_at_ms: 1,
                rounds: vec![],
            })
        }

        async fn list_call_logs(&self, query: CallLogListQuery) -> AppResult<PagedCallLogsView> {
            Ok(PagedCallLogsView {
                items: vec![CallLogListItemView {
                    session_id: 11,
                    user_id: Some(1),
                    caller: CallLogCallerView {
                        caller_kind: "platform_user".to_owned(),
                        platform_user_id: Some(1),
                    },
                    character_id: query.character_id.unwrap_or(7),
                    character_name: "Rem".to_owned(),
                    scene_id: Some(3),
                    scene_name: Some("Cafe".to_owned()),
                    status: "active".to_owned(),
                    runtime_status: "ready".to_owned(),
                    runtime_failure_reason: None,
                    owner_instance: "instance-a".to_owned(),
                    started_at: Utc::now(),
                    ended_at: None,
                    duration_seconds: 0,
                    event_count: 3,
                    round_count: 1,
                }],
                total: 1,
                page: query.page,
                page_size: query.page_size,
            })
        }

        async fn get_call_detail(&self, session_id: i64) -> AppResult<CallDetailView> {
            Ok(CallDetailView {
                session: CallSessionSummaryView {
                    session_id,
                    user_id: Some(1),
                    caller: CallLogCallerView {
                        caller_kind: "platform_user".to_owned(),
                        platform_user_id: Some(1),
                    },
                    character_id: 7,
                    character_name: "Rem".to_owned(),
                    scene_id: Some(3),
                    scene_name: Some("Cafe".to_owned()),
                    voice: "voice-9".to_owned(),
                    agent_session_id: "agent-session-1".to_owned(),
                    asr_language: "zh".to_owned(),
                    status: "active".to_owned(),
                    runtime_status: "ready".to_owned(),
                    runtime_failure_reason: None,
                    owner_instance: "instance-a".to_owned(),
                    started_at: Utc::now(),
                    ended_at: None,
                    duration_seconds: 0,
                    event_count: 3,
                    round_count: 1,
                },
                conversation: vec![],
                timeline: vec![],
                activity_log: CallActivityLog {
                    session_id,
                    started_at_ms: 1,
                    rounds: vec![],
                },
                prompt_logs: vec![],
            })
        }
    }

    struct StubTokenService;

    #[async_trait]
    impl TokenService for StubTokenService {
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
                "good-token" => Ok(Claims {
                    subject_id: 1,
                    role: Role::User,
                }),
                "admin-token" => Ok(Claims {
                    subject_id: 2,
                    role: Role::Admin,
                }),
                _ => Err(shared_kernel::AppError::unauthorized(
                    "invalid or expired token",
                )),
            }
        }
    }

    #[tokio::test]
    async fn start_call_requires_authentication() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/call/7/start")
            .method("POST")
            .body(Body::from("{}"));
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn history_returns_payload_for_user() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/call/history")
            .header("authorization", "Bearer good-token")
            .method("GET")
            .body(Body::empty());
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::OK);

        let body_result = axum::body::to_bytes(response.into_body(), usize::MAX).await;
        assert!(body_result.is_ok());
        let body = match body_result {
            Ok(body) => body,
            Err(_) => return,
        };
        let payload_result: Result<Value, _> = serde_json::from_slice(&body);
        assert!(payload_result.is_ok());
        let payload = match payload_result {
            Ok(payload) => payload,
            Err(_) => return,
        };
        assert_eq!(payload["data"][0]["session_id"], 11);
    }

    #[tokio::test]
    async fn admin_call_logs_require_admin_auth() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/admin/call-logs/11/timeline")
            .header("authorization", "Bearer good-token")
            .method("GET")
            .body(Body::empty());
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_call_logs_allow_admin_role() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/admin/call-logs/11/timeline")
            .header("authorization", "Bearer admin-token")
            .method("GET")
            .body(Body::empty());
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_call_log_list_allows_admin_role() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/admin/call-logs?page=1&page_size=20&character_id=7")
            .header("authorization", "Bearer admin-token")
            .method("GET")
            .body(Body::empty());
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_call_detail_allows_admin_role() {
        let router = build_call_router(
            Arc::new(StubCallService),
            Arc::new(StubCallLogService),
            Arc::new(StubTokenService),
        );
        let request_result = Request::builder()
            .uri("/api/admin/call-logs/11")
            .header("authorization", "Bearer admin-token")
            .method("GET")
            .body(Body::empty());
        assert!(request_result.is_ok());
        let request = match request_result {
            Ok(request) => request,
            Err(_) => return,
        };

        let response_result = router.oneshot(request).await;
        assert!(response_result.is_ok());
        let response = match response_result {
            Ok(response) => response,
            Err(_) => return,
        };
        assert_eq!(response.status(), StatusCode::OK);
    }
}
