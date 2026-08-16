# ADR-0020: List the configured personas over the API

- **Status**: Accepted
- **Date**: 2026-08-16
- **Tags**: `scope`, `ops`
- **Related**: ADR-0011, ADR-0018

## Context

A call starts at `POST /api/call/{character_id}/start`. The id is derived from
the persona's name — the first eight bytes of its SHA-256, masked positive — so
that editing `sonari.toml` does not renumber personas and invalidate history.
Nothing publishes those ids. A client that wants to start a call must therefore
already know an id it has no way to compute from anything a person can read.

For a person about to try a call by hand this is the whole difficulty: they know
the persona is called `companion`, and what the API wants is `4611...`. The
Android client will face the same wall.

## Decision

Add `GET /api/personas`, returning every persona in the live configuration as an
id and a name, plus the scene name where one is configured. Serve it
unauthenticated, alongside `POST /api/session`.

The list is read through a port on `character-context`, implemented in `app` by
the same `ConfigPersonas` that already resolves a persona by id, so there is one
place that turns configuration into personas and one definition of the id.

## Consequences

- A client can offer a choice without being told ids out of band, and the derived
  id stays an implementation detail of the server instead of a rule every client
  reimplements.
- The public API grows an endpoint, which the Android client will use too.
- Anyone who can reach the deployment can read the persona names. There is no
  authentication anywhere in front of a call — `POST /api/session` mints a token
  for any `uid` — so requiring one here would add a step, not a defence.
- Editing `sonari.toml` changes what the endpoint returns on the next request:
  the list is read from the live settings handle, not captured at startup.

## Alternatives considered

| Alternative | Why not |
|---|---|
| The client computes the id from the name | Copies a server rule into every client, and the two drift the moment the derivation changes |
| A person reads the id out of the logs | Makes trying a call a debugging exercise; the Android client cannot do it at all |
| Require a token to list personas | The token is free to mint and identifies nobody (product.md §3), so the requirement is ceremony |
| Return the full persona, prompts included | The prompts are the operator's material, and nothing in a client needs them |
