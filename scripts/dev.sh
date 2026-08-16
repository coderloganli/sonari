#!/usr/bin/env bash
# Runs a cargo command on Linux with a persistent build cache.
#
# The full binary links only on Linux — libwebrtc is built against the dynamic
# CRT and the sherpa-onnx prebuilt against the static one, which cannot be
# reconciled on Windows. Provider-level tests still run natively; everything
# else goes through here.
#
#   scripts/dev.sh cargo test -p app
#   scripts/dev.sh cargo clippy --workspace --all-targets -- -D warnings
set -euo pipefail

cd "$(dirname "$0")/.."
# The sonari service demands real keys; compose insists on resolving every
# service before it will run any of them, and this one only compiles.
export ELEVENLABS_API_KEY="${ELEVENLABS_API_KEY:-unused-by-the-dev-container}"
export LLM_API_KEY="${LLM_API_KEY:-unused-by-the-dev-container}"

exec docker compose --profile dev run --rm dev "$@"
