# Vendored assets for the browser test client

Served by `crates/api/src/dev_client.rs`, compiled into the binary with
`include_str!` (ADR-0018). Nothing here is read from disk at runtime.

## `livekit-client.umd.min.js`

| | |
|---|---|
| Package | `livekit-client` |
| Version | 2.21.0 |
| Source | `https://cdn.jsdelivr.net/npm/livekit-client@2.21.0/dist/livekit-client.umd.min.js` |
| Licence | Apache-2.0, text in `LICENSE-livekit-client` |
| Global | `LivekitClient` |

The UMD build is what LiveKit's client SDK README documents for use from a
script tag, which is why there is no build step here (ADR-0019).

To upgrade: replace the file, replace the licence text if it changed, and edit
the version above — in one commit, so the version in this table is always the
version in the file.
