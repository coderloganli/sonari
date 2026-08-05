# ADR-0002: Ship one service containing two planes

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `process`, `scope`
- **Related**: ADR-0003, ADR-0004, ADR-0013

## Context

Sonari's work splits into two categories with radically different performance characteristics:

- **Media plane** — audio arrives at ~50 frames per second and barge-in must take effect within ~100 ms
- **Control plane** — a handful of HTTP requests per call, latency-insensitive

The conventional response is to deploy these as separate services. The four standard arguments for that were each tested against this project at its target scale (tens to low hundreds of concurrent calls):

| Argument | Applies? |
|---|---|
| Independent deployment | No — a full redeploy takes seconds |
| Independent scaling | No — the bottleneck is the media plane, which cannot be split (ADR-0003); the splittable part is not the bottleneck |
| Technology heterogeneity | No — both are Rust |
| Team autonomy | No — single maintainer |

Fault isolation is the one argument with residual merit: the text-only core (ADR-0008) remains usable when speech fails, and a crash in linked native code takes down the whole process (ADR-0005). Neither justifies a network boundary between the two planes — the degradation path is error handling, not process separation.

## Decision

Ship a single binary. Separate the two planes as a **logical** boundary enforced at compile time (ADR-0013), not as a deployment boundary.

Provide role subcommands so the same binary can later be deployed split without an interface change:

```
sonari all      # default: both planes, one process
sonari serve    # control plane only
sonari worker   # media plane only
```

CI runs the same eval suite in both `all` and split modes, so the seam is verified rather than asserted.

## Consequences

- `docker compose up` starts one service of ours; local debugging is one process and one log stream.
- No shared state store is required for orchestration, which removes the need for Redis (ADR-0012).
- Scaling out later is a deployment change, not a rewrite — provided CI keeps the split path honest.
- Cost: within one process, nothing physically prevents the control plane from reaching into media-plane internals. ADR-0013 addresses this with crate visibility.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Two services from the start | Pays network, deployment, and distributed-state costs immediately for benefits that do not apply at this scale |
| One process with no internal plane boundary | Removes the seam entirely; converting to a split deployment later becomes a rewrite |
| Media plane only, no HTTP surface | Leaves no home for transcripts, history, or long-term memory, and pushes LiveKit token issuance onto the client |
