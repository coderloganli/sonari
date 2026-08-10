# ADR-0017: Observability is structured logs read by an agent, not a dashboard

- **Status**: Accepted
- **Date**: 2026-08-08
- **Tags**: `ops` `latency`
- **Related**: ADR-0010

## Context

ADR-0010 requires eight latency markers from phase one and both derived figures
always reported together. It left open how they are delivered, and the inherited
code carried a Prometheus registry, a `/metrics` route and an OTLP exporter.

Nobody here reads a dashboard. The consumer of this system's telemetry is a
coding agent: it greps text, correlates by identifier, and computes what it needs.

The two audiences want opposite things. A dashboard wants pre-aggregated series
and a UI. An agent wants one event per line, raw, with the identifiers that let
it reconstruct a call, and no service standing between it and the data.

## Decision

Every event is one structured JSON line carrying `session_id`, and `turn` where
it applies. Latency markers carry elapsed values as explicit fields rather than
timestamps to be subtracted. Spans are logged as start and end events correlated
by span id.

No metrics backend, no collector, no visualisation. The Prometheus dependency,
the `/metrics` route and the OTLP exporter are removed.

## Consequences

Two containers fewer to run and to explain, which for a project whose first ten
minutes decide whether anyone stays is worth more than the graphs.

Percentiles are computed by whatever reads the logs, not by a backend. The eval
harness does this over a golden set; ad-hoc questions are answered with `rg`.

Long-running trend analysis becomes harder: there is no time-series store, so
"how did p95 move over three months" needs the logs retained and processed. That
is accepted — this is a project being built, not a service being operated.

An operator who does want dashboards can point a log shipper at the files. What
is given up is the built-in path, not the possibility.

## Alternatives considered

**Keep Prometheus, add logs.** Two systems recording the same facts, disagreeing
eventually. The metrics were also the wrong shape: a counter cannot answer "what
happened in that call", which is the question actually asked.

**OTLP traces without a UI.** Traces carry causality better than logs, and a
coding agent could read them — but only through a collector, which is the
container the decision was trying to avoid. Span start and end as log lines keep
the causality and drop the dependency.
