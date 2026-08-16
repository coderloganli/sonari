# Sonari

A real-time voice agent you can hold a conversation with. One process, hosted
inference, sub-2s response.

Speech in, speech out: voice activity detection runs in-process, recognition and
synthesis at ElevenLabs, the model at any OpenAI-compatible endpoint. The turn
state, the endpointing and the interruption logic are yours to read and change.

---

## Running it

You need Docker, an ElevenLabs API key, and a key for an OpenAI-compatible model
endpoint.

```bash
cp .env.example .env                 # fill in ELEVENLABS_API_KEY and LLM_API_KEY
cp sonari.toml.example sonari.toml   # who the agent is, and how it listens
./scripts/fetch-models.sh            # one file: the voice activity model
docker compose up
```

Three containers come up — the agent, PostgreSQL and LiveKit. Nothing needs
seeding: a persona is a section of `sonari.toml`, and a caller is a `uid` they
choose.

```bash
curl -X POST localhost:8080/api/session \
  -H 'content-type: application/json' \
  -d '{"uid":"brave-otter-4417"}'
```

That returns a token. The client carries it, joins the LiveKit room a call
returns, and talks.

## What sends where

**Audio leaves your deployment.** Every frame a caller speaks goes to ElevenLabs
for recognition, and every reply is synthesised there. Transcripts go to
whichever model endpoint you configure. If that is not acceptable for your
callers, this is not the right system for them, and the decision is recorded in
[ADR-0014](docs/adr/0014-hosted-inference-for-recognition-and-synthesis.md)
along with what it bought and what it cost.

**There is no authentication.** A `uid` identifies a conversation history; anyone
who types someone else's `uid` reaches it. Bind the service to localhost or put
it behind something that authenticates. This is stated plainly rather than
hidden behind a login screen that does nothing.

## Configuration

`sonari.toml` holds everything an operator edits — the persona and its scene,
the prompts wrapped around it, which models to ask for, and when a turn starts
and ends. It is watched: save the file and the next call uses the new version.
An invalid file is rejected and the running configuration keeps going.

The environment holds only what should never be in a file: the two API keys, the
database DSN, and where LiveKit is.

## Layout

| | |
|---|---|
| `crates/providers` | Voice activity detection, and the ElevenLabs adapters |
| `crates/voice` | The provider traits the call path speaks to |
| `crates/call/*` | Sessions, dispatch, the media pipeline, LiveKit |
| `crates/agent` | Prompt assembly, conversation history, the model client |
| `crates/config` | `sonari.toml` — parsing, validation, watching |
| `crates/harness` | Drives one turn from a WAV file and reports what it cost |
| `crates/probe` | Joins a call over WebRTC as a caller that is not a person |
| `crates/api`, `crates/app` | HTTP surface and the composition root |

`docs/product.md` says what it is for. `docs/architecture.md` describes how it fits
together. `docs/adr/` records why, one decision per file, including the ones
that were reversed. `crates/harness/OPTIMISATION-LOG.md` holds every latency
figure that has been measured.

## Measuring it

The eval harness runs a recording through the whole pipeline without LiveKit, a
browser or a client, and prints what each stage cost:

```bash
SONARI_MODELS_DIR=./models cargo run --release -p harness -- recording.wav
```

Latency figures come from release builds only — a debug build inflated one stage
by half again, which is enough to point optimisation at the wrong place.

## Development

The full binary links only on Linux: `libwebrtc` and the speech runtime disagree
about which C runtime and which copy of protobuf to use. Provider-level tests run
natively; everything else goes through the container, which keeps a build cache:

```bash
scripts/dev.sh cargo test --workspace
scripts/dev.sh cargo clippy --workspace --all-targets -- -D warnings
```

## Licence

MIT.
