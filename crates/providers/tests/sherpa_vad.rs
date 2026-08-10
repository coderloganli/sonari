//! Drives voice activity detection over a real recording.
//!
//! Skipped unless `SONARI_MODELS_DIR` points at a directory holding the model,
//! so a clone without it still builds and tests green.

use std::path::{Path, PathBuf};

use providers::{SherpaVad, VadConfig};
use voice::{Vad, VadState};

const SAMPLE_RATE_HZ: u32 = 16_000;
/// 20 ms at 16 kHz — the frame size the pipeline delivers.
const FRAME_SAMPLES: usize = 320;
/// A recording that ships with the recognition model archive.
const SPEECH_WAV: &str = "sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/test_wavs/0.wav";

fn models_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var("SONARI_MODELS_DIR").ok()?);
    dir.is_dir().then_some(dir)
}

fn read_frames(path: &Path) -> Vec<Vec<i16>> {
    let mut reader = hound::WavReader::open(path).expect("open test wav");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, SAMPLE_RATE_HZ, "test wav must be 16 kHz");
    assert_eq!(spec.channels, 1, "test wav must be mono");
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .map(|s| s.expect("sample"))
        .collect();
    samples.chunks(FRAME_SAMPLES).map(<[i16]>::to_vec).collect()
}

fn load(models: &Path) -> SherpaVad {
    SherpaVad::load(&VadConfig {
        model: models
            .join("silero_vad.onnx")
            .to_string_lossy()
            .into_owned(),
        threshold: 0.5,
        min_silence_seconds: 0.25,
        min_speech_seconds: 0.25,
        sample_rate_hz: SAMPLE_RATE_HZ as i32,
        num_threads: 1,
    })
    .expect("load vad model")
}

#[test]
fn speech_is_detected_in_a_spoken_recording() {
    let Some(models) = models_dir() else {
        eprintln!("SONARI_MODELS_DIR is not set; skipping");
        return;
    };
    let mut vad = load(&models);
    let detected = read_frames(&models.join(SPEECH_WAV))
        .into_iter()
        .any(|frame| vad.push(&frame).expect("push frame") == VadState::Speech);
    assert!(detected, "VAD reported no speech in a spoken recording");
}

#[test]
fn silence_is_not_reported_as_speech() {
    // The detector decides when a turn starts and ends, so a false positive on
    // silence would open a turn nobody began.
    let Some(models) = models_dir() else {
        eprintln!("SONARI_MODELS_DIR is not set; skipping");
        return;
    };
    let mut vad = load(&models);
    let silence = vec![0_i16; FRAME_SAMPLES];
    let detected = (0..100).any(|_| vad.push(&silence).expect("push frame") == VadState::Speech);
    assert!(!detected, "VAD reported speech in two seconds of silence");
}
