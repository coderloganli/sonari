# Architecture Decision Records

One decision per record. Accepted records are immutable — a changed decision produces a new record that supersedes the old one, and both stay on disk.

Format: [template.md](template.md). Rationale: [ADR-0001](0001-record-architecture-decisions.md).

## How to query this without reading everything

**This index is designed to answer most questions on its own.** The `Decision` column states what was decided, not just what the record is about. Open a record only when you need the reasoning, the costs, or the rejected alternatives — typically because you are about to change it.

```bash
# What was decided about latency, without opening anything
rg '\| `latency`' docs/adr/README.md

# Just the decision paragraphs across every record
rg -A4 '^## Decision' docs/adr/

# Current status of everything
rg '\*\*Status\*\*:' docs/adr/

# What has been revisited
rg -l 'Superseded' docs/adr/
```

Reading the whole directory costs roughly 400 tokens per record. Reading this index costs a few lines per record. Prefer the index, then open at most two or three files.

## Active decisions

| # | Decision | Status | Tags | Date |
|---|---|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Architectural decisions are recorded as numbered, immutable ADRs; `architecture.md` stays free of rationale | Accepted | `scope` | 2026-08-05 |
| [0002](0002-one-process-two-planes.md) | One binary containing both planes; the split is logical, with `serve`/`worker` roles reserved for later scale-out | Accepted | `process` `scope` | 2026-08-05 |
| [0003](0003-audio-never-crosses-a-process-boundary.md) | No PCM-handling component sits across a process boundary from the audio source | Accepted | `audio` `process` `latency` | 2026-08-05 |
| [0004](0004-colocate-orchestration-with-audio.md) | The turn state machine runs beside VAD/ASR/TTS — a remote component may be a leaf, never the conductor | Accepted | `audio` `process` `latency` | 2026-08-05 |
| [0005](0005-in-process-vad-asr-tts-via-sherpa-onnx.md) | VAD, ASR and TTS link into the binary via sherpa-onnx; a native segfault kills the process, accepted knowingly | Accepted | `providers` `audio` `process` | 2026-08-05 |
| [0006](0006-llm-as-external-openai-compatible-service.md) | The LLM is reached over the OpenAI-compatible HTTP interface, streaming with tool calls, endpoint configurable | Accepted | `providers` `process` `ops` | 2026-08-05 |
| [0007](0007-livekit-as-webrtc-transport.md) | LiveKit provides WebRTC transport; browser-side echo cancellation is a precondition for barge-in | Accepted | `audio` `ops` | 2026-08-05 |
| [0008](0008-text-core-as-first-class-entrypoint.md) | The conversation core is audio-agnostic and separately addressable, enabling layered eval and text degradation | Accepted | `scope` `providers` | 2026-08-05 |
| [0009](0009-sentence-level-tts-pipelining.md) | Synthesis is driven per sentence as the LLM streams, so non-streaming TTS is sufficient | Accepted | `latency` `audio` `providers` | 2026-08-05 |
| [0010](0010-latency-instrumentation-from-phase-one.md) | Eight markers ship before any model; both system response and perceived latency are always reported | Accepted | `latency` `ops` | 2026-08-05 |
| [0011](0011-no-multi-tenancy.md) | No tenant dimension in any type, table, or interface; personas are configuration, not isolation | Accepted | `scope` `data` | 2026-08-05 |
| [0012](0012-no-redis.md) | Turn state is an in-memory value with one owner; facts go straight to PostgreSQL | Accepted | `data` `ops` | 2026-08-05 |
| [0013](0013-compiler-enforced-plane-boundaries.md) | Eight crates; media-plane internals stay private so the plane boundary is a compile error, not a review comment | Accepted | `process` `scope` | 2026-08-05 |

## Superseded

_None yet._ When a record is superseded, move its row here and note the replacement.

## By tag

`process` 0002 0003 0004 0005 0006 0013 · `audio` 0003 0004 0005 0007 0009 · `latency` 0003 0004 0009 0010 · `providers` 0005 0006 0008 0009 · `data` 0011 0012 · `ops` 0006 0007 0010 0012 · `scope` 0001 0002 0008 0011 0013

Tags are a closed vocabulary. Adding one requires a decision about what it means.

## Reading order

`0003` and `0004` are the constraints everything else follows from. `0002` is what they imply for deployment. The rest are details.

## Keeping this index honest

An index that drifts is worse than none. CI enforces:

1. Every `NNNN-*.md` has exactly one row here, and every row has a file
2. Numbers are unique and contiguous
3. `Status` is one of `Accepted`, `Proposed`, `Superseded by ADR-NNNN`, `Deprecated`
4. Every tag used appears in the closed vocabulary above
5. A `Superseded` record names its replacement, and the replacement exists

## When this grows past ~50 records

- Split `Active` by tag into sub-tables; keep one row per record
- Move superseded rows to `adr/ARCHIVE.md`, leaving only a count here
- Keep the `Decision` column to one line. If it will not fit, the ADR is doing more than one thing
