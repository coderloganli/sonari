# speech-runtime

`speech-runtime` owns runtime speech-session orchestration.

It now lives under the `call` domain family at:

```text
call/speech-runtime/
```

Responsibilities:
- validate runtime speech-session context through `call-execution`
- accept streaming input audio for a runtime speech session
- accept normal runtime input at any live phase and explicitly drop frames that should not enter ASR
- drive an explicit speech-session phase machine
- evaluate input frames through an injected segmentation policy
- segment utterances inside the backend speech session
- run `ASR -> agent -> TTS`
- emit streaming output events back to the caller
- own its module-specific Redis/Postgres adapters for runtime state and segmentation config
- keep speech-session state writes behind a per-session single-writer boundary so stale input/ASR progress cannot revive a completed live output turn
- preserve terminal close-path events before removing successful sessions from the runtime store

Input admission:
- `open` worker media state enters segmentation / ASR unless the durable session is already in a live output turn.
- `output_turn_pending` and `bot_playback` worker media states are accepted with 2xx and dropped before ASR.
- live output turn races are accepted with 2xx and dropped without overwriting `phase`, `active_turn`, or dispatched round state.
- terminal, owner-mismatch, and invalid protocol input remain explicit errors.

State ownership:
- `phase`, `active_turn`, and `dispatched_round_ids` are live output lifecycle fields.
- input/ASR progress can advance input-owned phases, pending rounds, and utterance buffers, but cannot resurrect a stale `responding` phase after the live output turn has completed.
- Redis compare-save is a persistence guard, not the lifecycle owner; application state changes must pass the per-session single-writer boundary first.

Non-responsibilities:
- call lifecycle control
- worker polling/status
- rtc/media execution
- provider config management

Current collaborators:
- resolves runtime turn context through a module-owned `SpeechRuntimeContextPort` adapter backed by the shared execution-owned `call-runtime-context` contract
- stores runtime speech-session state through `SpeechSessionStorePort`
- persists the current live TTS turn in speech-session state so owner-process loss is detected and surfaced as `SessionFailed`
- reads formal segmentation policy config through `SpeechSegmentationConfigPort`
  backed by a keyed `speech_runtime_configs` record instead of a magic singleton row
- uses `voice`'s narrow runtime execution port for ASR/TTS
- uses a module-owned `AgentTurnPort` adapter for LLM turns

Current speech-session phases:
- `listening`
- `speech_detected`
- `flushing`
- `responding`
- `closing`
- `failed`

## Technical Debt

- `speech-runtime` validates runtime ownership and execution readiness through a module-owned `SpeechRuntimeContextPort`.
  - Why kept: runtime turn execution must reject stale or cross-owned work, while `call-execution` remains the execution-side source of truth.
  - Impact: `speech-runtime` application depends on its own narrowed context port, and its adapter consumes the shared execution-owned `call-runtime-context` boundary contract instead of duplicating models or importing `call-execution` directly.
  - Follow-up: if runtime turn execution grows richer than owner/readiness/language checks, evolve the shared execution-owned context rather than widening worker request DTOs or reintroducing `call` dependencies.

- `speech-runtime` now resolves an opaque `agent_session_id` from `RuntimeSessionContextPort` instead of trusting the worker request payload.
  - Why changed: turn orchestration should integrate with `agent` session context, not with an LLM-named field exposed across process boundaries.
  - Impact: speech-session execution is less coupled to `call` runtime work items and no longer depends on worker-visible LLM session semantics.
  - Follow-up: if runtime speech sessions eventually need richer context than owner/status/session, introduce a dedicated speech-session context model rather than widening worker request DTOs.

- `speech-runtime` now consumes `voice`'s narrow runtime execution port instead of reaching into `voice` repositories/config.
  - Why changed: `voice` owns supplier/config/route ownership, while `speech-runtime` owns turn orchestration.
  - Impact: runtime ASR/TTS execution stays narrow at the module boundary without leaking supplier/config ownership into `speech-runtime`.
  - Follow-up: evolve the runtime request/response shape at the `voice` runtime port rather than reintroducing repo-level coupling.

- `speech-runtime` application now talks to `agent` through a module-owned `AgentTurnPort`.
  - Why changed: application code should depend on module-owned ports, not directly on another module's application DTOs/use cases.
  - Impact: `speech-runtime` no longer imports `agent::ChatCommand` or `agent::AgentRuntimeUseCases` in its orchestration layer.
  - Follow-up: keep future turn-level agent integration behind this port instead of leaking agent application types back into `speech-runtime`.

## Remaining Product Gaps

- Worker-driven interruption now suppresses the interrupted speech session by rotating the runtime speech session, but richer interruption-aware partial-turn policy and observability still need refinement.
- Runtime speech-stage logs are now emitted for queue aggregation with stable speech round IDs, but operator-facing timelines still need richer cross-cut media correlation beyond speech-turn boundaries.
- These are part of the required LiveKit call experience and logging product goals, not optional cleanup.
