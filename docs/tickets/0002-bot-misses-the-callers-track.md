# 0002 — The bot intermittently never sees the caller's audio track

**Status**: Open · **Found by**: the evaluation harness, 2026-08-14 · **Area**: `call/rtc`, LiveKit join

## What was observed

Running `sonari-eval run evals/set.jsonl --live` against `docker compose up`,
calls complete but most produce no speech at all. The service logs, per run:

```
livekit runtime timed out waiting for expected remote audio track
  room_name=call-N  participant_identity=bot-N
  expected_remote_participant_identity=platform_user:<id>  timeout_ms=30000
```

and occasionally, for one call in a batch:

```
recognition task started
native audio stream queue overflow; dropped 1 queued frames
```

So the path works sometimes. When it does not, the timeline contains only
`call_start_requested`, `call_end_requested`, `runtime_stop_requested`, and the
harness correctly reports the clip as having opened no turn.

The harness publishes the clip as a `LocalAudioTrack` immediately after joining,
waits up to 10 s for the bot to appear before streaming, streams at 20 ms
cadence, and stays in the room until its deadline. It is doing what a caller
does; the track is published before any frame is sent.

## What has been ruled out

- **Reachability.** The bot connects (`connecting to ws://livekit:7880/rtc`) and
  joins its room; the caller joins the same room with the token the service
  issued for that call.
- **Publishing too early.** The harness now waits for the bot's
  `TrackSubscribed` / `ParticipantConnected` before sending frames.
- **Hanging up too soon.** The harness holds the room until its reply deadline
  rather than a fixed grace period.

## What to look at next

- Whether repeated calls reusing one `uid` — and therefore one participant
  identity — interact badly across rooms or with a not-yet-closed prior session.
- Whether the runtime's wait keys on an identity that differs in form from the
  one LiveKit reports for the caller.
- Whether the bot's 30 s wait begins before the caller has been issued a token,
  making it a race the caller can lose on a slow start.

## Reproducing

See `docs/tickets/0001-turn-never-ends-on-silence.md` for the environment; the
same command reproduces this. A batch of four clips is enough — the failure is
not deterministic, and one clip in a batch typically behaves differently from the
rest, which is itself a clue.

## Relationship to 0001

Independent. 0001 is about a turn that starts and never ends; this is about a
turn that never starts. Both have to be resolved before the live evaluation can
report a number.
