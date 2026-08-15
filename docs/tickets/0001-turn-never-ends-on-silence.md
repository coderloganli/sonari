# 0001 — A turn appears never to end on trailing silence

**Status**: Fix written 2026-08-15; live verification blocked by ticket 0004

> **2026-08-15 — confirmed by measurement.** With tickets 0002 and 0003 fixed,
> a live call now shows exactly what the reasoning below predicts:
>
> ```
> + 3.11s  speech_detected
>          … twenty seconds of nothing …
> +22.89s  call_end_requested          ← the caller hangs up
> +23.03s  speech_session_closing
> +23.03s  speech_utterance_flushing   ← same millisecond as the close
> +23.14s  speech_asr_final_received   transcript: ""
> ```
>
> The clip is 4.2 s long and `silence_flush_ms` is 700 ms, so the utterance
> should have been committed around six seconds in. It never was. The one flush
> that does appear comes from the close path, not from silence — which is also
> what an earlier run showed, and what briefly looked like evidence against this
> ticket. · **Found by**: the evaluation harness, 2026-08-14 · **Area**: endpointing (ADR-0016)

## What was observed

Driving the running stack with the evaluation harness (`sonari-eval run
evals/set.jsonl --live`), every call reached recognition and then stopped there:

- the caller joins, publishes audio, and the bot joins and subscribes;
- the service logs `recognition task started` and audio frames arrive;
- **no call produced a `speech_turn_latency` event**, because no turn completed;
- the caller stayed on the line for 20 s after the audio ended — far longer than
  `silence_flush_ms` (700 ms) or `silence_force_agent_ms` (8 s).

## The likely cause

`ConfigEndpointing` hardcodes the amplitude threshold to zero
(`crates/app/src/endpointing.rs:36`):

```rust
// Compares PCM amplitude directly, which means nothing against a neural
// detector's speech probability. The field survives because the policy still
// reads it; replacing that policy is what makes it go away.
voice_activity_threshold: 0,
```

The policy tests voice activity as the mean absolute sample value against that
threshold (`crates/call/speech-runtime/application/segmentation.rs:110`):

```rust
let avg = pcm_s16le.iter().map(|s| i32::from(s.abs())).sum::<i32>() / pcm_s16le.len() as i32;
avg >= i32::from(threshold)
```

`avg` is a mean of absolute values, so it is never negative, so with a threshold
of `0` **every non-empty frame counts as speech**. Silence is therefore never
observed, `SpeechSegmentationDecision::FlushUtterance` is unreachable, and
`silence_flush_ms` has no effect. `silence_force_agent_ms` depends on the same
signal and appears equally unreachable.

Static reading and the live run agree. What actually ends a turn in production —
the caller hanging up, a provider-side commit, something else — has not been
traced.

## Why it matters

Endpointing is the one part of the turn sonari implements itself (ADR-0016): the
recogniser is opened with `commit_strategy=manual` precisely so the decision
stays local. If the decision never fires, the four endpointing parameters an
operator tunes in `sonari.toml` are inert, and the comment beside them — "tuned
by ear against real calls, changing one is an experiment" — describes an
experiment that cannot currently have an effect.

## What would settle it

1. Trace what closes a turn today on a real call that completes.
2. Decide what the threshold should be, or replace the amplitude policy with the
   Silero detector already vendored in `providers` — the comment above suggests
   the amplitude comparison was always meant to be temporary.
3. If the decision changes, ADR-0016's implementation changes with it, so a
   superseding record belongs in the same change.

## Reproducing

```bash
docker compose up -d
scripts/dev.sh cargo run --release -p harness --features live -- \
    run evals/set.jsonl --live
```

Needs `SONARI_BASE_URL`, `SONARI_LIVEKIT_URL`, `SONARI_CHARACTER_ID` and
`SONARI_ADMIN_TOKEN`; see `.claude/new-task.md`. The failure is every row
reporting `the call produced no speech_turn_latency event`.

## The fix

`voice_activity_threshold` is now a setting rather than a hardcoded zero
(`[endpointing]` in `sonari.toml`), defaulting to 300 — comfortably above a
−60 dBFS noise floor, which lands near 33 on this scale, and far below the
thousands that measured speech reaches.

Four tests pin the behaviour down, including the one that would have caught this
at the time: at a threshold of zero even digital silence counts as voice, because
the comparison is against a mean of absolute values and that is never negative.

**Not yet verified end to end.** The Docker daemon went down before a live run
could confirm that a turn now ends on silence. The proof to look for is a
`speech_utterance_flushing` roughly `silence_flush_ms` after the caller stops,
rather than one that arrives with the hang-up.

Whether an amplitude comparison is the right instrument remains open. The Silero
detector is already vendored and used nowhere in the call path; moving to it
would change how ADR-0016 is implemented and wants its own decision.

## Live verification, and what it ran into

With the threshold at 300 no speech is detected on a live call at all. That is
not the threshold being wrong: instrumenting the comparison shows the loudest
frame reaching it in any second is **9**, against a clip whose frames peak at
**16823** and sit around 9000 through the speech. The caller's audio is not
arriving at the segmentation policy.

It arrives sometimes — an earlier run transcribed `"I'd like a table-"` from the
same clip — so delivery is intermittent, and that is ticket 0004. Until it is
reliable, whether 300 is a good value cannot be answered.

The comparison now logs the loudest frame it saw each second, which is what made
this visible; sampling a frame at random reports silence during speech often
enough to mislead.
