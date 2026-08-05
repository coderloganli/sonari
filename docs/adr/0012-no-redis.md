# ADR-0012: No Redis

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `data`, `ops`
- **Related**: ADR-0002, ADR-0004

## Context

The predecessor's deployment includes Redis for two purposes: per-turn conversational state, and a call-event stream aggregated across instances.

Neither need is intrinsic to voice agents. Both are consequences of its process split. Because orchestration ran in one process and audio capture in another (ADR-0004), turn state had to be observable from both, so it was externalized — to Redis for live session state and to PostgreSQL for bot-speech state. The latter table was migrated five times in eleven days, which is what fighting a misplaced boundary looks like in a schema.

Under ADR-0002 and ADR-0004, one process owns a call for its entire lifetime. Turn state has exactly one observer.

Event volume argues the same way. A call produces on the order of dozens of turn-level facts, not thousands. That is an ordinary insert rate; it needs no queue, no consumer group, and no stream.

## Decision

No Redis. Orchestration state is an in-memory value owned by the task handling the call. Turn-level facts are written directly to PostgreSQL.

## Consequences

- One fewer container, one fewer failure mode, one fewer thing to configure.
- Turn state is an ordinary Rust value with a single owner — no serialization, no expiry, no cache coherence.
- State does not survive a process restart. This is correct: a call whose process died is over, and there is nothing useful to resume.
- Should the split deployment of ADR-0002 ever be exercised, a call is still owned end-to-end by one worker, so this decision holds. Assigning calls to workers needs a claim mechanism, but that is a dispatch concern, not shared conversational state.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Redis for session state | Solves a problem created by splitting the process; the split is not made (ADR-0002) |
| Redis Streams for events | Dozens of events per call is an insert, not a stream |
| Redis as a cache | Nothing on the hot path is worth caching — model configuration is a file, and conversation history is per-call and in memory |
