# call execution

`call-execution` is the execution-control crate inside the `call` domain family.

Responsibilities:
- own worker-facing `poll/status` contracts
- own runtime launch artifact preparation and DTOs
- translate thin control-plane work descriptors from `call control` into worker-consumable start/stop work items
- expose execution-facing runtime speech context without leaking `call-control` state enums
- consume control-owned runtime-session state and reduce it into execution-facing runtime context
- emit execution-stage call events into the shared call-event pipeline

Non-responsibilities:
- `StartCall` / `EndCall`
- the primary `call_session` business state machine
- turn-level `ASR -> agent -> TTS` orchestration
- speech-turn session context modeling
- operator-facing log aggregation

## Technical Debt

- `call-execution` still uses database claim/update semantics instead of a dedicated queue.
  - Why kept: current work is focused on boundary cleanup and deterministic control flow without adding new dispatch infrastructure.
  - Impact: execution concurrency and recovery still depend on shared persisted state plus polling semantics.
  - Follow-up: if call concurrency grows, evaluate moving dispatch signaling from database polling to a dedicated queue.

- Start work is claimed atomically before launch preparation.
  - Why kept: atomic claim is required to prevent multiple workers from preparing launch artifacts for the same pending session.
  - Impact: launch preparation runs only after one worker owns the start work; any post-claim failure must converge immediately into a control-owned terminal fact instead of leaving `StartClaimed` / `StopClaimed` rows stranded.
  - Follow-up: if launch preparation cost becomes significant, replace database polling/claim semantics with a dedicated dispatch substrate instead of reintroducing non-atomic peek-before-claim flow.

- `call-execution` exposes a reduced runtime speech-readiness model instead of forwarding `call-control` status enums.
  - Why changed: `speech-runtime` should depend on execution-facing readiness, not on `call-control`'s internal state machine.
  - Impact: `call-control` owns the persisted runtime-session facts and exposes them through an owner-side narrow port; `call-execution` only reduces those facts into the shared `call-runtime-context` contract that `speech-runtime` consumes.
  - Follow-up: if speech execution needs richer readiness semantics later, evolve the shared execution-owned contract without leaking control-plane enums or reintroducing persistence ownership into `call-execution`.

- `call-execution` now accepts an explicit `missing` runtime status from workers.
  - Why changed: “local runtime absent” is an execution fact that must not be collapsed into fake `stopped` or unconditional `failed`.
  - Impact: `call-control` resolves that fact against durable lifecycle state, allowing stopping sessions to converge to `stopped` while surfacing unexpected runtime loss as failure.
  - Follow-up: if runtime reclamation semantics get richer later, keep extending explicit execution facts instead of overloading terminal statuses.

- Initial bot speech no longer travels inside the execution/runtime launch contract.
  - Why changed: execution must not carry control-owned bot-speech planning across the worker launch boundary.
  - Impact: runtime launch now contains only neutral startup artifacts; workers fetch initial bot speech from a separate control-owned internal endpoint after runtime startup.
  - Follow-up: if startup sequencing grows richer later, keep it on a dedicated control-owned runtime trigger path instead of reintroducing bot-speech DTOs into execution contracts.
