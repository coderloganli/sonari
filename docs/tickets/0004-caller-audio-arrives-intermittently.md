# 0004 — The caller's audio reaches the policy only sometimes

**Status**: Open · **Found by**: the evaluation harness, 2026-08-15 · **Area**: `call/worker`, `call/rtc`

## What is seen

The segmentation policy now logs the loudest frame it saw each second. On a call
driven by the built-in probe with `evals/clips/baseline-question.wav`:

```
peak_mean_abs 0    threshold 300
peak_mean_abs 9    threshold 300
peak_mean_abs 0    threshold 300
```

The clip's own frames peak at **16823** and hold around **9000** through the
speech, measured directly from the file. So the audio reaching the policy is not
quiet — it is absent.

On those calls the timeline ends at `runtime_playback_completed`: no
`worker_barge_in_detected`, no `speech_detected`, nothing until the caller hangs
up.

**It is not always absent.** An earlier call, same clip, same build, produced
`speech_asr_final_received "I'd like a table-"`. Delivery works sometimes.

## Why it matters

It blocks ticket 0001. Whether `voice_activity_threshold` is set well cannot be
judged while the audio it judges arrives at random.

## Where to look

- Whether the probe's published track is being subscribed before it starts
  sending, and what happens to frames sent in between.
- `AudioPreprocessor` — the policy sees whatever it emits, and a preprocessor
  that suppressed everything would look exactly like this.
- Whether the frames the policy inspects are the same buffer that reaches
  recognition; recognition has produced real transcripts on calls where the
  policy saw silence, which would be impossible if they were the same audio.

That last point is the sharpest lead: the two disagreeing about the same call
means they are not looking at the same samples.

## Reproducing

```bash
docker compose up -d
scripts/dev.sh cargo run --release -p probe -- evals/clips/baseline-question.wav
```

with `RUST_LOG=speech_runtime=debug` and `SONARI_URL` / `SONARI_LIVEKIT_URL` set.
Watch `peak_mean_abs` in the service log against the clip's own levels.
