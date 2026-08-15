# 0003 — The media pipeline is torn down two seconds into a call

**Status**: Open · **Found by**: the evaluation harness, 2026-08-15 · **Area**: `call/worker` lifecycle

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

## Where to look

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
