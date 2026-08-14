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

## What the logs pin down

Timestamps from one call, and the whole log holds no other warning or error:

```
19:35:10.614  recognition task started
19:35:11.721  worker push speech frame failed  error="recognition session is closed"
19:35:11.721  … fifteen more, all in the same millisecond
```

Three facts follow.

- **The recognition WebSocket never opened.** `recognition session open` is
  logged the moment the handshake completes and never appears — not once in the
  entire log.
- **The task did not fail; it was cancelled.** Every failure path in
  `run_socket` returns an error that the spawning task logs as
  `recognition session failed`, and a server close logs its reason. Neither
  appears. A cancelled task logs nothing, and the only thing that cancels these
  is `abort_turn_tasks`, on the session-close path.
- **Frames had been queuing.** Sixteen failures land in the same millisecond,
  which is a backlog draining against an already-closed channel rather than
  frames failing as they arrive.

So the session is closed about a second into the call, taking the
half-open recognition socket with it.

**Not the network.** From the same compose network, `api.elevenlabs.io` answers
in 150 ms.

## Narrowed to one difference: the service image

Instrumenting the open path (`opening recognition session` before the handshake,
`recognition session open` after, and a line on every task exit) shows this, for
a whole run:

```
1 × opening recognition session
1 × recognition task started
0 × recognition session open      ← the handshake never completes
0 × recognition session ended     ← the task did not return cleanly
0 × recognition session failed    ← and it did not error
1 × speech session failed
```

Neither exit line appears, so **the task is cancelled while still connecting** —
which is why the failure left no trace before this logging existed.

The same code, key and network succeed from a different container:

```
$ docker compose --profile dev run --rm dev ./target/release/sonari-eval evals/clips/baseline-question.wav
"transcript":"What time do you close on Sundays?"
```

And the service container itself reaches the host:

```
$ docker compose exec sonari openssl s_client -connect api.elevenlabs.io:443 -brief
CONNECTION ESTABLISHED   TLSv1.3   CN = elevenlabs.io
```

So it is not the network, not DNS, not the key, not the request, and not missing
CA certificates — the runtime image installs them. What differs is the image
itself: `debian:bookworm-slim` with `ca-certificates`, `libstdc++6` and
`libglib2.0-0`, against the full Rust image where the same binary works.

**Next step:** trace the runtime's ASR open path layer by layer in the service
image — which timeout fires, and which layer discards the stream handle — since
the cancellation currently hides whatever the connect would have reported.

## The two questions

1. **What closes the speech session a second in?** Nothing in the harness does
   — it stays on the line for twenty seconds and calls `end_call` only
   afterwards. `mark_session_closing` and `fail_speech_session` are the two
   paths (`crates/call/speech-runtime/application/mod.rs:974`, `:2307`); which
   one fires, and why, is the thread to pull. Neither logs at INFO today, so the
   first step is to make them.
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
