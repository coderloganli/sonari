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
