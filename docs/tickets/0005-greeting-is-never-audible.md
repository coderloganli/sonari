# 0005 — The caller may never hear the greeting

**Status**: Open · **Found by**: the evaluation harness, 2026-08-15 · **Area**: `call/worker` playback, or the client's audio stream

## What is seen

The probe now logs the loudest frame on the agent's track each second. Across a
call whose service-side timeline shows a greeting synthesised and played:

```
the agent never greeted   loudest=0   threshold=100     ← ten seconds of nothing
agent track level  second=3  peak=0
agent track level  second=4  peak=12550                 ← after the caller spoke
agent track level  second=5  peak=16486
agent track level  second=6  peak=1139
```

The reply is plainly audible. The greeting, which the service logs as
`speech_tts_started`, `speech_tts_first_chunk_received`, `speech_reply_finished`
and `runtime_playback_completed` around 1.5–3.6 s into the call, arrives as
zeros. Two runs agree.

## Two explanations, not yet separated

1. **The greeting is never written to the room.** A caller would pick up, hear
   silence, say something, and only then hear the agent — which is a real defect
   and the more serious reading.
2. **The client's audio stream does not deliver for its first few seconds.** The
   first per-second line is `second=3`, so frames may simply not have been
   flowing while the greeting played, and the greeting was fine.

Telling them apart needs one of: a second subscriber joining late and listening
for a later server-initiated turn; recording the room from LiveKit's side; or
instrumenting the worker's playback path to log what it actually handed to the
mixer and when.

## Why it matters either way

If it is (1), the demo's first impression is silence. If it is (2), every client
in this repository under-reports the beginning of a call, and both the probe and
the eval harness are waiting on a clock for a greeting they were never going to
hear.

## Reproducing

```bash
docker compose up -d
RUST_LOG=sonari_probe=debug scripts/dev.sh cargo run --release -p probe --     evals/clips/baseline-question.wav
```

Compare `agent track level` against the same call's timeline at
`/api/admin/call-logs/{session_id}/timeline`.
