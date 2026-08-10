# worker

`worker` is the process-level runtime host above `call` and `rtc`.

It now lives under the `call` domain family at:

```text
call/worker/
```

Current responsibilities:
- poll internal runtime work
- start and stop a single-call runtime
- bind user audio input from `rtc`
- create bot audio output through `rtc`
- stream input audio frames to backend speech sessions and consume output events
- start TTS playback when the first streamed `AudioChunk` arrives, without waiting for `ReplyFinished`
- track local output state after a live output turn starts and before local bot playback begins
- gate barge-in during bot playback locally so only confirmed near-end speech interrupts bot audio
- keep streaming input during bot playback with worker media-state metadata; backend speech-runtime decides whether to drop or consume the frame
- report `ready` / `failed` / `stopped` back to the backend

Local output observation states:
- `Open`: preprocessed microphone frames are streamed to backend speech sessions.
- `OutputTurnPending`: a speech-runtime live output turn has started, but no local bot audio frame has been written yet; microphone frames are streamed with `output_turn_pending` and cannot trigger barge-in.
- `BotPlayback`: local bot audio is playing; microphone frames are streamed with `bot_playback` while the worker locally evaluates barge-in.
- `Closed`: terminal or closing session; microphone frames are dropped.

Internal structure:
- `client.rs`: internal runtime API client
- `config.rs`: worker process config
- `input.rs`: streaming user audio input handling
- `pipeline.rs`: speech session orchestration shell and remote speech handler
- `playback.rs`: bot audio playback helper
- `runtime.rs`: single-session runtime lifecycle
- `worker.rs`: top-level polling loop and work dispatch

Current limitation:
- the speech handler now depends on backend internal speech APIs, so provider/runtime failures will surface through backend responses rather than local module calls

## Technical Debt

- `worker` now depends on a backend internal speech-turn API for `ASR -> agent -> TTS`.
  - Why changed: this preserves clean service boundaries and lets `worker` remain a real process-layer crate without direct module or database dependencies.
  - Impact: backend now sits in the middle of runtime speech execution, and PCM payloads cross the process boundary.
  - Follow-up: if runtime throughput becomes a bottleneck, optimize the streaming speech-session contract rather than moving business modules into `worker`.

- `worker` no longer receives `llm_session_id` through runtime work polling.
  - Why changed: worker startup should only depend on runtime launch/control data; agent session context belongs behind backend speech orchestration.
  - Impact: `/internal/runtime/poll` is thinner, and worker startup is no longer blocked on an agent-specific field in the work contract.
  - Follow-up: keep worker-facing work items focused on runtime ownership and launch only; resolve any future speech-turn context through backend orchestration ports.

- Turn segmentation is now delegated to backend `speech-runtime` sessions instead of fixed worker-side chunks.
  - Why changed: worker should stream audio transport, not reinterpret arbitrary frame windows as complete turns.
  - Impact: segmentation and flush semantics now live behind backend speech-session APIs, so runtime behavior depends on backend session-state policy.
  - Follow-up: keep refining backend speech-session semantics without reintroducing local chunk-based turn logic in `worker`.

- Worker no longer owns or forwards ASR input language.
  - Why changed: speech input language is part of backend runtime session context, not worker process configuration.
  - Impact: create-session DTOs are thinner and worker startup no longer carries speech-language policy.
  - Follow-up: keep speech-language selection behind execution/runtime context instead of reintroducing worker-side config knobs.

- Stop work reports a distinct `missing` runtime fact when the local runtime is already absent.
  - Why changed: a missing local runtime is neither a real stop success nor a definitive business failure; `call-control` owns the durable lifecycle and decides how that fact converges.
  - Impact: worker now emits `worker_runtime_missing` and reports runtime status `missing`, letting the control plane resolve `Ending -> Stopped` and `Active/Starting -> Failed` from durable state.
  - Follow-up: if later you need richer reclaim/restart observability, extend execution facts rather than collapsing them back into `failed` or `stopped`.

## Remaining Product Gaps

- `worker` now has a formal preprocessing boundary, playback interruption path, and bot-output mixer, but the current preprocessing implementation is still a simple noise gate instead of a stronger production-grade denoise/noise-suppression chain.
- `worker` now restarts the backend speech session when barge-in is detected so interrupted speech output is suppressed, and interruption/external-audio events now carry stable round IDs, but runtime-start/stop and other media-side events still have coarser grouping than speech-turn events.
- `worker` now supports external audio playback mixed with TTS into the single LiveKit bot track, but the current external-audio source path assumes fetchable WAV assets and should be broadened only if product requirements expand beyond that format.
- These are product requirements for the LiveKit call path, not optional follow-ups.
