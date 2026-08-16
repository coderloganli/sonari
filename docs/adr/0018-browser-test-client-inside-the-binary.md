# ADR-0018: Ship a browser test client inside the binary

- **Status**: Accepted
- **Date**: 2026-08-16
- **Tags**: `ops`, `scope`
- **Related**: ADR-0007, ADR-0019, ADR-0020

## Context

There is no way for a person to talk to a running deployment. `POST /api/session`
returns a token and `POST /api/call/{character_id}/start` returns everything
needed to join a LiveKit room, but nothing in the repository carries either one
into a browser. Trying a change by hand means writing a throwaway page every
time, and the eval harness — which feeds WAV files and never opens a browser
(ADR-0010) — deliberately does not exercise the transport a person uses.

`docker compose up` on a clean clone is required to hold a conversation
(product.md §3). Anything the page needs at runtime that is not already in the
image breaks that: the compose file mounts exactly two paths, `models/` and
`sonari.toml`, and adding a third for a test page makes the test tool a
deployment concern.

Android remains the product surface. What is missing is a way to exercise the
call by hand, not a second product client.

## Decision

Serve a single-page browser test client from the sonari binary at `GET /dev`,
with its assets compiled in via `include_str!` rather than read from disk.

The page is same-origin with the API, so it calls `/api/...` with relative paths
and the API grows no CORS layer. It walks the existing contract and adds no
endpoint of its own beyond the persona list (ADR-0020): session, start call,
join the room, end call.

It is named a test client — in the route, in the page title, and in the
documents — so that it is not mistaken for the product surface.

## Consequences

- `docker compose up`, then one URL, and a person can talk to the agent.
- No volume, no asset directory, no Dockerfile change, no second container. The
  page cannot be missing at runtime: if the binary exists, the page exists.
- The production binary carries a test tool, and the release image serves it on
  the same port as the API. A deployment that must not expose it has to put a
  proxy in front — acceptable for a project that has no authentication at all
  (product.md §3) and would need that proxy regardless.
- Editing the page means recompiling. At the size of one page this costs less
  than the runtime path lookup it replaces.
- Every asset the page uses must be vendored, because there is nothing to serve
  from disk (ADR-0019).

## Alternatives considered

| Alternative | Why not |
|---|---|
| An HTML file in the repository, opened with `file://` | Cross-origin to the API, so it needs a CORS layer on production routes — a larger change than serving the page, and one that exists only for the test tool |
| `ServeDir` over an assets directory | Needs a third mount in compose and a path to configure; the page can then be absent at runtime, which is a failure mode a test tool should not have |
| A separate static container in compose | One more container and still cross-origin, so CORS anyway |
| Only the automated harness, no page | The harness cannot hear the agent, and cannot exercise WebRTC, echo cancellation or barge-in — the parts that only a person with a microphone can judge |
