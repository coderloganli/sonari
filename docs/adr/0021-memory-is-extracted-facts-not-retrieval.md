# ADR-0021: Carry long-term memory as an extracted fact set, injected whole

- **Status**: Accepted
- **Date**: 2026-08-17
- **Tags**: `data`, `latency`, `scope`
- **Related**: ADR-0008, ADR-0022, ADR-0023

## Context

A companion agent that forgets between calls fails at the thing it exists for.
Conversation history today is the last six turns of the current session, so
nothing survives hanging up.

The obvious answer is retrieval: embed the caller's utterance, search past turns,
inject the top matches. It does not fit this system, for four reasons.

**Latency.** Retrieval happens inside the turn — embed, search, then prompt. The
whole turn budget is two seconds (product.md). Published per-query figures for
hosted memory services are of the same order as that entire budget; the vendors
publishing them dispute each other's measurements, and none of them have been
reproduced here. Whatever the true number, it is not small relative to two
seconds, and it lands on the critical path.

**Retrieval fires on similarity, and the facts that matter are not similar to
anything.** A caller does not say "how is my cat" to prompt the agent into
remembering there is a cat; they expect to be asked. What a companion must know
is unconditionally relevant, which is precisely what a relevance-ranked search
will not surface.

**The retrieval unit is wrong.** Past turns are transcripts: filler, false
starts, and recognition errors. Injecting them puts recognition mistakes back
into the prompt as though they were established fact.

**The volume does not call for it.** What is stably true about one caller is tens
of short sentences. Retrieval is a technique for context that does not fit; this
fits several times over.

## Decision

Store long-term memory as a bounded set of **facts**: one natural-language
sentence, one category from a closed list — `identity`, `relationship`,
`preference`, `situation`, `commitment`. Inject the whole set into the prompt as
one system message. Do not search it.

Structure the row, not the sentence. A companion's facts are an open set —
"afraid of flying" belongs to no column anyone would have thought to add — so
typed fields would force a migration for every new kind of thing a person can
say. The category exists for eviction quotas and for grouping the injected text,
not to make the fact machine-readable.

A model rewrites the whole set from the previous set plus recent turns, and the
result replaces it. There is no per-fact deduplication.

## Consequences

- The turn path gains one indexed local `SELECT` and no network call. The
  two-second budget is untouched.
- What the agent knows is a finite, readable set. A test can assert on it, a
  caller can be shown it, and a caller can delete it.
- Categories give eviction something to be fair about: a cap per category keeps a
  talkative week of `situation` facts from evicting the caller's name.
- Cost: rewriting the whole set is lossy. The model can silently drop a fact it
  should have kept. The raw turns remain in `llm_messages`, so a damaged set can
  be rebuilt; the set is small enough to read; and `GET /api/memory` exists partly
  so the loss is visible rather than theoretical.
- Cost: prompt length grows with the set. The cap is what bounds it, and the cap
  is configuration.
- Retrieval is not ruled out — it is the right tool for episodic memory, which is
  a later task. The `pgvector` image stays as it is (architecture.md §6); nothing
  here uses it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Vector retrieval over past turns | In-turn latency against a two-second budget; fires on similarity when the needed facts are unconditional; retrieves transcripts rather than conclusions |
| Typed columns (`name`, `occupation`, `pets`) | The set of things worth remembering about a person is open; every new kind is a migration |
| One free-text block rewritten each time | Nothing to cap, nothing to evict fairly, nothing to assert on, and single facts cannot be deleted |
| Keep every past turn in the prompt | Unbounded prompt growth; recognition errors accumulate; cost per turn rises with the length of the relationship |
