# Optimisation Log

What the harness measured, what changed as a result, and what was rejected.
Newest first.

This lives beside the harness because the harness produced it. A change with no
measurement does not belong here, and a measurement without the conditions it
was taken under is not a measurement.

## Conditions

Unless an entry says otherwise:

| | |
|---|---|
| Machine | Ryzen 7 7800X3D (8 cores), 32 GB |
| ASR | ElevenLabs Scribe v2 Realtime, WebSocket |
| TTS | ElevenLabs, PCM 16 kHz requested directly |
| LLM | xAI `grok-4.20-0309-non-reasoning` |
| Build | **release**. A debug build inflated one stage by half again |
| Samples | One turn per figure unless stated. Percentiles wait for a golden set |

Everything below the line marked *local inference* was measured against models
running in-process, before ADR-0014 moved them to hosted providers. Those figures
describe a system that no longer exists; they are kept because the reasoning that
produced them still applies.

**System response** is `speech_end` → first audio out. That is the figure the sub-2s target is set against.

---

## The first figures published — 854 ms answered, 1553 ms waited

The first run of the whole evaluation set against the running service, over
LiveKit, with a probe joining the room as the caller. These are the figures
`README.md` and `docs/architecture.md` now carry; before this entry neither
document held a number.

Run: `evals/runs-live/2026-08-15T19-28-00.118098504+00-00.json`.

| | p50 | p95 |
|---|---|---|
| **System response** — `speech_end` → first audio frame | **854 ms** | **976 ms** |
| **Perceived latency** — `speech_last_voiced` → first audio frame | **1553 ms** | **1677 ms** |

Both are under the two-second target, and the gap between them is the
endpointing hangover: 700 ms of silence has to pass before a turn is called
finished, and the caller waits through all of it. That is the largest single
cost in what a caller experiences, and it is a policy value, not a slow
component.

Recognition quality over the same run: corpus WER 4.4%, p50 0%, p90 18%. At this
set size the confidence interval is roughly ±5-10 points absolute — a regression
tripwire and a category-failure detector, not an instrument for ranking systems a
point apart.

**What these figures cannot claim.** Read them with all of this:

- **One epoch.** Each clip was run once, so p95 is an interpolation near the
  worst sample rather than a tail. The set is 16 clips; percentiles over 48
  samples would mean considerably more.
- **15 clips, not 16.** `idle-force-agent` was added to the set after this run.
- **The build is not recorded.** This run predates the `build` field, so the file
  cannot say it came from a release build, and every figure in this repository is
  supposed to be a release figure. The command used carried `--release`, but the
  file is the evidence and the file is silent.
- **14 of 15 samples succeeded.** `edge-8khz-stereo` failed, and by design: the
  clip is 8 kHz stereo and the pipeline carries 16 kHz mono, so it was rejected
  before it reached the service. The percentiles are over the 14. Two clips —
  `edge-silence` and `edge-cough` — opened no turn, which is the outcome they
  test for, and there were no false triggers.

These stand until the set is re-run at three epochs from a build that says so,
which is scheduled after the features still to be built land.

---

## First measured turn on hosted inference — 1325 ms

The whole path, one recording through the harness:

| Marker | Elapsed from speech end |
|---|---|
| Recognition final | 120 ms |
| First token | 589 ms |
| First sentence complete | 656 ms |
| Whole reply complete | 833 ms |
| First audio out | **1325 ms** |

Under the two-second target. Recognition is no longer free — it was 0.1 ms when
decoding happened in-process, and is now a network round trip plus the
provider's own latency.

Against the same recording on local models: **690 ms → 1325 ms**, and the
transcript went from `BRAFFLEL` to `brothels`. That is the trade ADR-0014 made,
now with numbers on both sides.

The gap between first sentence and whole reply is **177 ms** — what sentence
segmentation could save. Still not worth the prosody it costs.

---

# Local inference

Everything below was measured before ADR-0014. The system it describes is gone.

---

## Constrain reply length — 1213 ms → 690 ms

The `[prompts]` section did not exist and the prompt template table was empty, so the model received no system prompt at all. It answered as a general assistant: six sentences, **22 seconds of synthesised speech** for one utterance.

Adding a `conversation_system` prompt that says this is a phone call, replies are one or two sentences, and no lists or emoji:

| | Before | After |
|---|---|---|
| System response | 1213 ms | **690 ms** |
| LLM generation complete | 1007 ms | 527 ms |
| Synthesised audio | 22.0 s | 5.1 s |

**−43%, and no code changed.** Generation time scales with output length, so the reply that is too long to listen to is also the reply that is slow.

Cost: none. A shorter reply is better in both directions here.

The failure this uncovered is worth more than the figure: a missing prompt template resolved to an empty string and the conversation worked. It answered, fluently, as nobody in particular. Configuration now refuses to start with a persona and no `conversation_system`.

---

## Sentence segmentation — rejected

Splitting the token stream into sentences so synthesis starts on the first one rather than the last token.

| Reply length | Available saving |
|---|---|
| Six sentences (before the prompt fix) | 458 ms |
| Two sentences (after) | **50 ms** |

Rejected. The cost is prosody — each sentence is synthesised independently and intonation across the boundary is lost — and after replies were shortened the saving fell by nine tenths.

Note that synthesis already streams: given a complete reply, sherpa-onnx splits it and emits audio per batch. Splitting ourselves only buys starting *before the model has finished*, which is exactly what shortening the reply made cheap.

**Revisit if** replies grow, or a slower model widens the gap between first sentence and last token.

---

## Resample synthesis output — ~2 ms

Piper produces 22.05 kHz; the pipeline carries 16 kHz. 22.05 kHz is not a rate Opus encodes natively, so declining to resample only moves the work into libwebrtc where it cannot be measured.

Polyphase FIR, 441:320, 64 taps per phase, cutoff at 7.2 kHz.

| Build | First chunk without | First chunk with |
|---|---|---|
| Release | 81 ms | **83 ms** |
| Debug | 81 ms | 121 ms |

The debug figure is a build-mode artefact, not a cost. It is recorded because it nearly sent the estimate wrong by a factor of twenty — **latency figures come from release builds only.**

---

## Warm the models at startup — 1.24 s on the first turn

ONNX initialises lazily. The first synthesis after loading took **1.24 s**; every one after it took **81 ms**.

Without a warm-up the first caller of a fresh process pays over a second that nobody else does — invisible in any figure averaged over a session, and exactly the caller most likely to be a first-time user.

The eval harness spends it before the clock starts. The service must do the same.

---

## Structural choices that cost nothing

Not measured individually; recorded because they are load-bearing and easy to undo by accident.

- **`OnlineRecognizer` needs no mutex.** It is `Sync`. Wrapping it would serialise decoding across every concurrent call and cap throughput at one session's worth.
- **`max_num_sentences = 1`** in synthesis. It sets the callback granularity; the default is larger and delays the first chunk.
- **Recognition's endpoint is free.** Streaming ASR decodes while frames arrive, so `asr_final` measured 0.1 ms after speech end. ASR is not on the critical path; generation is.
- **Inference runs off the async runtime.** ONNX is blocking CPU work; inline it would stall unrelated sessions rather than itself.
