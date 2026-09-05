# ADR-0023: Scope long-term memory to the caller and the persona

- **Status**: Accepted
- **Date**: 2026-08-17
- **Tags**: `data`, `scope`
- **Related**: ADR-0011, ADR-0021

## Context

Facts are learned inside a conversation with one persona. Whether another persona
should see them is a product question, not a storage one. A `uid` reaches the
same history on any device (product.md), which makes the caller the obvious key;
the question is whether the persona is part of it.

## Decision

Key the fact set on `(user_id, character_id)`. What a caller told one persona is
not visible to another.

## Consequences

- Each persona's knowledge matches its own history with the caller. A persona
  never refers to something it was never told, which is the failure that reads as
  broken rather than merely forgetful.
- The key is already on `AgentSession`; no new identity is introduced, and
  ADR-0011 still holds — this is not a tenant dimension, it is the two ids the
  session already carries.
- Deletion has a natural narrow form and a natural broad one: one persona's
  facts, or everything the caller has anywhere.
- Cost: a caller who switches persona starts over. For a companion that is
  arguably correct, but it is a real loss and callers will notice it.
- Cost: storage multiplies by the number of personas a caller talks to. At tens
  of sentences per pair this does not matter.
- Cost: the same fact is extracted once per persona, so the same model call
  happens more than once across personas.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Scope by `uid` alone | One persona speaks about things it was never told; for a companion that breaks the character more than forgetting does |
| Scope by `uid`, with per-persona visibility rules | A policy layer nobody has asked for, over a set of tens of sentences |
| Scope by session | That is the six-turn window, which already exists; it is not long-term memory |
