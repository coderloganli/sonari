# ADR-0015: Delete the local inference implementations rather than retain them

- **Status**: Accepted
- **Date**: 2026-08-08
- **Tags**: `providers` `scope`
- **Related**: ADR-0005, ADR-0014

## Context

ADR-0014 moves recognition and synthesis to hosted providers. The local
implementations they replace work: they were built against sherpa-onnx, covered
by tests that drive real audio through real models, and measured — first
synthesis chunk at 83 ms, recognition final at 0.1 ms after speech end.

Keeping them behind a configuration switch would preserve an offline mode, a
control group for latency comparisons, and a fallback if a provider fails.

## Decision

Delete them. `providers` keeps voice activity detection and nothing else.

## Consequences

The pipeline has one implementation of each stage and one set of failure modes.
Nobody has to ask which path a measurement came from, and no configuration
combination exists that was never run.

Offline operation ends. So does the ability to answer "how much worse is local"
with a number, which the original proposal listed as an acceptance criterion —
that criterion no longer applies and the document says so.

Returning to local inference means writing the adapters again. The trait they
implemented is unchanged, so what returns is the adapter, not the design; the
work is bounded and the shape is known.

## Alternatives considered

**Retain behind configuration.** The usual argument is that the code already
works, so keeping it is free. It is not: every path that can be selected has to
keep compiling, keep being tested, and keep being understood by whoever reads
the composition root. A second inference path that nobody runs is a second
inference path that quietly stops working.

**Retain only for the eval harness.** The harness measures the pipeline that
ships. A harness measuring a different pipeline reports numbers about nothing.
