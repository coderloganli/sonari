use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HttpRequestContext, ObservabilityResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorReport {
    pub level: String,
    pub category: String,
    pub message: String,
    pub stack_trace: String,
    pub session_id: Option<String>,
    pub device_name: Option<String>,
    pub app_version: String,
    pub os_version: String,
    pub fields: Value,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppErrorOccurrence {
    pub request_id: String,
    pub trace_id: Option<String>,
    pub fingerprint: String,
    pub report: AppErrorReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppErrorListQuery {
    pub level: Option<String>,
    pub category: Option<String>,
    pub page: i64,
    pub page_size: i64,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppErrorOccurrenceListQuery {
    pub group_id: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppErrorStatsQuery {
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorGroupListItem {
    pub id: i64,
    pub fingerprint: String,
    pub latest_level: String,
    pub category: String,
    pub sample_message: String,
    pub sample_stack_trace: String,
    pub latest_request_id: String,
    pub latest_trace_id: Option<String>,
    pub latest_session_id: Option<String>,
    pub latest_device_name: Option<String>,
    pub latest_app_version: String,
    pub latest_os_version: String,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub occurrence_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorListView {
    pub items: Vec<AppErrorGroupListItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorOccurrenceListItem {
    pub id: i64,
    pub group_id: i64,
    pub request_id: String,
    pub trace_id: Option<String>,
    pub fingerprint: String,
    pub level: String,
    pub category: String,
    pub message: String,
    pub stack_trace: String,
    pub session_id: Option<String>,
    pub device_name: Option<String>,
    pub app_version: String,
    pub os_version: String,
    pub fields: Value,
    pub client_ts_ms: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorOccurrenceListView {
    pub items: Vec<AppErrorOccurrenceListItem>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorStatBucket {
    pub key: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppErrorStatsView {
    pub total_occurrences: i64,
    pub total_groups: i64,
    pub by_category: Vec<AppErrorStatBucket>,
    pub by_app_version: Vec<AppErrorStatBucket>,
    pub by_device_name: Vec<AppErrorStatBucket>,
}

#[async_trait]
pub trait AppErrorSinkPort: Send + Sync {
    async fn append_app_errors(
        &self,
        request_context: &HttpRequestContext,
        occurrences: &[AppErrorOccurrence],
    ) -> ObservabilityResult<()>;
}

#[async_trait]
pub trait AppErrorQueryPort: Send + Sync {
    async fn list_app_errors(
        &self,
        query: &AppErrorListQuery,
    ) -> ObservabilityResult<(Vec<AppErrorGroupListItem>, i64)>;
    async fn list_app_error_occurrences(
        &self,
        query: &AppErrorOccurrenceListQuery,
    ) -> ObservabilityResult<(Vec<AppErrorOccurrenceListItem>, i64)>;
    async fn get_app_error_stats(
        &self,
        query: &AppErrorStatsQuery,
    ) -> ObservabilityResult<AppErrorStatsView>;
}
