# ADR-0014: Reach recognition, synthesis and the model over the network

- **Status**: Accepted
- **Date**: 2026-08-08
- **Tags**: `providers` `process` `audio` `latency`
- **Related**: ADR-0003, ADR-0004, ADR-0005, ADR-0006

## Context

ADR-0003 forbids any PCM-handling component from sitting across a process
boundary from the audio source, and ADR-0005 satisfies it by linking VAD, ASR and
TTS into the binary through sherpa-onnx. Both were built and measured: a turn ran
end to end at 690 ms system response with a 20M-parameter recogniser and a small
Piper voice.

That recogniser made audible errors — `BROTHELS` came back as `BRAFFLEL` — and it
is the smallest model available. Larger local models exist and were not tried.
The decision to move to hosted inference was taken before that comparison, on the
judgement that the quality of the leading hosted models is worth the cost, and
that the project's engineering interest lies in the pipeline rather than in
running the models.

The predecessor system is direct evidence for what that cost looks like. It
pushed every audio frame over HTTP to its own backend, which then forwarded to a
cloud recogniser, and recorded the result in its own source: *"消除每帧音频
HTTP(卡顿根因)"* — per-frame HTTP was the root cause of stuttering. It
subsequently moved recognition in-process. Two things differ here: that path
crossed two hops rather than one, and it used request-response HTTP per frame
rather than a persistent stream.

## Decision

Recognition and synthesis are reached at ElevenLabs, the model at xAI. Audio
crosses a process boundary in both directions.

Voice activity detection stays in-process. It runs on every frame and drives
interruption; a round trip to decide whether someone is speaking would cost more
than the decision is worth. sherpa-onnx remains for this and nothing else.

Endpointing stays ours. Recognition is told when to commit an utterance rather
than asked when the caller stopped.

## Consequences

**What is gained.** The quality of models nobody has to operate. No GPU to size,
no weights to distribute, no VRAM budget. The machine requirement drops to what
runs a Rust binary and a database.

**What is paid.**

- Every frame crosses the network. Jitter and loss now sit inside the audio path,
  where before they could not reach.
- Recognition latency becomes network latency plus provider latency, replacing a
  local decode that measured 0.1 ms at the endpoint.
- The system stops working without connectivity and without valid keys. There is
  no degraded local mode, because the local implementations are deleted.
- Cost scales with conversation minutes rather than with hardware bought once.
- Audio leaves the deployment. Anyone self-hosting this is sending their callers'
  speech to two third parties, which the README must say plainly.
- Two providers become availability dependencies. An outage at either ends every
  call in progress.

**What is retained.** ADR-0004 still holds: orchestration stays beside the audio,
and the remote components are leaves. The turn state machine, the interruption
path and the playback queue are unchanged by this record. ADR-0006's abstraction
paid for itself here — moving the model from a local server to xAI changed one
environment variable and no code.

**What is reversible.** Little, cheaply. The local implementations are removed
rather than kept behind configuration, so returning to self-hosted inference
means writing them again. That was chosen deliberately over carrying two paths.

## Alternatives considered

**Keep local inference and try larger models first.** Model size is configuration
and the comparison would have cost an afternoon. Rejected: the decision was made
on quality grounds that larger local models were not expected to close, and
holding the project open pending a benchmark had its own cost.

**Hosted synthesis, local recognition.** Synthesis is per-utterance and crosses
the boundary once; recognition crosses it per frame. This would have kept
ADR-0003 intact for the expensive direction. Rejected: it leaves two
infrastructures to maintain for one pipeline.

**Keep the local implementations selectable by configuration.** Rejected
explicitly — see ADR-0015.
