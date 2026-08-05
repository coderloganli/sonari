# ADR-0004: Colocate orchestration with the audio path

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `audio`, `process`, `latency`
- **Related**: ADR-0003, ADR-0002

## Context

ADR-0003 forbids audio from crossing a process boundary, but orchestration — the state machine deciding when to listen, when to think, and when to speak — never handles PCM. Its inputs are VAD events and transcripts; its outputs are prompts, sentences, and playback commands. On interface shape alone it could be deployed separately.

Two constraints say otherwise.

**Barge-in is hard real-time.** When the user interrupts, playback must stop within roughly 100 ms or the system reads as unresponsive. That decision path — speech detected, decide to stop, stop — cannot absorb a network hop, a retry, or a queue.

**Orchestration cannot be extracted alone.** VAD, ASR, and TTS all touch audio and must stay local by ADR-0003. Extracting only the state machine leaves a few hundred lines of pure-CPU logic in its own process, needing no independent scaling and no independent release.

This is precisely where the predecessor went wrong. It placed orchestration in the backend service alongside the recognizer, while audio capture and playback lived in a separate worker. The recognizer needed audio, so audio followed orchestration across the boundary. Every downstream pathology — instance affinity, per-turn state in PostgreSQL, 60 ms polling for synthesized audio — descends from that one placement.

The rule is therefore sharper than "audio stays local": **a remote component may be a leaf, but never the conductor.** A stateless recognizer that takes audio and returns text is a leaf. A state machine that decides when playback stops is not.

## Decision

Orchestration lives in the same process as VAD, ASR, TTS, and playback. The four are treated as one indivisible unit.

## Consequences

- Barge-in is a memory write observed by the playback loop on its next frame.
- Turn state is an in-memory value with a single owner, so it needs no external store and no cross-process consistency protocol.
- The media plane cannot be decomposed further; scaling it means running more whole instances of it (ADR-0002).

## Alternatives considered

| Alternative | Why not |
|---|---|
| Orchestration as its own service | Isolates a few hundred lines that need neither independent scaling nor independent release, while putting a network hop inside the barge-in path |
| Orchestration in the control plane | The predecessor's arrangement. Drags audio across the boundary, because the recognizer sits beside the orchestrator |
| Orchestration duplicated on both sides | Two authorities over one state machine; divergence is a matter of time |
