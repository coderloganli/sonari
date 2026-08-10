use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestContext {
    pub request_id: RequestId,
    pub trace_id: Option<TraceId>,
    pub flow_id: Option<String>,
    pub method: String,
    pub path: String,
    pub user_agent: Option<String>,
    pub remote_addr: Option<String>,
    pub received_at: DateTime<Utc>,
}

impl RequestId {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TraceId {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
