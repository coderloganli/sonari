# rtc

`rtc` is the execution-plane crate that owns LiveKit-specific implementation details for the
`call` domain family.

## Responsibilities

- LiveKit room connection and teardown
- participant identity and join-token usage
- remote user audio subscription
- bot audio output sink binding
- conversion between LiveKit runtime callbacks and the worker-facing runtime shell

## Non-responsibilities

- call lifecycle business state
- agent-session lifecycle
- speech-turn orchestration
- provider configuration ownership
- denoise / interruption policy ownership
- external audio selection policy

## Product scope

- LiveKit only
- no TRTC
- no legacy WebRTC signaling stack

## Boundary rules

- `call control` must not own LiveKit details from this crate.
- `worker` drives this crate during runtime execution.
- `speech-runtime` must not depend on LiveKit SDK types.
- `app` assembles `rtc`; it does not implement media logic.
