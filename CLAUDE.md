# Sonari — working notes

A real-time voice agent. See [docs/architecture.md](docs/architecture.md).

## Retrieving design context

Documentation is tiered so that answering a question costs a bounded amount of context. **Follow the tiers; do not read `docs/adr/` wholesale.**

| Question | Read |
|---|---|
| How is the system built? Where does my code go? | `docs/architecture.md` — always current, self-contained |
| What was decided about X? | `docs/adr/README.md` index only. The `Decision` column answers most questions outright |
| Why was it decided that way? What was rejected? | The one or two specific ADRs the index points to |
| Which decisions touch area X? | The `By tag` line in `docs/adr/README.md` |

Cost per tier: the index is a few lines per record; a full record is ~400 tokens. Opening more than three records to answer one question means the question should have been asked against the index.

To pull one section across many records without reading them:

```bash
rg -A4 '^## Decision' docs/adr/
rg '\*\*Status\*\*:'  docs/adr/
```

## Documentation rules

- **`architecture.md` describes the present.** It carries no rationale — decisions link out as `(ADR-NNNN)`.
- **Accepted ADRs are immutable.** A changed decision is a new ADR that supersedes the old one; the old file stays.
- **ADR numbers are permanent.** Never renumber, reuse, or delete.
- **One decision per ADR.** A title needing "and" is two records.
- Changing an architectural decision means writing the superseding ADR **in the same change** as the code.

## Conventions

- Everything committed to this repository is written in English — docs, code, comments, commit messages, configuration.
- No latency figure enters any document until it has been measured (ADR-0010).
- Third-party API behaviour, model names, and parameters are verified against official documentation before being written down. Unverified claims are labelled as such.
