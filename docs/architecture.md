# Sonari Architecture

A self-hosted real-time voice agent. This document describes how the system is built and where code belongs. For why it is built this way, see [ADR index](adr/README.md).

**Target**: sub-2s response at tens to low hundreds of concurrent calls, English-first, all models self-hosted.
**Stack**: Rust (Tokio, Axum), LiveKit, PostgreSQL, Docker.

---

## 1. Deployment

```
                          ┌──────────────┐
                          │   Browser    │
                          └──┬────────┬──┘
                             │        │
           HTTP: start/end   │        │   WebRTC: mic + speaker
               call, history │        │
                             │        ▼
                             │  ┌───────────┐
                             │  │  livekit  │
                             │  └─────┬─────┘
                             │        │
                             │        │  PCM frames — 50/sec
                             │        │
  ┌──────────────────────────▼────────▼─────────────────────────┐
  │ sonari                                                      │
  │                                                             │
  │   ┌── control plane ───┐      ┌── media plane ───────────┐  │
  │   │                    │      │                          │  │
  │   │  HTTP API          │      │   VAD ──► ASR ──┐        │  │
  │   │  demo page         │ ◄────┤                 ▼        │  │
  │   │  persistence       │ turn │         orchestrator ────┼──┼──┐
  │   │                    │ facts│                 │        │  │  │
  │   │                    │      │   mixer ◄── TTS ┘        │  │  │
  │   └─────────┬──────────┘      └──────────────────────────┘  │  │
  │             │                                               │  │
  └─────────────┼───────────────────────────────────────────────┘  │
                │                                                  │
                │ SQL                   OpenAI-compatible HTTP      │
                │                       1 call/turn — text only     │
                ▼                                                  ▼
         ┌─────────────┐                                   ┌─────────────┐
         │  postgres   │                                   │    vllm     │
         │ transcripts │                                   │ LLM on GPU  │
         └─────────────┘                                   └─────────────┘
```

| Container | Ours | Role |
|---|---|---|
| `sonari` | yes | HTTP API, voice pipeline, demo page. VAD / ASR / TTS in-process (ADR-0005) |
| `livekit` | no | WebRTC transport (ADR-0007) |
| `postgres` | no | Transcripts, turn facts, usage |
| `vllm` | no | LLM inference on GPU (ADR-0006) |

Audio flows at 50 frames/sec and never leaves the `sonari` process (ADR-0003). Everything crossing a process boundary is text or a turn-level fact — dozens per call.

Optional overlay `docker-compose.observability.yml` adds Prometheus and Grafana. Off by default.

**Roles.** One binary, three modes (ADR-0002). `sonari all` is the default and runs both planes in one process. `sonari serve` and `sonari worker` run one plane each, for horizontal scaling. CI exercises both paths.

---

## 2. Crates

Eight crates. Media-plane crates keep internals private; the planes communicate only through `sonari-core` types (ADR-0013).

| Crate | Owns | Does not own | Depends on |
|---|---|---|---|
| `sonari-core` | Domain types, provider traits, error types | Any implementation, any I/O | — |
| `sonari-pipeline` | Turn state machine, endpointing policy, sentence segmentation, barge-in, playback queue | Model inference, transport, persistence | `core`, `telemetry` |
| `sonari-providers` | `AsrEngine` / `TtsEngine` / `LlmClient` / `Vad` implementations, model loading | When to call them | `core`, `telemetry` |
| `sonari-rtc` | LiveKit rooms, tokens, track binding, PCM in/out | Anything about conversation | `core` |
| `sonari-store` | PostgreSQL schema, migrations, repositories | Business rules | `core` |
| `sonari-api` | HTTP routes, DTOs, static demo assets | Persistence details, pipeline internals | `core`, `store` |
| `sonari-telemetry` | Latency markers, metrics, tracing setup | Interpretation of what it records | — |
| `sonari` | Composition root, config loading, role subcommands, shutdown | Any logic | all |

`sonari-api` and `sonari-store` must not depend on `sonari-pipeline`, `sonari-providers`, or `sonari-rtc`. Only `sonari` sees both planes.

---

## 3. Domain model

Defined in `sonari-core`. No tenant dimension (ADR-0011).

| Type | Meaning | Lifetime |
|---|---|---|
| `Persona` | System prompt, voice, model parameters, enabled tools | Loaded from config at startup |
| `Session` | One call, from connect to hangup | In memory for its duration; a summary row is persisted |
| `Turn` | One exchange: user utterance → agent reply | In memory while active; persisted on completion |
| `Transcript` | Final text of one utterance or reply | Persisted |
| `TurnFacts` | Timing markers, token usage, interruption flag | Persisted on turn completion |
| `AudioSegment` | One synthesized sentence, PCM plus metadata | Discarded after playback |

Only `Session`, `Turn`, `Transcript`, and `TurnFacts` cross into the control plane. `AudioSegment` and everything below it stay in the media plane.

**In-flight state is never persisted.** Only completed facts reach the database (ADR-0012).

---

## 4. Turn lifecycle

The state machine in `sonari-pipeline`. One instance per session, owned by one task.

```
                    ┌──────────┐
                    │   Idle   │
                    └────┬─────┘
                         │ session established
                         ▼
                  ┌─────────────┐◄──────────────────────┐
              ┌──►│  Listening  │                       │
              │   └──────┬──────┘                       │
              │          │ VAD onset                    │ playback drained
              │          ▼                              │
              │   ┌─────────────┐                ┌──────┴──────┐
              │   │  Capturing  │                │ Responding  │
              │   └──────┬──────┘                └──────┬──────┘
              │          │ VAD offset                   │ VAD onset
              │          ▼                              ▼  (barge-in)
              │   ┌─────────────┐  hangover      ┌─────────────┐
              │   │ Endpointing ├───elapsed─────►│ Interrupted │
              │   └──────┬──────┘                └──────┬──────┘
              │          │ speech resumed               │ teardown done
              └──────────┴──────────────────────────────┘
                                                   (→ Capturing)
```

| State | What runs | Exits when |
|---|---|---|
| `Listening` | VAD only. Frames buffered in a short pre-roll ring | VAD reports onset → `Capturing` |
| `Capturing` | Pre-roll flushed to ASR, then frames stream in; partials arrive | VAD reports offset → `Endpointing` |
| `Endpointing` | Frames still stream to ASR; hangover timer runs | Timer elapses → `Responding`; speech resumes → `Capturing` |
| `Responding` | LLM streams, sentences segment and synthesize, segments play | Queue drains → `Listening`; VAD onset → `Interrupted` |
| `Interrupted` | Playback stopped, unplayed segments dropped, LLM stream and pending synthesis cancelled | Teardown completes → `Capturing` |

**Pre-roll.** `Listening` retains a short ring buffer of recent frames, flushed to ASR on transition to `Capturing`, so the first phoneme is not clipped by VAD reaction time. Buffer length is configuration.

**Endpointing.** Hangover length trades false endpoints against perceived latency and is configuration. Both effects are measured (ADR-0010).

**Barge-in.** `Interrupted` is entered from the VAD path, not from playback, so it takes effect on the next frame. It must be idempotent: a second onset during teardown is a no-op.

---

## 5. Two layers

The conversation core is audio-agnostic and independently usable (ADR-0008).

```
              ┌───────────────────────────────┐
 text in ────→│  core conversation loop        │────→ text out
              │  prompt → LLM → tools → memory │
              └───────────────────────────────┘
                      ↑                ↓
voice in → VAD → ASR ─┘                └── sentence split → TTS → playback
```

The inner layer takes text and returns a token stream. It knows nothing of frames, sentences, or timing. Sentence segmentation belongs to the outer layer, because it exists to drive synthesis (ADR-0009).

Both entrypoints are supported surfaces. The eval harness drives the inner layer directly for answer quality and the outer layer with WAV files for latency and word error rate — neither path needs LiveKit or a browser.

---

## 6. Provider interfaces

Declared in `sonari-core`, implemented in `sonari-providers`.

> Shapes below are the intended contract. They must be validated against the `sherpa-onnx` API before being fixed.

```rust
pub trait Vad: Send {
    fn push(&mut self, frame: &[i16]) -> VadState;   // Silence | Speech
    fn reset(&mut self);
}

pub trait AsrEngine: Send + Sync {
    fn open(&self, cfg: &AsrConfig) -> Result<Box<dyn AsrStream>>;
}

pub trait AsrStream: Send {
    fn push(&mut self, frame: &[i16]) -> Result<()>;
    fn poll(&mut self) -> Option<AsrEvent>;          // Partial | Final
    fn finish(&mut self) -> Result<Transcript>;
}

#[async_trait]
pub trait TtsEngine: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &VoiceId) -> Result<AudioSegment>;
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn stream(&self, req: LlmRequest) -> Result<BoxStream<'_, Result<LlmDelta>>>;
}

pub enum LlmDelta { Token(String), ToolCall(ToolCall), Done(Usage) }
```

**Asymmetry is intentional.** ASR is push/poll because it is fed at frame rate and produces results on its own schedule. TTS is a single call because it is driven per sentence. `LlmClient` streams and carries `ToolCall` from the outset — retrofitting either would restructure the pipeline (ADR-0006).

### Selected implementations

| Role | Implementation | Notes |
|---|---|---|
| VAD | sherpa-onnx built-in | Same runtime as ASR |
| ASR | NVIDIA Nemotron Speech Streaming En 0.6B | Cache-aware FastConformer. Chunk size tunable at runtime, 80 ms–1.12 s; WER 8.43% → 6.93% across that range. NVIDIA Open Model License |
| TTS | Piper (Kokoro alternate) | ~40 ms to first audio, RTF 0.03, CPU |
| LLM | Any OpenAI-compatible endpoint | Self-hosted and hosted differ only by base URL |

ASR chunk size is a runtime latency/accuracy dial; the eval harness plots the curve.

---

## 7. Concurrency

| Unit | Cardinality | Responsibility |
|---|---|---|
| Session task | one per call | Owns the state machine and all turn state. Single owner, no locks |
| Ingress | one per call | LiveKit track → bounded channel → session task |
| Inference workers | shared pool | ASR and TTS inference, dispatched via `spawn_blocking` |
| Playback task | one per call | Drains the segment queue at real-time cadence; checks the interrupt flag each frame |
| HTTP server | one | Control plane; touches no session state |

**Session state has exactly one owner.** Everything else communicates with it by channel. There is no shared mutable conversation state, therefore no lock ordering and no cross-process consistency problem (ADR-0012).

**Inference must not run on the async runtime.** ONNX inference is blocking CPU work; running it inline would stall unrelated sessions. It is dispatched to a blocking pool sized against available cores.

**Backpressure.** The ingress channel is bounded. If the pipeline cannot keep up, frames are dropped at ingress with a counter incremented — never buffered without limit. Unbounded audio buffering converts a throughput problem into an unbounded-memory problem.

---

## 8. Latency instrumentation

Emitted by `sonari-telemetry`, present from phase one (ADR-0010).

| Marker | Meaning |
|---|---|
| `speech_start` | VAD onset |
| `speech_last_voiced` | Last frame classified as speech |
| `speech_end` | Endpoint declared, hangover elapsed |
| `asr_final` | Final transcript available |
| `llm_first_token` | First token received |
| `llm_first_sentence` | First complete sentence segmented |
| `tts_first_chunk` | First synthesized audio available |
| `audio_first_frame` | First frame handed to transport |

Two figures, always reported together:

- **System response** — `speech_end` → `audio_first_frame`. The optimization target.
- **Perceived latency** — `speech_last_voiced` → `audio_first_frame`. What the user waits, including hangover.

**No latency figure appears in any document until it has been measured.**

---

## 9. Failure handling

| Failure | Effect | Recovery |
|---|---|---|
| LLM unreachable or times out | Turn fails | Spoken error notice; session continues |
| TTS fails on one sentence | That segment is skipped | Continue with remaining sentences; mark the turn degraded |
| ASR produces no final transcript | Turn produces nothing | Return to `Listening` without a reply |
| ASR model fails to load at startup | Voice unavailable | Serve text mode only; report unhealthy (ADR-0008) |
| Native runtime segfault | **Process terminates** | Container restart. No in-process recovery (ADR-0005) |
| LiveKit connection lost | Session ends | Client reconnects and starts a new session |
| PostgreSQL unreachable | Facts not persisted | Calls continue; persistence failures are logged, never block the audio path |

**Persistence never blocks audio.** Writes are dispatched off the session task; a database outage degrades record-keeping, not conversation.

---

## 10. Configuration

Files and environment variables only. No configuration in the database — `docker compose up` must be sufficient to hold a conversation.

| Source | Contents |
|---|---|
| `sonari.toml` | Personas, VAD thresholds, endpoint hangover, ASR chunk size, voice selection, model paths |
| Environment | Endpoint URLs, credentials, database DSN, log level |
| `.env.example` | Every variable, documented, no real values |

Secrets appear only in the environment, never in files under version control.

---

## 11. Data model

Completed facts only (ADR-0012), no tenant columns (ADR-0011).

| Table | Contents |
|---|---|
| `sessions` | One row per call: persona, start, end, outcome |
| `turns` | One row per completed turn: session, index, timing markers, token usage, interrupted flag |
| `transcripts` | User utterances and agent replies, linked to a turn |

Long-term memory (pgvector) is a later addition and will extend this schema; the `postgres` image is chosen to allow it without replacement.

---

## 12. Extension points

**Adding an ASR, TTS, or LLM provider.** Implement the trait in `sonari-providers`, register it in the composition root, select it by configuration. No change to `sonari-pipeline`.

**Adding a tool.** Tools are declared per persona and dispatched by the conversation core. A tool is a name, a JSON schema, and a handler; it never touches audio.

**Changing transport.** All LiveKit-specific code is confined to `sonari-rtc`. The pipeline consumes a PCM stream and produces one; the source is replaceable — the eval harness already substitutes files for a browser.

**Splitting the deployment.** Because no crate spans both planes, `sonari serve` and `sonari worker` differ only in what the composition root wires up.
