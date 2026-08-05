# Sonari Documentation

## What lives where

| Document | Answers | Read it when |
|---|---|---|
| [architecture.md](architecture.md) | **How the system is built.** Components, domain model, interfaces, state machine, concurrency, failure handling | You are writing code and need to know where it goes |
| [adr/](adr/) | **Why it is built that way.** One decision per record, with the alternatives that were rejected | You disagree with something, or you are about to change it |

The split is deliberate. `architecture.md` describes the system as it stands and stays current. ADRs are dated snapshots of reasoning and are **never edited after acceptance** — when a decision changes, a new ADR supersedes the old one and the old one stays on disk.

---

## How to find things

**"Why is X the way it is?"**
Search `architecture.md` for X. Every statement that came from a deliberate decision carries an inline `(ADR-NNNN)` link. Follow it.

**"Show me every decision about latency."**
Open [adr/README.md](adr/README.md) and filter the index table by tag. Tags are a closed vocabulary: `process`, `audio`, `latency`, `providers`, `data`, `ops`, `scope`.

**"Was this ever decided differently?"**
The index table shows `Status`. A superseded record names the ADR that replaced it, and the replacement links back. Nothing is deleted, so the trail is complete.

**"I want to record a new decision."**
```bash
cp docs/adr/template.md docs/adr/00NN-short-title.md
```
Then add a row to the index table in `adr/README.md`, and add the `(ADR-00NN)` link at the relevant place in `architecture.md`.

**From the shell:**
```bash
rg -l 'tags:.*latency'     docs/adr/    # every latency decision
rg -A1 '^## Status'        docs/adr/    # current status of everything
rg 'Superseded'            docs/adr/    # what has been revisited
```

---

## Rules

1. **ADR numbers are permanent.** Never renumber, never reuse, never delete a file.
2. **Accepted ADRs are immutable.** Corrections go in a new ADR that supersedes the old one.
3. **`architecture.md` is always current.** If it disagrees with an ADR, the ADR is history and the document is right — but that means a superseding ADR is missing, so write it.
4. **One decision per ADR.** If the title needs an "and", it is two records.
