# call control

`call control` is the business control-plane crate inside the `call` domain family.

## Responsibilities

- own `StartCall`, `EndCall`, and `ListCallHistory`
- own admin-facing call log list/detail/timeline/activity-log read models
- own durable `call_session` business state
- own persisted runtime-session facts derived from `call_sessions`
- own business-level bot speech queue semantics through `call/control/bot-speech`
- own cross-owner server-initiated turn composition through `call/control/orchestration`
- validate character / voiceprint / user-context prerequisites
- create agent sessions and persist the business link through `agent_session_id`
- emit business call events into the call-event pipeline
- expose thin execution descriptors to `call-execution`
- own the LiveKit-only transport choice for realtime calls

## Non-responsibilities

- LiveKit room / track / token details
- worker polling or worker status protocols
- runtime launch artifact preparation
- speech-session orchestration
- media preprocessing, playback, interruption, or mixing
- provider configuration ownership
- latency statistics
- TRTC or legacy WebRTC transport support

## Domain-family topology

```text
call/
  control/
  execution/
  rtc/
  worker/
  speech-runtime/
```

`call control` is the business source of truth. `call-execution`, `rtc`, `worker`, and
`speech-runtime` implement the execution path behind that control plane.

`call/control/orchestration` is the control-owned composition layer for cross-owner server-initiated
turn policy. `call/control/bot-speech` owns queue and playout semantics; orchestration owns how a
specific backend-triggered event becomes a bot-speech item.

## Main flow

### StartCall

1. Read user context, including timezone.
2. Read character context for the explicitly selected scene when provided; otherwise resolve the default active scene.
3. Create the linked agent session.
4. Persist `call_session` in `starting + pending_start`.
5. Emit business events.
6. Return control-plane response data.

### EndCall

1. Read the active session.
2. Persist `ending + stop_requested`.
3. Emit business events.
4. Let `call-execution` and the worker finish shutdown.

## Boundaries

- `call control` owns business semantics only.
- `call control` must not own worker-facing launch/work artifacts.
- `call control` must not import LiveKit-specific types.
- Cross-module adapters owned by `call control` live under `crates/call/control/adapters`.
- `app` only assembles those adapters; it must not implement them.
- `call control` consumes narrow owner-provided ports from adjacent modules:
  - `agent::AgentCallControlPort`
  - `character::CharacterCallContextReadPort`
  - `user-context::UserCallContextReadPort`
  - `voice::VoiceCallConfigUseCases` (single-purpose input-language config facade)

## Observability

`call control` emits business call events through `EventSinkPort`. Those events are published
into Redis Streams, reclaimed from stale pending entries after consumer restarts, consumed into
`call_events`, and queried through the log engine. The consumer path is idempotent on
`stream_message_id` and must surface append/ack failures explicitly. Pending recovery must perform
real Redis claim ownership before append/ack; database idempotency is only a duplicate-write guard.
This crate produces logs; it does not compute latency statistics.

## Current product scope

- LiveKit only
- no TRTC
- no legacy WebRTC signaling path
- no latency-stat feature

## Technical Debt

- `call control` still relies on backend-wide log-engine infrastructure that is assembled in `app`.
  - Why kept: queue consumer startup and shutdown remain process-level concerns.
  - Impact: control-plane event production is module-owned, but runtime operation of the queue
    pipeline still depends on composition-root wiring.
  - Follow-up: keep queue/runtime orchestration in composition and do not move business event
    decisions into `app`.

- Operator-facing call-log coverage is now present, but some worker/media events still have
  coarser grouping than speech-turn events.
  - Why kept: speech-runtime round IDs are richer than current worker runtime lifecycle IDs.
  - Impact: activity timelines are useful today but still denser and more speech-centric than the
    final operator experience.
  - Follow-up: continue improving worker/media event correlation without pushing media ownership
    back into `call control`.
