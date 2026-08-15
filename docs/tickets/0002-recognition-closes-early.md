# 0002 — Recognition never opened: two rustls providers in one binary

**Status**: Fixed 2026-08-15 · **Found by**: the evaluation harness · **Area**: `app`, TLS

## The cause

The recognition WebSocket never completed its handshake inside the service, and
the failure left no trace in any structured log. It was a panic:

```
thread 'tokio-rt-worker' panicked at rustls-0.23.38/src/crypto/mod.rs:249:
Could not automatically determine the process-level CryptoProvider from Rustls
crate features. Call CryptoProvider::install_default() before this point, or
make sure exactly one of the 'aws-lc-rs' and 'ring' features is enabled.
```

Both providers reach this binary — different dependencies pull different rustls
backends — so rustls refuses to choose and panics on the first handshake. The
panic happened inside a spawned task, so it went to stderr and never through
tracing; every structured log simply showed recognition ceasing to exist.

Everything downstream followed: frames dropped, no speech detected, the session
failed, and the next call got no runtime.

## The fix

`crates/app/src/main.rs` installs the `ring` provider once at startup, which is
what the panic message asks for. Feature unification across a workspace this size
is too fragile to rely on instead.

After it: `recognition session open` in 146 ms, speech detected, an utterance
flushed, and a transcript returned.

## Why it took so long to see

The adapter logged when its task started and when it failed, but nothing while
connecting and nothing on a clean return — so a task that panicked mid-handshake
was indistinguishable from one still working. Three lines now cover the open
attempt, the elapsed time on success, and a clean exit, and `fail_speech_session`
warns rather than only recording a call event.

## What remains

Two behaviours are still open and are not this bug:

- `speech_session_closing` arrives while a turn is still in flight, so no
  `speech_turn_latency` is emitted and the transcript is truncated —
  `"I'd like a table-"` against a reference of "i'd like a table for four people".
- After one call, the next gets no runtime: the bot never joins its room.
