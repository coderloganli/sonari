# ADR-0005: Run VAD, ASR and TTS in-process via sherpa-onnx

- **Status**: Accepted
- **Date**: 2026-08-05
- **Tags**: `providers`, `audio`, `process`
- **Related**: ADR-0003, ADR-0006, ADR-0008

## Context

ADR-0003 requires every PCM-handling component to live inside the Sonari process. That is a requirement on placement; this record covers how it is satisfied.

`sherpa-onnx` publishes Rust bindings (crate `sherpa-onnx`, v1.13.4, 2026-07-08) exposing `VoiceActivityDetector`, `OnlineRecognizer` (streaming recognition with partial results), and `OfflineTts` — all three needed capabilities behind one dependency. It links statically, with a build script that fetches a matching prebuilt native archive when `SHERPA_ONNX_LIB_DIR` is unset. The streaming recognizer runs INT8 ONNX models (~650 MB) fast enough on CPU, leaving the GPU entirely to the LLM.

`OfflineTts` returns a complete buffer rather than a stream. This is acceptable because synthesis is driven per sentence (ADR-0009), not per utterance: each call produces one to two seconds of audio, and the reported first-audio latency of ~40 ms at a real-time factor of 0.03 means later sentences finish well inside the playback of earlier ones.

An earlier draft of this decision placed the engines in a sibling container reached over WebSocket, on the grounds that it keeps the build pure Rust and lets Docker supply restart and health-checking. That reasoning inverted the priority: build convenience is a one-time cost, while the boundary is a permanent constraint. It was also imprecise — it treated ASR, TTS and LLM as one category when their call rates differ by two orders of magnitude.

## Decision

Link `sherpa-onnx` into the Sonari binary and run VAD, ASR, and TTS in-process. Access them through the provider traits defined in `sonari-core` so the implementation can be replaced without touching the pipeline.

## Consequences

- No audio crosses a process boundary, so ADR-0003 holds without exception or caveat.
- One dependency supplies all three capabilities; the deployment stays at four containers.
- The build must link ONNX Runtime, a C++ library. Cross-platform build friction is expected and is a one-time cost.
- Model files (~650 MB) must ship in the image or be fetched at startup. The image grows accordingly.
- **A segmentation fault in the native runtime terminates the whole process.** Rust's guarantees do not extend across FFI. Container restart plus the text-only path (ADR-0008) is the mitigation; there is no in-process recovery.

## Alternatives considered

| Alternative | Why not |
|---|---|
| ASR/TTS as a sibling container | Violates ADR-0003 for the ASR path specifically (~50 crossings/sec). Retained as the fallback if build or stability cost proves untenable — the provider traits make the swap a single-file change |
| Isolate the native library in a child process managed by Sonari | Buys crash isolation while keeping one container, but reimplements supervision, restart backoff, and health checking that Docker already provides — and still crosses a boundary per frame |
| Whisper for ASR | Not a streaming architecture. Chunking it degrades both latency and accuracy, and would remove a pillar of the sub-2s claim rather than support it |
