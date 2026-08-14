# 0002 — Recognition closes a second into the call, and the next call gets no runtime

**Status**: Open · **Found by**: the evaluation harness, 2026-08-14 · **Area**: `providers` ASR session, `call/worker`

## What actually happens

The media path is fine. With `RUST_LOG=rtc=debug` the runtime shows the whole
setup succeeding:

```
livekit runtime subscribed remote audio track
livekit user audio input validated remote audio track
livekit runtime published bot audio track
recognition task started
```

About a second later, every frame starts being dropped:

```
worker push speech frame failed; dropping frame and continuing  error="recognition session is closed"
worker push speech frame failed; dropping frame and continuing  error="speech session is terminal"
```

and from then on the log is nothing but that second message. No speech is ever
detected, so no turn opens and the clip reports as having produced nothing.

**The next call in the batch gets no runtime at all** — its timeline holds only
`call_start_requested`, `call_end_requested`, `runtime_stop_requested`, and the
bot never joins its room. After a fresh `docker compose up` the pattern is
reproducible: the first call runs and closes its recognition session, the second
never starts.

## The two questions

1. **Why does the recognition session close?** It closes roughly a second after
   `recognition task started`, before any frame has been accepted. Nothing in
   the ElevenLabs adapter logs a reason at INFO. The session is opened with
   `commit_strategy=manual` (`crates/providers/src/elevenlabs_asr.rs:63`), so the
   provider is not committing on its own; whether it is closing on its own, and
   why, needs adapter-level logging or a captured WebSocket close frame.
2. **Why does one closed session stop the next call?** A terminal speech session
   should not prevent a new call from getting a runtime. Whatever holds — the
   runtime owner claim, the worker's orchestration state — is not released.

The second is arguably the more serious: it turns one bad call into a service
that answers no further calls.

## What this supersedes

This ticket previously read "the bot intermittently never sees the caller's
track". That was wrong, and the error was the harness's: it waited for the bot's
audio track before sending audio, while the runtime subscribes to the caller
first and publishes only afterwards — a deadlock that produced the intermittent
look. Fixed in the harness; the readiness gate now waits for the bot to be
present in the room, which is the real precondition.

## Reproducing

```bash
docker compose down && docker compose up -d
head -2 evals/set.jsonl > evals/probe.jsonl
scripts/dev.sh cargo run --release -p harness --features live -- \
    run evals/probe.jsonl --live
```

with `SONARI_BASE_URL`, `SONARI_LIVEKIT_URL`, `SONARI_CHARACTER_ID` and
`SONARI_ADMIN_TOKEN` set — see `.claude/new-task.md`. Expect the first clip to
run and report no turn, and the second to fail with the bot never joining.

## Relationship to 0001

0001 asks why a turn never *ends*. This is upstream of it: on this stack a turn
never *starts*, because recognition is gone before speech is detected. 0001
cannot be answered from a live run until this is fixed.
