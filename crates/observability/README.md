# observability

`observability` owns the telemetry-facing plane for `backend`.

It owns:
- HTTP request correlation
- client debug ingest
- app-error ingest, grouping, and admin query
- metrics rendering
- trace export wiring
- Codex-first diagnostics workflow

It does not own:
- call business timelines
- Redis call-event streaming
- call session lifecycle

## Use This First

For local debugging, use this crate README as the primary entrypoint.

Safe local helper:

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 status
```

Other common commands:

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 urls
```

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 rules
```

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 targets
```

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 query -RequestId req_123
```

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 query -SessionId 42
```

The script is read-only. It does not mutate services, data, or alert state.

By default the script checks `127.0.0.1`. To print or query LAN-facing URLs, pass `-SurfaceHost` with the machine IP, for example:

```powershell
powershell -ExecutionPolicy Bypass -File backend/scripts/observability.ps1 urls -SurfaceHost 192.168.0.95
```

## Local Surfaces

- Grafana: `http://127.0.0.1:23000`
- Loki API: `http://127.0.0.1:23100`
- Tempo API: `http://127.0.0.1:23200`
- Prometheus: `http://127.0.0.1:29090`
- Alertmanager: `http://127.0.0.1:29093`
- backend health: `http://127.0.0.1:28080/healthz`
- backend metrics: `http://127.0.0.1:28080/metrics`
- local alert webhook receiver: `http://127.0.0.1:28080/internal/alerts/webhook`

LAN-facing surfaces use the same ports on the machine IP, for example `http://192.168.0.95:23000`.

External alert receiver contract:
- [docs/alert-webhook-contract.md](/C:/dev/combrabo-lite/docs/alert-webhook-contract.md:1)

Cloud deployment templates:
- [deploy/cloud/observability/README.md](/C:/dev/combrabo-lite/deploy/cloud/observability/README.md:1)

## What To Query Where

### Logs

Primary surface:
- Grafana Explore -> Loki

Use logs for:
- high-frequency client debug logs
- backend structured logs
- worker logs
- alert webhook intake logs

Recommended correlation keys:
1. `request_id`
2. `session_id`
3. `trace_id`
4. `service`
5. `runtime_owner_id`

Recommended Loki labels:
- `service`
- `env`
- `module`
- `level`
- `log_type`

Do not use high-cardinality values as labels:
- `request_id`
- `trace_id`
- raw user identifiers
- arbitrary message text

LogQL examples:

```logql
{service="backend"} |= "client debug events accepted"
```

```logql
{service="backend"} | json | request_id="req_123"
```

```logql
{service="worker"} | json | session_id="42"
```

### Traces

Primary surface:
- Grafana Explore -> Tempo

Useful span names:
- `http_request`
- `start_call`
- `poll_runtime_work`
- `prepare_runtime_launch`
- `ingest_client_events`
- `ingest_app_errors`
- `llm_complete`
- `voice_provider_request`

Useful filters:
- `request_id`
- `session_id`
- `character_id`

Provider trace checks:
- `llm_complete`
- `voice_provider_request`

### Metrics

Primary surface:
- Grafana dashboards
- Prometheus expressions

Core metrics:
- `combrabo_http_requests_total`
- `combrabo_http_request_duration_seconds`
- `combrabo_debug_ingest_events_total`
- `combrabo_app_errors_total`
- `combrabo_call_event_consumer_lag`
- `combrabo_runtime_start_failures_total`
- `combrabo_bot_speech_interruptions_total`

PromQL examples:

```promql
sum by (method, path, status) (rate(combrabo_http_requests_total[5m]))
```

```promql
sum(rate(combrabo_app_errors_total[15m]))
```

```promql
max(combrabo_call_event_consumer_lag)
```

### App Errors

Primary operator APIs:
- `GET /api/admin/app-errors`
- `GET /api/admin/app-errors/stats`
- `GET /api/admin/app-errors/{group_id}/occurrences`

Use these for:
- grouped issue view
- issue statistics
- occurrence drilldown

## Recommended Debug Flow

### Request-scoped backend issue

1. Find `request_id`
2. Search Loki by `request_id`
3. Open the matching Tempo trace by `request_id`
4. If a call is involved, pivot to `session_id`

### Call runtime issue

1. Find `session_id`
2. Check admin call timeline/detail
3. Search Loki by `session_id`
4. Check `runtime_owner_id`
5. Inspect Tempo spans for `poll_runtime_work` and `prepare_runtime_launch`

### Client debug ingest issue

1. Search Loki for `client debug events accepted`
2. Filter by `request_id` or `session_id`
3. Check `combrabo_debug_ingest_events_total`

### App error issue

1. Query `GET /api/admin/app-errors`
2. Query `GET /api/admin/app-errors/stats`
3. Search Loki by `request_id` or `session_id`
4. Open the related trace if present

### Alert delivery issue

1. Check Prometheus rule state
2. Check Alertmanager health and receiver config
3. Search Loki for `alertmanager webhook received`
4. Confirm the receiver returned `2xx`

### Provider call issue

1. Start from the request or session that should have triggered the provider call
2. Search Tempo for `llm_complete` or `voice_provider_request`
3. Check low-sensitivity span attributes:
   - `provider`
   - `operation`
   - `endpoint`
   - `model` where applicable
4. Pivot back to Loki using request or session correlation

## Current Runtime Shape

### HTTP request correlation

`attach_http_request_context` injects:
- `request_id`
- optional `trace_id`
- `method`
- `path`
- `user_agent`
- `received_at`

It also writes `x-request-id` back to the response and creates the request tracing span.

### Client debug ingest

`POST /debug/client-events`

Current flow:
1. validate and normalize incoming client events
2. emit one structured telemetry log record per accepted event
3. selectively bridge session-bound, business-valuable events into the `call` event pipeline

Raw client debug logs are not stored as the primary source of truth in PostgreSQL.

### App-error ingest

`POST /debug/app-errors`

Current flow:
1. validate and normalize incoming app-error reports
2. compute a stable fingerprint from category + message + primary stack frame
3. emit structured ingest logs with request correlation
4. upsert grouped issues and persist occurrences to PostgreSQL

## Internal Structure

```text
observability/
  src/
    app_error.rs
    client_event.rs
    error.rs
    error_group.rs
    http_middleware.rs
    metrics.rs
    postgres.rs
    request_context.rs
    service.rs
    tracing_init.rs
```

Key contracts:
- `ClientDebugEventSinkPort`
- `AppErrorSinkPort`
- `AppErrorQueryPort`
- `ObservabilityUseCases`

## Safety Rules For Codex

- Use the script first for status, URLs, Prometheus targets, and rules.
- Prefer `request_id` and `session_id` for correlation before broad free-text log searches.
- Do not invent high-cardinality Loki labels.
- Do not use observability paths to mutate runtime state.
- Treat `/internal/alerts/webhook` as a local verification surface, not a business API.
