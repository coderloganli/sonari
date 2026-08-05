# ADR-0003: Audio never crosses a process boundary

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `audio`, `process`, `latency`
- **Related**: ADR-0004, ADR-0005, ADR-0006

## Context

Audio is not a request — it is a continuous stream. At 20 ms per frame a five-minute call produces roughly fifteen thousand frames. Any process boundary placed on the audio path converts that stream into an equal number of network round trips.

This is not hypothetical. The predecessor project placed exactly such a boundary, then transported each frame as an individual HTTP POST carrying JSON-encoded PCM. Encoding `i16` samples as decimal text inflated the payload by roughly 2.7×, on top of per-request headers and parse cost. The return path polled for synthesized audio every 60 ms. The consequences compounded: because the recognizer connection lived on the far side, sessions became pinned to the instance that opened it, requiring roughly a hundred sites of instance-affinity code to keep traffic routed correctly.

A better protocol reduces this cost substantially — a binary WebSocket frame carries 2–14 bytes of framing versus hundreds for an HTTP request, and gRPC bidirectional streams or same-host shared memory do better still. But protocol choice does not address the two structural consequences: state that both sides must observe has to be externalized, and timing becomes non-deterministic, which must be absorbed by buffering, which is latency.

## Decision

No component that consumes or produces PCM sits on the far side of a process boundary from the audio source. VAD, ASR, TTS, and the playback buffer all execute inside the Sonari process.

## Consequences

- Passing a frame between pipeline stages is a function call, not a network operation.
- No jitter buffer is needed between internal stages, because there is no jitter to absorb.
- Session state has no reason to be externalized, since only one process observes it (ADR-0012).
- The ASR and TTS engines must therefore be linked into the process, which constrains the build (ADR-0005).
- This rule constrains audio only. Text and turn-level facts cross boundaries freely — that is what makes ADR-0006 acceptable.

## Alternatives considered

| Alternative | Why not |
|---|---|
| ASR/TTS as a sibling container over binary WebSocket | Cheaper than the predecessor's mistake by one to two orders of magnitude, but still pays a per-frame crossing and reintroduces a failure boundary in the hot path. Reconsider only if measurement shows in-process linking is untenable |
| Shared memory ring buffer between local processes | Sub-30 µs and technically sound, but adds an IPC layer to be designed, debugged, and made portable — for a boundary that has no reason to exist |
| Keep audio local but move orchestration out | Addressed separately in ADR-0004; this is what the predecessor did, and audio was dragged along behind it |
