# ADR-0019: Vendor the LiveKit browser SDK at a pinned version

- **Status**: Accepted
- **Date**: 2026-08-16
- **Tags**: `ops`, `audio`
- **Related**: ADR-0007, ADR-0018

## Context

The test client joins a LiveKit room from a browser, which means using LiveKit's
JavaScript client SDK. Its README documents a script-tag build —
`dist/livekit-client.umd.min.js`, exposing the global `LivekitClient` — so no
bundler is required. The package is `livekit-client`; version 2.21.0 is
Apache-2.0 and the UMD build is 562 KB minified. Both facts were read from the
npm registry metadata and the SDK README, not from memory.

The page has no build step and none is wanted: a `package.json` and a bundler
would put a second toolchain into a Rust repository for the sake of one page.
That leaves two ways to get the file into a browser — fetch it from a CDN at
page load, or keep a copy in the repository.

`docker compose up` on a clean clone must hold a conversation (product.md §3),
and the page is compiled into the binary (ADR-0018), so anything it loads at
runtime from a third party is a way for the test tool to stop working for
reasons that have nothing to do with the deployment.

## Decision

Vendor `livekit-client` 2.21.0's UMD build into the repository and compile it
into the binary alongside the page. Record its origin, version and licence in a
README beside it.

Upgrading is an explicit commit that replaces the file and the recorded version.

## Consequences

- The page works with no internet access beyond what a call already needs, and
  the version in use is whatever the commit says — a page that worked last month
  works today.
- The repository carries 562 KB of minified third-party JavaScript, and the
  binary grows by about that much.
- Apache-2.0 requires the licence and notice to travel with the copy; the README
  beside the file carries them.
- Upgrading is manual and will be forgotten. It is a test tool, so a version
  behind costs little, and the recorded version makes the drift visible.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A pinned CDN URL in a script tag | Gives the test tool an external dependency the product does not have; a blocked network or a CDN outage looks like a broken deployment |
| `package.json` plus a bundler | A second toolchain in a Rust repository, for one page with no build needs |
| Implement the WebRTC signalling by hand | Rejected for the same reason LiveKit was chosen at all (ADR-0007) |
