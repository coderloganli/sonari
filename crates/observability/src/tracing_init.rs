use anyhow::Result;
use opentelemetry::{KeyValue, global, trace::TracerProvider as _};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    trace::{SdkTracer, SdkTracerProvider},
};
use tracing::{Span, info_span};
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing(service_name: &str) -> Result<()> {
    crate::metrics::init_metrics().map_err(anyhow::Error::msg)?;
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if let Some(tracer) = build_otlp_tracer(service_name)? {
        Registry::default()
            .with(env_filter)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_thread_names(true)
                    .json()
                    .with_current_span(true)
                    .with_span_list(true),
            )
            .try_init()?;
    } else {
        Registry::default()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(true)
                    .with_thread_names(true)
                    .json()
                    .with_current_span(true)
                    .with_span_list(true),
            )
            .try_init()?;
    }

    tracing::info!(service = service_name, "tracing initialized");
    Ok(())
}

pub fn service_span(service_name: &'static str) -> Span {
    info_span!(
        "service",
        service = service_name,
        service_instance_id = %service_instance_id()
    )
}

fn build_otlp_tracer(service_name: &str) -> Result<Option<SdkTracer>> {
    let Some(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;
    let resource = Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", service_name.to_owned()),
            KeyValue::new("service.instance.id", service_instance_id()),
            KeyValue::new(
                "deployment.environment",
                std::env::var("COMBRABO_STACK").unwrap_or_else(|_| "unknown".to_owned()),
            ),
        ])
        .build();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();
    let tracer = provider.tracer(service_name.to_owned());
    global::set_tracer_provider(provider);
    Ok(Some(tracer))
}

fn service_instance_id() -> String {
    std::env::var("COMBRABO_INSTANCE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{}-{}", std::process::id(), service_suffix()))
}

fn service_suffix() -> String {
    std::env::var("COMBRABO_SERVICE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "service".to_owned())
}
