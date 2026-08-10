# ADR-0016: Decide the end of a turn locally, not at the recogniser

- **Status**: Accepted
- **Date**: 2026-08-08
- **Tags**: `audio` `latency` `providers`
- **Related**: ADR-0014

## Context

Hosted recognition reports its own sentence ends. Using them would remove the
hangover timer and the segmentation policy from this codebase.

Interruption cannot be delegated the same way. It is driven by voice activity
detection, which stays local precisely because waiting a round trip to notice
that someone has started speaking is not viable.

The predecessor system faced the same choice against a different provider and
kept endpointing local: it drove its own segmentation state machine and sent
`input_audio_buffer.commit` when it decided the utterance had ended. Its
parameters carry the marks of production — one of them, `min_speech_confirm_ms`,
exists to stop brief background noise from triggering a turn.

## Decision

Endpointing is decided locally, from the same voice activity signal that drives
interruption. The recogniser is told when to commit an utterance; it is not
asked when the caller stopped.

## Consequences

The start and the end of a turn come from one signal. Had they come from two, a
caller could be interrupted on one authority and cut off on another, and tuning
either would have unpredictable effects on the other.

The provider's own endpoint detection is disabled or ignored. If it turns out to
be substantially better than a hangover timer, that advantage is forgone.

Endpointing parameters remain ours to tune, and remain in configuration where
they can be changed without a deployment.

The neural detector reports a speech probability, while the inherited policy
compares PCM amplitude against `voice_activity_threshold`. That field has no
meaning against the new signal: this is a change of decision input, not a
retuning, and it is the substance of the turn state machine work.
