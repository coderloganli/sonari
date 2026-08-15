# 0003 — The media pipeline is torn down two seconds into a call

**Status**: Fixed 2026-08-15 · **Found by**: the evaluation harness, 2026-08-15 · **Area**: `call/worker` lifecycle

## What happens

With recognition working (ticket 0002), a call gets as far as a transcript and
then stops. Event times, measured from the start of the call:

```
+0.78  speech_listening_started
+1.59  speech_reply_started        "Hello, old friend. Good to hear you."
+1.86  worker_barge_in_detected
+1.86  speech_detected
+2.14  speech_utterance_flushing
+2.14  speech_asr_commit_requested
+2.14  speech_session_closing        ← here
+2.33  speech_asr_final_received     "I'd like a table-"
+25.10 call_end_requested            ← the caller only hangs up now
```

The caller is still speaking a 4.2 s clip and does not hang up for another
twenty-three seconds. The transcript is truncated to match: `"I'd like a table-"`
against a reference of "i'd like a table for four people".

**And the next call gets no runtime at all** — its timeline holds only
`call_start_requested`, `call_end_requested`, `runtime_stop_requested`, and the
bot never joins its room. One call ends the service's usefulness until it is
restarted.

## What has been established

`speech_session_closing` comes from `close_stream`, which the pipeline's main
task calls on its way out. So the main task ended at 2.14 s. Instrumenting the
inbound drain shows why it stopped:

```
worker inbound drain stopping: shutdown channel gone
```

`changed()` returning an error means **every `watch::Sender` was dropped** — not
a graceful stop, which sends `true` first and logs differently. The sender lives
in `SpeechPipeline`, inside `ActiveRuntime`, inside the worker's `active` map.

Both of the worker's own failure reports were raised from debug to warning to
catch this, and **neither fires**: the task did not return an error, and it did
not complete unexpectedly. So the pipeline value was dropped by something other
than the stop path or the reap path.

## The cause, and the fix

The media plane is one spawned task whose result nobody read
(`crates/app/src/bootstrap.rs`), and inside it every control-plane error
propagated with `?` straight out of the worker loop. So a single rejected fact —

```
publish runtime event failed (NotFound): speech session not found
```

— ended the task, which dropped the worker, its map of active runtimes, and with
them every `watch::Sender`. That is why the drain saw its shutdown channel
vanish, why the call in progress was torn down mid-turn, and why no later call
was ever claimed: **there was no media plane left**. The process went on
answering `/healthz` throughout.

Three changes:

- The media plane is supervised. If it ever returns, it says so at error level
  and states the consequence, rather than disappearing.
- A control-plane rejection about one session is recorded and skipped instead of
  ending the loop. One call's stale fact is not a reason to stop being able to
  serve calls.
- Queued actions that fail retryably stay queued instead of propagating, which
  was killing the plane on any transient blip.

Two consecutive probe calls now both complete with a spoken reply, and a
fifteen-clip live evaluation runs every clip.

## What this exposed next

With calls surviving, the evaluation set can finally see what it was built for,
and it confirms ticket 0001: speech is detected, and the utterance is then never
flushed until the caller hangs up. See that ticket.

## Where to look (original notes)

- What drops an `ActiveRuntime` without `stop()` — an `insert` replacing an
  existing entry, a map cleared, or a value moved out and discarded.
- Whether the co-located orchestration path (`worker 启用进程内编排`) constructs
  or holds the runtime differently from the two-process path this was extracted
  from. That path is new in sonari (ADR-0002 merged the planes), and both bugs
  found so far have been in code the extraction introduced rather than code
  combrabo had proven.

## Reproducing

```bash
docker compose down && docker compose up -d
scripts/dev.sh cargo run --release -p probe -- evals/clips/baseline-question.wav
```

with `SONARI_URL` and `SONARI_LIVEKIT_URL` set. The built-in probe reproduces it
without the eval harness; watch the service log for
`worker inbound drain stopping: shutdown channel gone`.
