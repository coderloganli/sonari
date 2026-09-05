# Sonari Memory

What the agent remembers, how it learns it, and every choice that shaped it.

This is the design document for one subsystem. [architecture.md](architecture.md)
says where memory sits in the system; [product.md](product.md) says what it is
for; the [ADRs](adr/README.md) hold the decisions that were made and are
immutable. **This document holds the option space** — including the options not
taken — so that anyone changing memory sees the whole board rather than only the
square that was chosen.

---

# Part 1 — The memory systems

## 1.1 Four kinds, three states

The vocabulary is the ordinary one: what is remembered within a conversation,
what is true about a person, what happened, and how to behave.

| Kind | What it holds | In Sonari | Status |
|---|---|---|---|
| **Working / short-term** | The current conversation | Last six turns of the session, re-read each turn | Exists, unexamined |
| **Semantic** | Facts about the caller | `agent_memory_facts` — this document | **Built** |
| **Episodic** | What happened in a particular past call | — | Not built |
| **Procedural** | How to behave | Personas and prompts in `sonari.toml` | Exists |

"How far we got in this call" is not a fourth kind. It is working memory, and
here it is whatever fits in the six-turn window. Extending it means a rolling
summary, not a new store.

## 1.2 What semantic memory is

One fact is a **category** and **one sentence of natural language**, plus three
pieces of metadata:

```
category:          relationship
content:           "The caller has a cat called Coal."
first_seen_at:     when it was first learned
updated_at:        when an extraction last confirmed it
source_session_id: which call confirmed it
```

**The row is structured; the sentence is not.** What is worth remembering about a
person is an open set — "afraid of flying" belongs to no column anyone would have
thought to add — so typed fields would force a migration for every new kind of
thing a person can say (ADR-0021).

The category exists for two mechanical purposes and no others: giving eviction a
quota to be fair about, and grouping the injected text. It does not make the fact
machine-readable.

| Category | Holds | Note |
|---|---|---|
| `identity` | Name, age, where they live, what they do | Most stable; the prompt forbids dropping one without contradiction |
| `relationship` | Family, friends, pets, colleagues | |
| `preference` | What they like, dislike, will not discuss | |
| `situation` | What is going on now | The category that expires |
| `commitment` | What was agreed between them and the agent | Strongest effect on a companion; most often missed by general-purpose memory |

The list order is the rendering order: stable first, passing last.

## 1.3 Storage

```sql
create table agent_memory_facts (
  id                bigserial primary key,
  user_id           bigint      not null,
  character_id      bigint      not null,
  category          text        not null,
  content           text        not null,
  first_seen_at     timestamptz not null,
  updated_at        timestamptz not null,
  source_session_id text        not null,
  constraint agent_memory_facts_category
    check (category in ('identity','relationship','preference','situation','commitment')),
  constraint agent_memory_facts_unique unique (user_id, character_id, content)
);
create index agent_memory_facts_owner on agent_memory_facts (user_id, character_id);
```

No vector column. The set is injected whole and never searched, so there is
nothing to embed. The `pgvector` image stays for episodic memory (architecture.md
§6); no column uses it today.

Keyed on `(user_id, character_id)`: what one persona was told, another does not
know (ADR-0023).

## 1.4 Reading — on the turn path

`build_chat_messages` and `generate_welcome_message` each load the set for the
session's `(user_id, character_id)` and render it into one system message:

```
system: conversation_system / character / scene   ← persona
system: what you already know about this person   ← memory
[the recent six turns]
user:   this utterance
```

Everything is injected; nothing is selected. The query is ordered explicitly by
`first_seen_at, id` so the text does not reshuffle between turns of one call.

**Cost: one indexed local query, no network call.** A read that fails is logged
and the turn proceeds without the message — memory failing makes the agent
forgetful, never makes a call fail.

Rendering an empty set produces nothing at all, so an agent with no memory sends
exactly the prompt it sent before this existed.

## 1.5 Writing — off the turn path

Every `extract_every_turns` completed turns, `chat_once` calls
`MemoryExtractionScheduler::schedule` and returns. The composition root's
scheduler spawns the work; the turn never awaits it (ADR-0022).

The extraction:

1. Loads the session, the current fact set, and the last `extract_every_turns`
   turns.
2. Sends the model the **whole current set plus those turns**, and asks for the
   set as it should now stand — not a list of edits (ADR-0021).
3. Parses the JSON reply. Facts with a category outside the closed list are
   dropped and counted; a reply that is not a fact set at all is abandoned.
4. Validates: trims, drops empties, deduplicates on content case-insensitively,
   caps per category, caps the total, in the order the model gave.
5. Replaces the stored set in one transaction — a fact whose content is unchanged
   keeps its `first_seen_at`, a fact absent from the new set is deleted, a new
   one is inserted.

**A fact disappears by not being mentioned again**, which is the same action as
the model failing to mention it. That is the central cost of this shape; see
D3.

One extraction per session at a time; a schedule arriving while one is running is
dropped. The extraction window equals the cadence, so turns covered by a dropped
extraction are not revisited by the next one.

Two refusals protect the stored set: an unparseable reply and an empty validated
set both leave it untouched. A model having a bad turn is far likelier than a
caller whose every fact stopped being true, and the two mistakes do not cost the
same.

## 1.6 The caller's own view

| Route | Does |
|---|---|
| `GET /api/memory` | Every fact held for this caller, across personas |
| `DELETE /api/memory` | Forgets all of it; `?character_id=N` narrows it to one persona |

Both behind the ordinary token; the caller is `claims.subject_id`. Nothing in the
request names whose memory it is, so nothing in the request can ask for someone
else's.

Read and delete only. This exists because a `uid` identifies without
authenticating, so notes kept about a person have to be visible to them
(product.md §4) — and because it is the only way to see what extraction is
actually doing.

## 1.7 Configuration

```toml
[memory]
enabled = true
extract_every_turns = 4       # completed turns between extractions
max_facts = 40                # what bounds prompt length
max_facts_per_category = 12
model = ""                    # empty means the conversation model
```

Validated at startup: intervals and caps at least 1, per-category cap no greater
than the total, and `prompts.memory_extraction` non-empty when enabled. An
invalid file is refused rather than half-applied.

Omitting the section leaves memory off, so a configuration written before this
existed does not silently start extracting.

The extraction model is the `Assistant` provider slot, at temperature 0 —
extraction is parsing, not style, and is not operator-tunable.

## 1.8 Failure behaviour

| Failure | Effect |
|---|---|
| Memory read fails | The turn runs without the fact set; logged |
| Extraction endpoint unreachable | Nothing new is remembered; stored set untouched |
| Reply will not parse | Same |
| Extraction yields no storable facts | Same |
| Two live sessions, same caller and persona | Last writer wins; not defended against (ADR-0022) |

## 1.9 Where this design ends

Whole-set injection is right for **tens** of facts. At `max_facts = 40` the
message is a few hundred tokens. Wanting hundreds of facts means retrieval, and
retrieval means a different design — that is the point at which this document is
rewritten rather than extended.

## 1.10 Assumptions not yet measured

Stated plainly because none of them are backed by data, and ADR-0010 forbids
inventing figures:

| Assumption | Now | How it would be settled |
|---|---|---|
| `extract_every_turns = 4` fits real calls | Guess | Turn-count distribution over `llm_messages` |
| `max_facts = 40` is enough, and cheap enough | Guess | Profile growth curve; effect of prompt length on `llm_first_token` |
| Five categories cover what callers say | Guess | The `unknown_categories` count already in the extraction log |
| Whole-set rewrite loses little | **Unmeasured** | Offline: scripted conversations, repeated extractions, count facts that vanish without being superseded |
| The model orders by importance, so tail-cutting is safe | Unmeasured | Requires judged evaluation |
| The extraction prompt selects the right things | **Untested** | Needs an evaluation set; no test covers this today |

The fourth row is the one that would change a structural decision.

---

# Part 2 — Decision points

Each is a real fork. The chosen option is marked **✓**; where a decision has an
ADR, the reasoning lives there and is not repeated.

## D1 — How memory reaches the prompt (ADR-0021)

| Option | For | Against |
|---|---|---|
| **✓ Inject the whole set** | No in-turn network call; unconditionally relevant facts are always present; testable and showable | Prompt grows with the set; hard cap on how much can be remembered |
| Vector retrieval per turn | Scales to thousands of facts | Embedding and search inside a two-second turn; fires on similarity when what matters is unconditional; retrieves transcripts, ASR errors included |
| Retrieve once per call, cache | Scales, and costs the turn nothing after the first | Still one network call at call start; needs a relevance signal before the caller has said anything |

## D2 — When extraction runs (ADR-0022)

| Option | For | Against |
|---|---|---|
| **✓ Every N turns, off the turn task** | Turn path pays a modulo and a spawn; long calls stay current | Facts from the last turns before a hang-up can be missed; background model call beside a live one |
| Inside the turn | Immediately available | A second model call on the critical path |
| At the end of the call | Cheapest; one call per conversation | Callers hang up; the end is not reliably observed, and everything learned goes with it |
| Periodic sweep over sessions | Decoupled entirely | A scheduler and a claim protocol to discover what the turn already knows |

## D3 — How the set is updated (ADR-0021)

**The decision most worth revisiting.**

| Option | For | Against |
|---|---|---|
| **✓ Whole-set rewrite** | The model can *tidy* — merge, rephrase, reclassify — not only append; no ids leave the database; reconciliation is three statements; output is a final state with no partial application | A fact disappears by not being mentioned, so a model that forgets to write one deletes it; blast radius of one bad reply is the whole profile; output tokens grow with the set |
| Incremental `ADD` / `UPDATE` / `DELETE` | Blast radius is the rows named; output grows with new information only; deletion is explicit | Ids must be exposed to the model and validated back, hallucinated ids handled; merging two facts is harder to express; more test surface |
| Additive only, never delete or overwrite | Nothing is ever lost | The profile accumulates contradictions and stale facts; nothing bounds it |
| Temporal invalidation (mark superseded, keep history) | History survives; contradictions resolve without loss | A second dimension in the schema and in every read; more than tens of facts need |

A cheap middle path exists and is not implemented: reject a rewrite that drops
`identity` facts or shrinks the set beyond a threshold, and log it.

## D4 — The shape of one fact (ADR-0021)

| Option | For | Against |
|---|---|---|
| **✓ Category + one natural-language sentence** | Open set of things worth remembering; readable by a person; injectable as-is | Not machine-queryable — "everyone with a cat" is not a query |
| Typed columns (`name`, `occupation`, `pets[]`) | Queryable; naturally bounded | Every new kind of fact is a migration; most of what a person says fits no column |
| One free-text block | Simplest to write and rewrite | Nothing to cap, nothing to evict fairly, nothing to assert on, no single fact can be deleted |

## D5 — The category vocabulary

| Option | For | Against |
|---|---|---|
| **✓ Closed list of five** | Eviction has a quota to be fair about; injected text groups; a check constraint enforces it | Facts that fit no category are dropped (counted, but dropped) |
| Open tags | Nothing is ever unclassifiable | No basis for a per-category quota; tags proliferate and mean nothing |
| No category at all | Simplest | Eviction can only cut by time, so a talkative week erases the caller's name |

## D6 — Who a fact belongs to (ADR-0023)

| Option | For | Against |
|---|---|---|
| **✓ `(caller, persona)`** | A persona never refers to something it was never told | Switching persona starts over; storage and extraction cost multiply by personas |
| Caller only | One profile, learned once, available everywhere | A persona speaks about things it was never told — for a companion, worse than forgetting |
| Caller, with per-persona visibility rules | Both | A policy layer nobody asked for, over tens of sentences |

## D7 — What is dropped when the cap is reached

| Option | For | Against |
|---|---|---|
| **✓ Cut from the tail of the model's own ordering** | No scoring machinery; the model states its priority by ordering | Rests on an unverified assumption that the ordering means anything |
| Recency + importance + relevance scoring | Principled, and the literature's answer | Needs an importance signal per fact and a scorer to produce it |
| Oldest first | Trivial and predictable | Deletes the caller's name, which is the oldest thing known about them |

## D8 — An extraction that returns nothing usable

| Option | For | Against |
|---|---|---|
| **✓ Leave the stored set untouched** | A model having a bad turn cannot erase a person | The model can never legitimately clear a profile; only `DELETE /api/memory` can |
| Apply it — an empty set means forget everything | The model's judgement is respected | One bad reply erases a relationship |

## D9 — What the caller can do with their profile

| Option | For | Against |
|---|---|---|
| **✓ Read and delete** | Answers the privacy question a `uid` creates; the only way to see extraction quality | A wrong fact can only be deleted, not corrected |
| Nothing — logs only | Smallest surface | What is held about a person is invisible to them |
| Full read/write/delete | Corrections possible | A write surface nobody asked for, in front of an identity that does not authenticate |

## D10 — Whether memory is on by default

| Option | For | Against |
|---|---|---|
| **✓ Off when the section is absent, on in the example file** | An existing deployment does not silently start sending notes about callers to the model provider; a clean clone still demonstrates the feature | Two states to reason about |
| On by default | The feature is never accidentally invisible | An upgrade changes what leaves the deployment, without anyone deciding |
| Off everywhere, including the example | Most conservative | `docker compose up` on a clean clone no longer shows what the system does |

## D11 — Where the fact set is held during a call

| Option | For | Against |
|---|---|---|
| **✓ Read per turn from PostgreSQL** | No cache lifetime to manage; matches how the session and recent turns are already read | One indexed local query per turn |
| Load once at call start, hold in the session | One query per call | Session-scoped state where there is none today, for a query that costs a local round trip |

## D12 — Which model extracts

| Option | For | Against |
|---|---|---|
| **✓ A configurable slot, defaulting to the conversation model, temperature 0** | A cheaper model can be used; extraction is parsing, so sampling is fixed rather than offered as a dial | One more configuration value |
| Always the conversation model | Nothing to configure | Pays conversation-model prices for a parsing job |
| A dedicated fine-tuned extractor | Best quality per token | Nothing to fine-tune on, and an operational burden this project rejects elsewhere |

## D13 — How this is tested

| Option | For | Against |
|---|---|---|
| **✓ Fakes throughout; no test judges model output** | Deterministic, runs in CI, needs no key or database | **Nothing verifies what gets remembered** — only what the system does with what the model said |
| Evaluation set with a judge | Covers the one thing fakes cannot | Judged scoring is itself uncertain; worth building when the prompt starts to iterate |
| Live-model integration tests | Real behaviour | Non-deterministic, cannot gate CI, costs money per run |

---

## Related records

- [ADR-0021](adr/0021-memory-is-extracted-facts-not-retrieval.md) — an extracted
  fact set, injected whole; not retrieval
- [ADR-0022](adr/0022-memory-extraction-runs-off-the-turn-path.md) — extraction
  every N turns, off the turn task
- [ADR-0023](adr/0023-memory-is-scoped-to-caller-and-persona.md) — scoped to the
  caller and the persona
- [ADR-0008](adr/0008-text-core-as-first-class-entrypoint.md) — memory lives in
  the text core, so the audio path does not know it exists
