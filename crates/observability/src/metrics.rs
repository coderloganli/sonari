use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};
use std::sync::OnceLock;

struct ObservabilityMetrics {
    registry: Registry,
    http_requests_total: IntCounterVec,
    http_request_duration_seconds: HistogramVec,
    debug_ingest_events_total: IntCounter,
    app_errors_total: IntCounter,
    call_event_consumer_lag: IntGauge,
    runtime_start_failures_total: IntCounter,
    bot_speech_interruptions_total: IntCounter,
}

static METRICS: OnceLock<ObservabilityMetrics> = OnceLock::new();

pub fn init_metrics() -> Result<(), String> {
    if METRICS.get().is_some() {
        return Ok(());
    }
    let metrics = build_metrics()?;
    let _ = METRICS.set(metrics);
    Ok(())
}

pub fn observe_http_request(method: &str, path: &str, status: u16, duration_seconds: f64) {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics
        .http_requests_total
        .with_label_values(&[method, path, &status.to_string()])
        .inc();
    metrics
        .http_request_duration_seconds
        .with_label_values(&[method, path])
        .observe(duration_seconds);
}

pub fn record_client_debug_ingest(count: usize) {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.debug_ingest_events_total.inc_by(count as u64);
}

pub fn record_app_error_ingest(count: usize) {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.app_errors_total.inc_by(count as u64);
}

pub fn set_call_event_consumer_lag(count: i64) {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.call_event_consumer_lag.set(count);
}

pub fn record_runtime_start_failure() {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.runtime_start_failures_total.inc();
}

pub fn record_bot_speech_interruptions(count: u64) {
    let Some(metrics) = METRICS.get() else {
        return;
    };
    metrics.bot_speech_interruptions_total.inc_by(count);
}

pub fn render_prometheus_metrics() -> Result<String, String> {
    let Some(metrics) = METRICS.get() else {
        return Err("observability metrics are not initialized".to_owned());
    };
    let encoder = TextEncoder::new();
    let families = metrics.registry.gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&families, &mut buffer)
        .map_err(|error| format!("failed to encode prometheus metrics: {error}"))?;
    String::from_utf8(buffer)
        .map_err(|error| format!("failed to build prometheus text response: {error}"))
}

fn build_metrics() -> Result<ObservabilityMetrics, String> {
    let registry = Registry::new_custom(Some("sonari".to_owned()), None)
        .map_err(|error| format!("failed to create metrics registry: {error}"))?;
    let http_requests_total = IntCounterVec::new(
        Opts::new(
            "http_requests_total",
            "Total HTTP requests by method, path, and status",
        ),
        &["method", "path", "status"],
    )
    .map_err(|error| format!("failed to create http_requests_total: {error}"))?;
    let http_request_duration_seconds = HistogramVec::new(
        HistogramOpts::new(
            "http_request_duration_seconds",
            "HTTP request duration by method and path",
        ),
        &["method", "path"],
    )
    .map_err(|error| format!("failed to create http_request_duration_seconds: {error}"))?;
    let debug_ingest_events_total =
        IntCounter::new("debug_ingest_events_total", "Accepted client debug events")
            .map_err(|error| format!("failed to create debug_ingest_events_total: {error}"))?;
    let app_errors_total = IntCounter::new("app_errors_total", "Accepted app error occurrences")
        .map_err(|error| format!("failed to create app_errors_total: {error}"))?;
    let call_event_consumer_lag = IntGauge::new(
        "call_event_consumer_lag",
        "Pending call events in Redis stream",
    )
    .map_err(|error| format!("failed to create call_event_consumer_lag: {error}"))?;
    let runtime_start_failures_total = IntCounter::new(
        "runtime_start_failures_total",
        "Claimed runtime start failures that converged to failed state",
    )
    .map_err(|error| format!("failed to create runtime_start_failures_total: {error}"))?;
    let bot_speech_interruptions_total = IntCounter::new(
        "bot_speech_interruptions_total",
        "Observed bot speech interruptions in the call event pipeline",
    )
    .map_err(|error| format!("failed to create bot_speech_interruptions_total: {error}"))?;

    registry
        .register(Box::new(http_requests_total.clone()))
        .map_err(|error| format!("failed to register http_requests_total: {error}"))?;
    registry
        .register(Box::new(http_request_duration_seconds.clone()))
        .map_err(|error| format!("failed to register http_request_duration_seconds: {error}"))?;
    registry
        .register(Box::new(debug_ingest_events_total.clone()))
        .map_err(|error| format!("failed to register debug_ingest_events_total: {error}"))?;
    registry
        .register(Box::new(app_errors_total.clone()))
        .map_err(|error| format!("failed to register app_errors_total: {error}"))?;
    registry
        .register(Box::new(call_event_consumer_lag.clone()))
        .map_err(|error| format!("failed to register call_event_consumer_lag: {error}"))?;
    registry
        .register(Box::new(runtime_start_failures_total.clone()))
        .map_err(|error| format!("failed to register runtime_start_failures_total: {error}"))?;
    registry
        .register(Box::new(bot_speech_interruptions_total.clone()))
        .map_err(|error| format!("failed to register bot_speech_interruptions_total: {error}"))?;

    Ok(ObservabilityMetrics {
        registry,
        http_requests_total,
        http_request_duration_seconds,
        debug_ingest_events_total,
        app_errors_total,
        call_event_consumer_lag,
        runtime_start_failures_total,
        bot_speech_interruptions_total,
    })
}
