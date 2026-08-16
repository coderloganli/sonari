#!/usr/bin/env bash
# Downloads the one model that runs in this process.
#
# Recognition and synthesis are reached over the network (ADR-0014). Voice
# activity detection is not: it runs on every frame and decides when a turn
# starts and ends, so it cannot be a round trip away.
set -euo pipefail

cd "$(dirname "$0")/.."
models="${SONARI_MODELS_DIR:-models}"
vad="$models/silero_vad.onnx"

if [ -f "$vad" ]; then
  echo "already present: $vad"
  exit 0
fi

mkdir -p "$models"
echo "fetching the voice activity model into $models"
curl --fail --location --progress-bar \
  --output "$vad" \
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"
echo "done: $vad"
