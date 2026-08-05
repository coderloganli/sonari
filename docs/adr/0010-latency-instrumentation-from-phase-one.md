# ADR-0010: Instrument latency in phase one and report two figures

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `latency`, `ops`
- **Related**: ADR-0008, ADR-0009

## Context

Sub-2s response is the project's headline claim, which makes measurement a deliverable rather than a diagnostic aid.

The predecessor illustrates the failure mode. Its architecture document placed latency statistics explicitly out of scope, and its voice path contained a single latency-adjacent log line. Its proposal nevertheless claimed reproducible sub-2s response. Both statements were written in good faith; neither could be checked.

There is a second, subtler trap in how the figure is defined. Endpointing works by waiting out a silence interval — typically 500–800 ms — before declaring the utterance complete. Measuring from that declaration excludes the wait, producing a number that is real but is not what the user experiences:

```
user's last sound
   │
   │  ← endpoint hangover, 500–800 ms
   ▼
speech_end
   │
   │  ← measuring only this understates the wait
   ▼
first audio out
```

Reporting only the post-endpoint figure would repeat the predecessor's error in a more defensible-looking form.

## Decision

Emit seven markers per turn, from phase one, before any model is connected:

| Marker | Meaning |
|---|---|
| `speech_start` | VAD detects onset |
| `speech_last_voiced` | last frame classified as speech |
| `speech_end` | endpoint declared, hangover elapsed |
| `asr_final` | final transcript available |
| `llm_first_token` | first token received |
| `llm_first_sentence` | first complete sentence segmented |
| `tts_first_chunk` | first synthesized audio available |
| `audio_first_frame` | first frame handed to transport |

Report two figures, always together and always labelled:

- **System response** — `speech_end` → `audio_first_frame`. Comparable across configurations; the optimization target.
- **Perceived latency** — `speech_last_voiced` → `audio_first_frame`. What the user actually waits. Includes hangover.

**No latency figure enters any document until it has been measured.**

## Consequences

- Endpoint hangover becomes a visible, tunable trade-off rather than a hidden cost. The eval harness plots hangover against both false-endpoint rate and perceived latency.
- Every stage boundary must be an explicit moment in the pipeline, which constrains its structure: recognition, generation, segmentation, and synthesis cannot be fused into one opaque step.
- The demo page shows the per-stage breakdown live, so the claim is visible rather than asserted.
- Cost: two figures require explanation wherever they appear. Reporting one would be simpler and less honest.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Report post-endpoint latency only | Understates the wait by 500–800 ms; the number sounds better than the experience |
| Report perceived latency only | Confounds endpointing policy with pipeline performance; a hangover change would look like a regression |
| Add instrumentation once the pipeline works | The measurement is the deliverable, and retrofitting markers means restructuring code already written around fused stages |
