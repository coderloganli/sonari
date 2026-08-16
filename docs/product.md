# Sonari — Product Requirements

What the system is for and what it must do. How it is built is in
[architecture.md](architecture.md); why, in [adr/](adr/).

---

## 1. What it is

A real-time voice agent for emotional companionship. Someone opens the app,
picks a character, and talks to it the way they would talk to a person on the
phone.

The engineering interest is the call itself: how quickly it answers, how
naturally it takes turns, and whether it can be interrupted. Everything else
exists to make that testable.

## 2. Scope

**v1** — one voice conversation, measured.

| | |
|---|---|
| Voice pipeline | Detect speech, recognise it, answer, speak the answer |
| Latency | Under two seconds from the caller finishing to the first sound back |
| Interruption | Speaking over the agent stops it |
| Personas | Operator-authored: a character and the scene they are in |
| Identity | A `uid` the caller enters or is assigned |
| Client | Android — enter a `uid`, choose a character and scene, talk |
| Trying it by hand | A browser test client at `/dev`, served by the binary itself (ADR-0018) |
| Evaluation | An automated harness and a headless caller, both runnable in CI |

**v2** — long-term memory, and work on how human the agent sounds. v1 records
the `uid` on every session so memory has history to work with when it arrives.

**Not built**: SDK surface, billing, admin console, multi-tenancy, consumer
login, tool calling, self-hosted inference.

## 3. Requirements

### The call

- The agent answers within two seconds of the caller stopping, measured from
  speech end to the first audio frame reaching the client.
- Speaking over the agent stops its playback. This is decided from the same
  signal that decides when a turn ended, so the two cannot disagree.
- Replies are one or two sentences. A paragraph is both slower to produce and
  worse to listen to.
- A failure is spoken or reported, never silence. A caller waiting on a dead
  turn has no way to tell it is dead.

### Personas

- A persona is a character and, optionally, a scene: who the agent is, how it
  speaks, where the conversation is set, and what it is for.
- Personas are authored by whoever runs the deployment, not by callers, and are
  edited often. Editing one takes effect on the next call without a restart.
- A persona names the voice it speaks with.

### Identity

- No account, no password, no phone number. A caller presents a `uid` — a short
  readable string they can type again on another device.
- The same `uid` reaches the same history anywhere.
- This identifies; it does not authenticate. Anyone who types someone's `uid`
  reaches their history, and the documentation says so plainly rather than
  implying otherwise.

### Operating it

- `docker compose up` on a clean clone holds a conversation. No seed data, no
  admin bootstrap, no console.
- Everything an operator edits is one file, versioned in git. Secrets are not in
  it.
- An invalid configuration is refused at startup rather than half-applied.

### Evidence

- Latency is measured, not asserted. No figure appears in any document before it
  has been measured, and measurements come from release builds.
- Two figures are always reported together: the system's own response time, and
  what the caller actually waits.
- Both the inference path and the transport path have automated tests that run
  without a person or a phone.

## 4. What callers should know

Audio leaves the deployment. Speech goes to a recognition provider and replies
are synthesised by one; transcripts go to a model provider. Anyone running this
for other people is sending those people's voices to third parties, and should
say so.

## 5. Out of scope, and why

| | |
|---|---|
| Accounts and login | A companion does not need to know who you are, only which conversation is yours |
| Self-hosted models | The engineering interest is the pipeline, not operating GPUs (ADR-0014) |
| Tool calling | v1 is conversation. Tools add a second round trip inside a turn, which a phone call feels |
| Multi-tenancy | One deployment, one operator, personas in a file |
| A web *product* client | Android is the product surface. A browser page is shipped at `/dev` as a test tool and is named one (ADR-0018) — it exists because a person needs to hear the call, which no automated test can judge |
