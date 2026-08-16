# 0005 — The client sometimes starts listening after the greeting

**Status**: Resolved 2026-08-15 — not a service defect · **Found by**: the evaluation harness · **Area**: eval clients

## The question

The probe heard nothing while the service logged a greeting synthesised and
played, and heard the reply perfectly a moment later. Either the greeting never
reached the room, or the client was not yet listening.

## The answer

It was the client. A second run, same build, same clip:

```
agent track level  second=1  peak=0
agent track level  second=2  peak=12356    ← the greeting, plainly audible
agent track level  second=3  peak=10522
agent track level  second=4  peak=690
```

The greeting is written to the room and is loud. In the run where it was missed,
the first per-second line was `second=3` — the client's audio stream produced no
frames at all for its first three seconds, and the greeting had come and gone
inside that window.

So the intermittency was never in the audio: sometimes the stream is delivering
by the time the agent speaks, sometimes it is not.

## What follows

- **No service change.** The greeting is fine.
- Both clients wait before speaking, which remains right, but neither can rely on
  *hearing* the greeting end. Their fallback timer is what actually runs when
  the stream starts late, and ticket 0004 records that.
- A client that needs the first second of a call — a browser front end showing
  "the agent is speaking", say — cannot assume its stream is live the moment it
  subscribes.

## The observation left over, and what it turned out to be

The silent run showed the agent never speaking again, and `silence_force_agent_ms`
is 8000, which read like a promise it had broken. A clip was added to measure it
— `idle-force-agent`, twelve seconds of nothing — and it reported one server turn,
the greeting, and no more.

Reading the code before reporting that: the setting is not what its name says.
The deadline is set at commit time as `silence_force_agent_ms - silence_flush_ms`
and is spent in `take_forced_transcript_turn_if_overdue` — it is how long to wait
for recognition's final result before proceeding with the partial. A caller who
says nothing gets no turn from it, and the agent's silence was correct.

The comment in `crates/config` now says what the code does. The clip stays: what
happens when a caller says nothing is worth watching whether or not a setting
promises it, and the report states the count without implying a verdict.
