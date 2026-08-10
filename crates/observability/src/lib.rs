mod error_group;
mod http_middleware;
mod metrics;
mod postgres;
mod request_context;
mod tracing_init;

pub mod app_error;
pub mod client_event;
pub mod error;
pub mod service;

pub use app_error::{
    AppErrorGroupListItem, AppErrorListQuery, AppErrorListView, AppErrorOccurrence,
    AppErrorOccurrenceListItem, AppErrorOccurrenceListQuery, AppErrorOccurrenceListView,
    AppErrorQueryPort, AppErrorReport, AppErrorSinkPort, AppErrorStatBucket, AppErrorStatsQuery,
    AppErrorStatsView,
};
pub use client_event::{ClientDebugEvent, ClientDebugEventSinkPort};
pub use error::{ObservabilityError, ObservabilityErrorKind, ObservabilityResult};
pub use http_middleware::attach_http_request_context;
pub use metrics::{
    observe_http_request, record_app_error_ingest, record_bot_speech_interruptions,
    record_client_debug_ingest, record_runtime_start_failure, render_prometheus_metrics,
    set_call_event_consumer_lag,
};
pub use postgres::PostgresAppErrorRepository;
pub use request_context::{HttpRequestContext, RequestId, TraceId};
pub use service::{ObservabilityService, ObservabilityUseCases, parse_query_date};
pub use tracing_init::{init_tracing, service_span};
