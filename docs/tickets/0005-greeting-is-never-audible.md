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

## One observation left over

In the silent run the caller sent nothing but noise floor from second 5 to second
24, and `silence_force_agent_ms` is 8000, so the agent should have spoken on its
own. The track stayed at zero throughout. Whether that timer fires at all is
unmeasured, and would be worth a clip of its own: an evaluation set that only
ever speaks cannot see it.
