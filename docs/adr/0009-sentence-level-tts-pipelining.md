# ADR-0009: Drive TTS at sentence granularity

- **Status**: Superseded by ADR-0014
- **Date**: 2026-08-05
- **Tags**: `latency`, `audio`, `providers`
- **Related**: ADR-0005, ADR-0006, ADR-0010

## Context

The naive sequence waits for the LLM to finish generating, then synthesizes the whole reply, then plays it. Under that arrangement the user waits for the full generation before hearing anything — for a three-sentence answer, easily over a second of avoidable silence.

Two facts make a better arrangement available. The LLM streams tokens (ADR-0006), so partial output is usable as it arrives. And synthesis is far faster than real time: Piper reports first audio at roughly 40 ms and a real-time factor near 0.03, meaning one second of speech costs about 30 ms to produce.

This also resolves what would otherwise be a constraint from ADR-0005: `sherpa-onnx` exposes only non-streaming synthesis, returning a complete buffer per call. At sentence granularity that is not a limitation — each call produces one to two seconds of audio, and the next sentence finishes synthesizing well inside the playback of the current one.

The predecessor project did not implement this; its own design notes list sentence-level LLM-to-TTS overlap explicitly among unimplemented features that must not be claimed.

## Decision

Segment the LLM's token stream into sentences as it arrives. Dispatch each completed sentence to synthesis immediately and enqueue the result for playback, while generation continues on subsequent sentences.

## Consequences

- Time to first audio depends on the first sentence, not the full reply.
- Non-streaming synthesis becomes adequate, so ADR-0005 needs no exception.
- Playback becomes a queue of segments rather than one buffer, which is also what barge-in needs: interruption discards unplayed segments and cancels pending synthesis.
- Sentence segmentation is now on the critical path and must be cheap and incremental. It must handle abbreviations, decimals, and text that never terminates a sentence — a maximum-length fallback is required so an unpunctuated stream still produces audio.
- A synthesis failure mid-reply affects one segment. Recovery policy — skip, retry, or abandon the turn — is a pipeline concern and is specified in `architecture.md`.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Synthesize the complete reply | Time to first audio scales with reply length; the dominant avoidable cost |
| Synthesize per token or per fixed window | Prosody breaks down; boundaries fall mid-word and the result does not sound like speech |
| Require a streaming TTS engine | Narrows engine choice for a benefit sentence granularity already provides |
