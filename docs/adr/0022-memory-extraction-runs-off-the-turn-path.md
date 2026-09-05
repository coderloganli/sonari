# ADR-0022: Run memory extraction off the turn path

- **Status**: Accepted
- **Date**: 2026-08-17
- **Tags**: `latency`, `process`, `data`
- **Related**: ADR-0012, ADR-0021

## Context

Turning conversation into facts costs a model call. The turn budget is two
seconds end to end (product.md), and a second model call inside a turn would
spend a large part of it on work the caller is not waiting for.

There is a real trade in when the extraction runs. Doing it when the call ends is
cheapest, but a call that never ends cleanly — the common case, since a caller
hangs up — loses everything learned in it. Doing it during the call keeps long
calls current, at the price of concurrent work beside a live conversation.

## Decision

Extract every N completed turns, N being configuration, on a task spawned outside
the session task. The turn path schedules the work and returns; it never awaits
it.

Extraction reads the recent turns and the current fact set, asks the model for a
replacement set, and writes it. Everything it touches is already persisted, so it
holds no reference to live session state.

One extraction per session at a time. A schedule arriving while one is running is
dropped, not queued: the next one will see the same turns plus more.

Extraction failure — an unreachable endpoint, output that will not parse — is
logged and abandoned. The stored set is left as it was.

## Consequences

- The turn path costs one scheduling call: a modulo and a spawn.
- A long call keeps its memory current rather than banking all of it against a
  hang-up that may never be observed.
- Facts learned in the current call are not in the fact set until the next
  extraction lands. Within the call this costs nothing, because the six-turn
  window still carries them; across calls, a fact said in the last turns before a
  hang-up can be missed. That is the accepted cost of having no reliable
  end-of-call signal.
- Memory can degrade without conversation degrading, which is the rule
  persistence already follows (architecture.md §3).
- Cost: a background model call runs beside a live one, adding load and spend per
  call. `extract_every_turns` is what bounds it.
- Cost: the write is last-writer-wins over a whole set. Two calls by the same
  caller to the same persona at once can lose one side's facts. This is not
  defended against; one caller holding two simultaneous calls is not a case this
  system has.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Extract inside the turn | A second model call on the critical path against a two-second budget |
| Extract when the call ends | Callers hang up; the end is not reliably observed, and everything learned goes with it |
| Extract on a periodic sweep over all sessions | A scheduler and a claim protocol to discover what one line at the end of a turn already knows |
| Queue overlapping extractions | The queued run would read the same rows plus a few more; the queue buys nothing and can grow |
