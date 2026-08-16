# Project notes for /new-task

Facts a task needs that the repository cannot tell you.

## A fresh worktree needs the local config copied in

Credentials and local configuration are gitignored, so `git worktree add` does
not bring them. They live in the main checkout. First thing after creating a
worktree:

```bash
cp ../main/.env ../main/sonari.toml .
mkdir -p models && cp ../main/models/silero_vad.onnx models/
```

| File | Holds | Tracked template |
|---|---|---|
| `.env` | Provider keys, database DSN, LiveKit endpoint and secret | `.env.example` |
| `sonari.toml` | Personas, prompts, endpointing parameters | `sonari.toml.example` |
| `models/silero_vad.onnx` | The one model that runs in this process | `scripts/fetch-models.sh` |

Without them the failure is misleading: the harness reports
`ELEVENLABS_API_KEY must be set` or `models.vad.model points at a file that does
not exist`, which reads as "no credentials exist" rather than "they are one
directory up".

Load them into the environment before running anything that talks to a provider:

```bash
set -a; . ./.env; set +a
```

## Commands

| | Command |
|---|---|
| Tests, native | `cargo test -p harness -p speech-runtime -p agent` |
| Everything, including what links only on Linux | `scripts/dev.sh cargo test --workspace` |
| Lint as CI does | `scripts/dev.sh cargo clippy --workspace --all-targets -- -D warnings` |
| The full stack | `docker compose up -d` |

`app` and anything pulling in `libwebrtc` link only on Linux
(`docs/architecture.md` §10), so they go through `scripts/dev.sh`. Provider-level
crates, `speech-runtime`, `agent` and `harness` build natively on Windows, which
is the fast loop.

## Evaluation harness

```bash
cargo run --release -p harness -- generate                       # build the clip set
cargo run --release -p harness -- run evals/set.jsonl --epochs 3 # components
scripts/dev.sh cargo run --release -p harness --features live -- \
    run evals/set.jsonl --live                                   # the running service
```

`--live` needs the stack up and `SONARI_BASE_URL` set. Timings mean nothing from
a debug build.
