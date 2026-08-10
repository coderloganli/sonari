use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;

use crate::{
    app_error::{
        AppErrorListQuery, AppErrorListView, AppErrorOccurrence, AppErrorOccurrenceListQuery,
        AppErrorOccurrenceListView, AppErrorQueryPort, AppErrorReport, AppErrorSinkPort,
        AppErrorStatsQuery, AppErrorStatsView,
    },
    client_event::{ClientDebugEvent, ClientDebugEventSinkPort},
    error::{ObservabilityError, ObservabilityResult},
    error_group::compute_app_error_fingerprint,
    metrics::{record_app_error_ingest, record_client_debug_ingest},
    request_context::HttpRequestContext,
};

const MAX_CLIENT_EVENT_BATCH: usize = 200;
const MAX_APP_ERROR_BATCH: usize = 100;
const MAX_TEXT_LEN: usize = 4096;

#[async_trait]
pub trait ObservabilityUseCases: Send + Sync {
    async fn ingest_client_events(
        &self,
        request_context: HttpRequestContext,
        events: Vec<ClientDebugEvent>,
    ) -> ObservabilityResult<usize>;
    async fn ingest_app_errors(
        &self,
        request_context: HttpRequestContext,
        reports: Vec<AppErrorReport>,
    ) -> ObservabilityResult<usize>;
    async fn list_app_errors(
        &self,
        query: AppErrorListQuery,
    ) -> ObservabilityResult<AppErrorListView>;
    async fn list_app_error_occurrences(
        &self,
        query: AppErrorOccurrenceListQuery,
    ) -> ObservabilityResult<AppErrorOccurrenceListView>;
    async fn get_app_error_stats(
        &self,
        query: AppErrorStatsQuery,
    ) -> ObservabilityResult<AppErrorStatsView>;
}

pub struct ObservabilityDependencies {
    pub client_event_sink: Box<dyn ClientDebugEventSinkPort>,
    pub app_errors: Box<dyn AppErrorSinkPort>,
    pub app_error_queries: Box<dyn AppErrorQueryPort>,
}

pub struct ObservabilityService {
    client_event_sink: Box<dyn ClientDebugEventSinkPort>,
    app_errors: Box<dyn AppErrorSinkPort>,
    app_error_queries: Box<dyn AppErrorQueryPort>,
}

impl ObservabilityService {
    pub fn new(deps: ObservabilityDependencies) -> Self {
        Self {
            client_event_sink: deps.client_event_sink,
            app_errors: deps.app_errors,
            app_error_queries: deps.app_error_queries,
        }
    }
}

#[async_trait]
impl ObservabilityUseCases for ObservabilityService {
    async fn ingest_client_events(
        &self,
        request_context: HttpRequestContext,
        events: Vec<ClientDebugEvent>,
    ) -> ObservabilityResult<usize> {
        if events.len() > MAX_CLIENT_EVENT_BATCH {
            return Err(ObservabilityError::invalid_input("too many client events"));
        }
        let normalized = events
            .into_iter()
            .map(normalize_client_event)
            .collect::<ObservabilityResult<Vec<_>>>()?;
        if normalized.is_empty() {
            return Ok(0);
        }
        for event in &normalized {
            emit_client_event_log(&request_context, event);
            if event.session_id.is_some() {
                self.client_event_sink
                    .publish_client_event(event.clone())
                    .await?;
            }
        }
        tracing::info!(
            request_id = %request_context.request_id.as_str(),
            trace_id = request_context.trace_id.as_ref().map(|value| value.as_str()),
            path = %request_context.path,
            accepted_count = normalized.len(),
            "client debug events accepted",
        );
        record_client_debug_ingest(normalized.len());
        Ok(normalized.len())
    }

    async fn ingest_app_errors(
        &self,
        request_context: HttpRequestContext,
        reports: Vec<AppErrorReport>,
    ) -> ObservabilityResult<usize> {
        if reports.len() > MAX_APP_ERROR_BATCH {
            return Err(ObservabilityError::invalid_input("too many app errors"));
        }
        let normalized = reports
            .into_iter()
            .filter(|report| !report.message.trim().is_empty())
            .map(normalize_app_error)
            .collect::<ObservabilityResult<Vec<_>>>()?;
        if normalized.is_empty() {
            return Ok(0);
        }
        let occurrences = normalized
            .into_iter()
            .map(|report| AppErrorOccurrence {
                request_id: request_context.request_id.as_str().to_owned(),
                trace_id: request_context
                    .trace_id
                    .as_ref()
                    .map(|trace_id| trace_id.as_str().to_owned()),
                fingerprint: compute_app_error_fingerprint(&report),
                report,
            })
            .collect::<Vec<_>>();
        tracing::info!(
            request_id = %request_context.request_id.as_str(),
            trace_id = request_context.trace_id.as_ref().map(|value| value.as_str()),
            path = %request_context.path,
            accepted_count = occurrences.len(),
            "app errors accepted",
        );
        self.app_errors
            .append_app_errors(&request_context, &occurrences)
            .await?;
        record_app_error_ingest(occurrences.len());
        Ok(occurrences.len())
    }

    async fn list_app_errors(
        &self,
        mut query: AppErrorListQuery,
    ) -> ObservabilityResult<AppErrorListView> {
        if query.page < 1 {
            query.page = 1;
        }
        if query.page_size < 1 {
            query.page_size = 20;
        } else if query.page_size > 100 {
            query.page_size = 100;
        }
        let (items, total) = self.app_error_queries.list_app_errors(&query).await?;
        Ok(AppErrorListView {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn list_app_error_occurrences(
        &self,
        mut query: AppErrorOccurrenceListQuery,
    ) -> ObservabilityResult<AppErrorOccurrenceListView> {
        if query.group_id < 1 {
            return Err(ObservabilityError::invalid_input(
                "group_id must be positive",
            ));
        }
        if query.page < 1 {
            query.page = 1;
        }
        if query.page_size < 1 {
            query.page_size = 20;
        } else if query.page_size > 100 {
            query.page_size = 100;
        }
        let (items, total) = self
            .app_error_queries
            .list_app_error_occurrences(&query)
            .await?;
        Ok(AppErrorOccurrenceListView {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn get_app_error_stats(
        &self,
        query: AppErrorStatsQuery,
    ) -> ObservabilityResult<AppErrorStatsView> {
        self.app_error_queries.get_app_error_stats(&query).await
    }
}

fn normalize_client_event(mut event: ClientDebugEvent) -> ObservabilityResult<ClientDebugEvent> {
    if event.name.trim().is_empty() {
        return Err(ObservabilityError::invalid_input(
            "client event name is required",
        ));
    }
    if event.name.len() > 255 {
        return Err(ObservabilityError::invalid_input(
            "client event name is too long",
        ));
    }
    if event.level.trim().is_empty() {
        event.level = "info".to_owned();
    }
    event.name = truncate_text(event.name.trim());
    event.level = truncate_text(event.level.trim());
    event.flow_id = event.flow_id.map(|value| truncate_text(value.trim()));
    event.event_class = event.event_class.map(|value| truncate_text(value.trim()));
    event.session_id = event.session_id.map(|value| truncate_text(value.trim()));
    event.character_id = event.character_id.map(|value| truncate_text(value.trim()));
    event.device_name = event.device_name.map(|value| truncate_text(value.trim()));
    if event.merged_count < 1 {
        event.merged_count = 1;
    }
    event.fields = normalize_json_object(event.fields)?;
    Ok(event)
}

fn normalize_app_error(mut report: AppErrorReport) -> ObservabilityResult<AppErrorReport> {
    report.level = if report.level.trim().is_empty() {
        "error".to_owned()
    } else {
        truncate_text(report.level.trim())
    };
    report.category = truncate_text(report.category.trim());
    report.message = truncate_text(report.message.trim());
    report.stack_trace = truncate_text(&report.stack_trace);
    report.session_id = report.session_id.map(|value| truncate_text(value.trim()));
    report.device_name = report.device_name.map(|value| truncate_text(value.trim()));
    report.app_version = truncate_text(report.app_version.trim());
    report.os_version = truncate_text(report.os_version.trim());
    report.fields = normalize_json_object(report.fields)?;
    Ok(report)
}

fn emit_client_event_log(request_context: &HttpRequestContext, event: &ClientDebugEvent) {
    tracing::info!(
        target: "client_debug_event",
        log_type = "client_debug_event",
        request_id = %request_context.request_id.as_str(),
        trace_id = request_context.trace_id.as_ref().map(|value| value.as_str()),
        flow_id = request_context.flow_id.as_deref().or(event.flow_id.as_deref()),
        path = %request_context.path,
        client_event_name = %event.name,
        client_event_level = %event.level,
        event_class = event.event_class.as_deref(),
        session_id = event.session_id.as_deref(),
        character_id = event.character_id.as_deref(),
        device_name = event.device_name.as_deref(),
        merged_count = event.merged_count,
        client_ts_ms = event.ts_ms,
        fields = %event.fields,
        "client debug event accepted",
    );
}

fn normalize_json_object(value: Value) -> ObservabilityResult<Value> {
    match value {
        Value::Object(map) => Ok(Value::Object(map)),
        Value::Null => Ok(Value::Object(Default::default())),
        _ => Err(ObservabilityError::invalid_input(
            "fields must be a JSON object",
        )),
    }
}

fn truncate_text(value: &str) -> String {
    value.chars().take(MAX_TEXT_LEN).collect()
}

pub fn parse_query_date(value: Option<&str>) -> ObservabilityResult<Option<DateTime<Utc>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ObservabilityError::invalid_input("invalid date, expected YYYY-MM-DD"))?;
    Ok(date
        .and_hms_opt(0, 0, 0)
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)))
}
