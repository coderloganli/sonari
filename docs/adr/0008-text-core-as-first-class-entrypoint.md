# ADR-0008: Make the text conversation core a first-class entrypoint

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `scope`, `providers`
- **Related**: ADR-0005, ADR-0009, ADR-0010

## Context

The obvious pipeline shape is a single line: audio in, audio out. Under that shape the conversational logic — prompt assembly, LLM interaction, tool calls, memory retrieval — is only reachable through the audio path.

Three problems follow. Iterating on a persona or a tool definition requires starting LiveKit and speaking into a microphone. Evaluation cannot separate concerns: when answer quality regresses, there is no way to tell whether recognition misheard the input or the model reasoned worse. And when speech components fail there is nothing left to degrade to, even though the model itself is still healthy.

The last point matters more than it first appears. A crash in the linked native runtime terminates the process (ADR-0005), but the far more common failures — a model file that fails to load, a device that is unavailable, a synthesis timeout — leave the conversational core entirely functional.

## Decision

Structure the pipeline as two layers. The inner layer is a pure text conversation loop — prompt, LLM, tools, memory — with no knowledge that audio exists. The outer layer wraps it with recognition on the way in and synthesis on the way out.

```
              ┌───────────────────────────────┐
 text in ────→│  core conversation loop        │────→ text out
              │  prompt → LLM → tools → memory │
              └───────────────────────────────┘
                      ↑                ↓
voice in → VAD → ASR ─┘                └── sentence split → TTS → playback
```

Both entrypoints are supported surfaces, not one real path and one test hook.

## Consequences

- Persona, prompt, tool, and memory work proceeds without the audio stack.
- Evaluation splits cleanly: the text layer measures answer quality, the voice layer measures latency and word error rate. A regression localizes to one layer.
- Speech failures degrade to text instead of ending the call.
- The layering is not additional structure — it is where the boundary already was. Recognition and synthesis are adapters over a conversation loop that never needed to know about them.
- Cost: the inner layer's interface must stay audio-agnostic. Anything sentence- or timing-related belongs to the outer layer, which takes discipline where the two meet.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Single audio-only path | Cannot iterate without the audio stack, cannot localize eval regressions, has no degradation target |
| Text mode as a debug-only flag | The same code with weaker guarantees; a path that is not a supported surface will not stay working |
| Separate text and voice pipelines | Two implementations of one conversation loop, guaranteed to diverge |
