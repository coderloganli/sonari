# ADR-0013: Enforce plane boundaries with crate visibility

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `process`, `scope`
- **Related**: ADR-0002

## Context

ADR-0002 keeps both planes in one process, which means nothing physically prevents control-plane code from reaching into media-plane internals. Left unaddressed, the boundary erodes: someone under deadline reaches across, it works, and within months the seam that ADR-0002 depends on no longer exists.

The predecessor demonstrates both halves of this. It enforced boundaries through a written layering document, and that document's prohibitions read as a post-mortem — entries forbid consumer-local SQL mirrors of another module's tables, duplicated boundary contracts with translation shims, and using another module's broad use-case trait where a narrow port exists. Each was added after the violation was found.

It also shows that crate count is not the answer. It reached 45 crates, 14 of them under 100 lines, with 10 using `#[path]` to place source outside `src/` — and the layering still failed to constrain size, producing a 16,845-line file. Directional structure was enforced; nothing else was.

A rule a person can violate is a rule that will be violated. A rule the compiler rejects is not.

## Decision

Organize the backend as eight crates. Media-plane crates keep their internals private; only the types crossing the plane boundary are `pub`.

```
sonari-core/        domain types + trait definitions, no implementations
sonari-pipeline/    orchestration state machine        ┐
sonari-providers/   ASR / TTS / LLM implementations    ├─ media plane
sonari-rtc/         LiveKit integration                ┘
sonari-store/       PostgreSQL persistence             ┐
sonari-api/         HTTP control plane                 ┘─ control plane
sonari-telemetry/   latency markers + metrics
sonari/             binary entrypoint, composition root
```

`sonari-api` and `sonari-store` do not depend on `sonari-pipeline`, `sonari-providers`, or `sonari-rtc`. The planes communicate through types defined in `sonari-core`. Only the `sonari` binary crate depends on both sides, and only to wire them together.

## Consequences

- Crossing the boundary is a compile error, not a review comment.
- The set of types crossing the boundary is exactly the `pub` surface of `sonari-core` — enumerable, and reviewable when it grows.
- Splitting into separate binaries (ADR-0002) becomes a change to the composition root, because no other code spans both planes.
- Eight crates is small enough to hold in mind. Crates exist to enforce a boundary; a module suffices where there is no boundary to enforce.
- Cost: some code that would naturally sit together is separated by the boundary, and adding a type to the crossing surface requires touching `sonari-core`. This friction is the mechanism working.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One crate with module conventions | Rust module privacy does not prevent a sibling module from reaching in; the boundary reverts to convention |
| A crate per DDD layer, as the predecessor did | 45 crates, 14 trivial, 10 fighting Cargo's layout — and it still failed to constrain what mattered |
| A written layering document plus code review | Exactly what the predecessor tried; the resulting document is a list of violations already committed |
| A custom lint or `cargo-deny` dependency rules | Better than prose, but a second mechanism to maintain when crate visibility already expresses it |
