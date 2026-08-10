use axum::{
    extract::MatchedPath,
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue, header::USER_AGENT},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use tracing::{Instrument, info_span};
use uuid::Uuid;

use crate::{
    metrics::observe_http_request,
    request_context::{HttpRequestContext, RequestId, TraceId},
};

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
const FLOW_ID_HEADER: HeaderName = HeaderName::from_static("x-sonari-flow-id");

pub async fn attach_http_request_context(mut request: Request, next: Next) -> Response {
    let started_at = std::time::Instant::now();
    let matched_path = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| request.uri().path())
        .to_owned();
    let context = HttpRequestContext {
        request_id: request_id_from_headers(request.headers()),
        trace_id: trace_id_from_headers(request.headers()),
        flow_id: flow_id_from_headers(request.headers()),
        method: request.method().to_string(),
        path: matched_path,
        user_agent: request
            .headers()
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        remote_addr: None,
        received_at: Utc::now(),
    };
    let span = info_span!(
        "http_request",
        request_id = %context.request_id.as_str(),
        trace_id = context.trace_id.as_ref().map(|value| value.as_str()),
        flow_id = context.flow_id.as_deref(),
        method = %context.method,
        path = %context.path,
    );
    request.extensions_mut().insert(context.clone());
    let mut response = next.run(request).instrument(span).await;
    if let Ok(header_value) = HeaderValue::from_str(context.request_id.as_str()) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, header_value);
    }
    observe_http_request(
        &context.method,
        &context.path,
        response.status().as_u16(),
        started_at.elapsed().as_secs_f64(),
    );
    response
}

fn request_id_from_headers(headers: &HeaderMap) -> RequestId {
    let request_id = headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(generate_request_id);
    RequestId::new(request_id)
}

fn trace_id_from_headers(headers: &HeaderMap) -> Option<TraceId> {
    headers
        .get(HeaderName::from_static("traceparent"))
        .and_then(|value| value.to_str().ok())
        .and_then(parse_traceparent_trace_id)
        .map(TraceId::new)
}

fn flow_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(FLOW_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_traceparent_trace_id(traceparent: &str) -> Option<String> {
    let mut parts = traceparent.split('-');
    let _version = parts.next()?;
    let trace_id = parts.next()?;
    let _parent_id = parts.next()?;
    let _flags = parts.next()?;
    if trace_id.len() != 32 || !trace_id.chars().all(|value| value.is_ascii_hexdigit()) {
        return None;
    }
    if trace_id.chars().all(|value| value == '0') {
        return None;
    }
    Some(trace_id.to_ascii_lowercase())
}

fn generate_request_id() -> String {
    Uuid::now_v7().to_string()
}
