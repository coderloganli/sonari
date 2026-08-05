# ADR-0006: Run the LLM as an external OpenAI-compatible service

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `providers`, `process`, `ops`
- **Related**: ADR-0003, ADR-0005

## Context

ADR-0003 keeps audio inside the process, but says nothing about text. The LLM's profile differs from ASR's on every axis that matters:

| | ASR | LLM |
|---|---|---|
| Call rate | ~50/sec | 1 per turn |
| Payload | audio frames | a few KB of text |
| Hardware | CPU | GPU |
| Own latency | microseconds | hundreds of milliseconds |
| **Transport cost ÷ own latency** | significant, paid per frame | ~0.3%, paid once per turn |

The last row is the test. Where that ratio is small, a process boundary is nearly free; where it is large, the boundary dominates.

Two further factors point the same way. The LLM is the only component requiring a GPU, and the target hardware (RTX 3060 Ti, 8 GB) cannot host it alongside anything else — development runs a quantized model locally under WSL2, while published measurements come from a cloud GPU instance. Separating it means changing a URL, not a deployment topology. Second, the mature serving runtimes are Python-ecosystem projects; embedding one in a Rust binary serves no purpose.

vLLM exposes an OpenAI-compatible HTTP API. Adopting that interface makes self-hosted inference and commercial APIs the same code path, distinguished only by base URL — which yields the self-hosted-versus-API comparison as a by-product rather than as separate work.

## Decision

Call the LLM over HTTP using the OpenAI-compatible chat completions interface, with streaming enabled and tool calling supported from the outset. Ship vLLM in the default compose file. The endpoint is configuration.

## Consequences

- The GPU can live on another machine without any code change.
- Swapping models, runtimes, or providers is a configuration change.
- "Self-hosted versus API" latency and cost comparisons run the same eval twice against different URLs.
- The compose file carries a container whose startup is slow, because it loads model weights. Health checks must account for this.
- Sonari must handle LLM unavailability and timeouts as a normal condition, not an assertion failure.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Link an inference engine in-process (e.g. llama.cpp bindings) | Ties the binary to one runtime, forces the GPU onto the same host as the media plane, and buys latency that is noise against hundreds of milliseconds of generation |
| A bespoke provider interface with per-vendor adapters | The OpenAI-compatible schema is already the de facto interface for self-hosted serving runtimes; inventing another one adds an adapter layer with no gain |
| Non-streaming completion | Forecloses sentence-level pipelining (ADR-0009), which is a primary lever for the latency target |
