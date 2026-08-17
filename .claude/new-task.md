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

`--live` needs the stack up, and four variables. In full:

```bash
set -a; . ./.env; set +a
docker compose up -d                       # sonari, postgres, livekit

# Compose network names, not localhost: the dev container is on that network.
export SONARI_BASE_URL=http://sonari:8080
export SONARI_LIVEKIT_URL=ws://livekit:7880
export SONARI_CHARACTER_ID=$(curl -s localhost:8080/api/personas | jq -r '.data[0].id')

# Every marker the live solver reports comes from GET
# /api/admin/call-logs/{id}/timeline, which requires an admin token. Nothing
# issues one — POST /api/session issues `user` — so the run mints it, signed
# with JWT_SECRET (default `dev-secret`, crates/app/src/config.rs). `sub` must be
# the literal "access": validate_access_token rejects every other value
# (crates/auth/adapters/jwt.rs).
export SONARI_ADMIN_TOKEN=$(python - <<'TOKEN'
import base64, hashlib, hmac, json, time
def b64(raw): return base64.urlsafe_b64encode(raw).rstrip(b"=")
now = int(time.time())
head = b64(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
body = b64(json.dumps({"sub": "access", "user_id": 1, "role": "admin",
                       "perms": [], "iat": now, "exp": now + 3600}).encode())
sig = b64(hmac.new(b"dev-secret", head + b"." + body, hashlib.sha256).digest())
print((head + b"." + body + b"." + sig).decode())
TOKEN
)

scripts/dev.sh cargo run --release -p harness --features live -- \
    run evals/set.jsonl --live --epochs 3 --out evals/runs-live
```

The token recipe is read off `crates/auth/adapters/jwt.rs` and
`crates/app/src/config.rs` and has not been exercised against a running stack, so
if the admin surface answers 401, start with the claims: `sub` and `role` are both
checked, and the secret has to be the one the service booted with.

`--out` matters: the default is `evals/runs`, where the component-level runs go.
Timings mean nothing from a debug build — the run records which build it was, and
`scripts/check-published-figures.sh` will not let a figure into a document that
is not in the newest run under `evals/runs-live/`.
