# ADR-0001: Record architecture decisions in ADRs

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `scope`
- **Related**: —

## Context

Sonari's architecture is derived from a small number of physical constraints rather than from convention. Several of its choices — a single service, no Redis, no multi-tenancy — look like omissions unless the reasoning is available. Without a record, a future contributor will either re-litigate settled questions or "fix" them back to the conventional shape.

The reasoning also cannot live in `architecture.md`: that document must describe the system as it currently is, while reasoning is a dated snapshot that becomes historically interesting rather than wrong.

## Decision

Record every architectural decision as a numbered ADR under `docs/adr/`, using the format in `template.md`. Accepted records are immutable; a changed decision produces a new ADR that supersedes the old one.

## Consequences

- Every non-obvious statement in `architecture.md` can be traced to a decision and its rejected alternatives.
- Reversing a decision requires stating what changed, which discourages drift by accretion.
- Cost: writing a record for each decision, and the discipline of not editing accepted ones.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Reasoning inline in `architecture.md` | Mixes "what is" with "why" — the document stops being usable as a reference, which is what happened to its first draft |
| Commit messages and PR descriptions | Not discoverable; nobody greps git history to find out why there is no Redis |
| A single `design-notes.md` | Grows without structure, has no status lifecycle, and cannot express supersession |
