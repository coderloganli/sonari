# ADR-0007: Use LiveKit as the WebRTC transport

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `audio`, `ops`
- **Related**: ADR-0003, ADR-0010

## Context

Getting microphone audio from a browser to a server in real time requires solving echo cancellation, noise suppression, automatic gain control, jitter buffering, packet loss concealment, codec negotiation, and NAT traversal. Browsers implement the first three inside the WebRTC audio pipeline, exposed through `getUserMedia` constraints; the rest are handled by the WebRTC stack itself. Reimplementing any of this over a raw WebSocket is not a reasonable use of effort.

Echo cancellation is not a refinement here — it is load-bearing. Barge-in detects user speech during playback. Without AEC, the microphone picks up the agent's own output, VAD reports speech, playback stops, silence returns, playback resumes, and the system interrupts itself in a loop. The feature does not function without it.

WebRTC is designed peer-to-peer; reaching a server requires a media server. LiveKit is open source, self-hostable — consistent with the project's zero-external-dependency goal — publishes a Rust SDK, and also ships native iOS and Android SDKs, so a future mobile client needs no change of transport.

## Decision

Use LiveKit as the WebRTC media server, deployed from its official image. Confine all LiveKit-specific code to the `sonari-rtc` crate.

LiveKit's responsibility ends at delivering clean PCM to the pipeline; it performs no VAD, recognition, or conversational logic. Conversely the pipeline knows nothing of codecs, packet loss, or NAT.

## Consequences

- Echo cancellation, noise suppression, and gain control are handled before audio reaches us.
- A future iOS client reuses the same transport.
- LiveKit is one more container in the default deployment.
- Because transport is isolated in `sonari-rtc`, it is one input source among several rather than part of the pipeline — the eval harness feeds WAV files directly and runs without LiveKit or a browser (ADR-0010).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Raw PCM over WebSocket | No echo cancellation, so barge-in cannot work; also requires implementing jitter buffering and loss concealment |
| A different SFU (mediasoup, Janus, Pion) | Viable, but no Rust SDK of comparable maturity and no first-party mobile SDKs |
| A hosted real-time audio service | Contradicts the self-hosted goal and introduces an external dependency in the hot path |
