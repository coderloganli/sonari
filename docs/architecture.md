# Sonari Architecture

A real-time voice agent. This document describes how the system is built and
where code belongs. For why it is built this way, see the [ADR index](adr/README.md).

**Target**: sub-2s response, English-first.
**Stack**: Rust (Tokio, Axum), LiveKit, PostgreSQL, Docker.

---

## 1. Deployment

```
  ┌─────────────┐
  │   client    │   uid → token, choose persona, talk
  └──────┬──────┘
         │ HTTPS: create session; start/end call
         │ WebRTC: mic + speaker
         ▼
  ┌───────────┐
  │  livekit  │
  └─────┬─────┘
        │ PCM frames — 50/sec
        ▼
  ┌──────────────────────────────────────────┐
  │ sonari — one process                      │
  │                                           │
  │   VAD ──► ASR ──► agent loop              │
  │                       │                   │
  │   mixer ◄── TTS ◄─────┘                   │
  └───┬─────────────────┬─────────────────┬───┘
      │ SQL             │ WebSocket/HTTPS │ HTTPS
      ▼                 ▼                 ▼
 ┌──────────┐    ┌──────────────┐  ┌────────────┐
 │ postgres │    │  ElevenLabs  │  │ LLM        │
 │          │    │  ASR and TTS │  │ endpoint   │
 └──────────┘    └──────────────┘  └────────────┘
```

| Container | Ours | Role |
|---|---|---|
| `sonari` | yes | HTTP API, voice pipeline, VAD in-process |
| `livekit` | no | WebRTC transport (ADR-0007) |
| `postgres` | no | Sessions, turns, transcripts |

**Audio crosses a process boundary** (ADR-0014). Frames go to recognition as
they arrive; synthesised audio comes back the same way. Only voice activity
detection is local, because it runs on every frame and decides both when a turn
begins and when it ends.

**One binary.** The control plane and the media plane are one process
(ADR-0002); the split is logical.

---

## 2. Crates

| Crate | Owns | Depends on |
|---|---|---|
| `shared-kernel` | Error type, caller identity | — |
| `sonari-config` | `sonari.toml`: parsing, validation, watching | `providers` |
| `providers` | VAD on sherpa-onnx; ElevenLabs recognition and synthesis | `voice` |
| `voice` | The provider traits and the runtime the call path speaks to | `shared-kernel` |
| `agent` | Prompt assembly, conversation history, the streaming model client | `shared-kernel` |
| `call/rtc` | LiveKit rooms, tokens, track binding, PCM in/out | `shared-kernel` |
| `call/speech-runtime` | Per-session speech state, rounds, endpointing policy | `voice`, `agent` |
| `call/worker` | The media plane: pipeline, mixer, playback | `call/*`, `voice` |
| `call/control`, `call/execution` | Call lifecycle, dispatch, persistence | `shared-kernel` |
| `platform/postgres` | Schema, migrations | — |
| `api`, `app` | HTTP routes; composition root | all |
| `harness` | Drives one turn from a WAV file, reports the cost | `providers`, `agent` |

No credential appears in any provider trait signature. Adapters take their key
at construction, from the environment.

---

## 3. A turn

```
speech onset  → frames stream to recognition
speech offset → hangover timer
endpoint      → commit the utterance, take the transcript
                model streams tokens → synthesis → playback
barge-in      → playback stops, pending synthesis cancelled
```

**Endpointing is ours** (ADR-0016). The recogniser is opened with
`commit_strategy=manual` and told when an utterance ended; it is not asked. The
same voice activity signal drives interruption, so both ends of a turn come from
one place.

Session state lives in memory for the call's duration. Only completed facts
reach the database, dispatched off the session task — a database outage degrades
record-keeping, not conversation.

The ingress channel is bounded. Under load frames are dropped at ingress with a
counter incremented, never buffered without limit. A commit is never dropped:
losing a frame costs a word, losing the commit means the turn never ends.

---

## 4. Configuration

`sonari.toml` carries what an operator edits: personas and their scenes, the
prompts wrapped around them, which models to ask for, and the endpointing
parameters. It is watched — a change is parsed and validated, and only a valid
result replaces the live one. An invalid file at startup is fatal.

A session resolves its persona once at call start and holds that snapshot, so
editing a persona affects new calls only.

The environment carries only what must not be in a file: API keys, the database
DSN, and where LiveKit is.

**No configuration lives in the database.** `docker compose up` on a clean clone
is sufficient to hold a conversation.

---

## 5. Identity

There is no login. A `uid` is a human-typeable string; creating a session with
one returns a token. The identity is derived from the `uid` rather than
allocated, so the same `uid` reaches the same history on any device without a
user table.

This identifies, it does not authenticate. Adding real authentication later is a
new layer in front, not a change to the client contract.

---

## 6. Data

| Table | Contents |
|---|---|
| `call_sessions`, `call_events`, `call_event_outbox` | One row per call; events |
| `llm_sessions`, `llm_messages`, `llm_usage_logs` | Conversation history and usage |
| `app_error_*` | Recorded failures |

pgvector extends this schema when long-term memory lands; the image is chosen to
allow it without replacement.

---

## 7. Observability

Observability is for a coding agent, not a dashboard (ADR-0017). Every event is
one structured JSON line carrying `session_id`, and `turn` where it applies.

Eight latency markers per turn — `speech_start`, `speech_last_voiced`,
`speech_end`, `asr_final`, `llm_first_token`, `llm_first_sentence`,
`tts_first_chunk`, `audio_first_frame` — carry elapsed values as explicit fields
rather than timestamps to subtract. Two figures are always reported together:

- **System response** — `speech_end` → `audio_first_frame`
- **Perceived latency** — `speech_last_voiced` → `audio_first_frame`

No latency figure enters any document until it has been measured, and
measurements come from release builds.

---

## 8. Failure handling

| Failure | Effect | Recovery |
|---|---|---|
| Recognition socket fails | The turn fails and says so | Session continues; the failure is returned rather than looking like silence |
| Synthesis fails | That reply is not spoken | Turn marked degraded |
| Model endpoint unreachable | Turn fails | Spoken error notice; session continues |
| LiveKit connection lost | Session ends | Client reconnects and starts a new session |
| PostgreSQL unreachable | Facts not persisted | Calls continue; persistence never blocks audio |

---

## 9. Extension points

**Another recognition or synthesis provider.** Implement the trait in
`providers`, construct it in the composition root. No change to the pipeline.

**Another model.** One environment variable — any OpenAI-compatible endpoint
(ADR-0006).

**Changing transport.** LiveKit-specific code is confined to `call/rtc`. The
harness already substitutes a file for a client.

---

## 10. Building

The full binary links only on Linux. `libwebrtc` and the speech runtime disagree
about the C runtime on Windows, and ship separate copies of protobuf that
collide in some link units. Provider-level tests run natively; anything linking
the whole application goes through `scripts/dev.sh`.
