//! Voice activity detection on sherpa-onnx.
//!
//! The only model that stays in-process. It runs on every frame and decides both
//! when a turn starts and when it ends, so a round trip is out of the question
//! (ADR-0014, ADR-0016).
//!
//! The pipeline carries 16-bit PCM; sherpa-onnx takes normalised floats, so
//! frames are converted on the way in.

use serde::Deserialize;
use shared_kernel::{AppError, AppResult};
use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use voice::{Vad, VadState};

/// Full scale for 16-bit PCM. Dividing by this maps the range onto `[-1, 1)`.
const I16_FULL_SCALE: f32 = 32_768.0;

fn to_f32(frame: &[i16]) -> Vec<f32> {
    frame
        .iter()
        .map(|sample| f32::from(*sample) / I16_FULL_SCALE)
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct VadConfig {
    /// Path to the Silero VAD ONNX model.
    pub model: String,
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    #[serde(default = "default_min_silence_seconds")]
    pub min_silence_seconds: f32,
    #[serde(default = "default_min_speech_seconds")]
    pub min_speech_seconds: f32,
    #[serde(default = "default_sample_rate")]
    pub sample_rate_hz: i32,
    #[serde(default = "default_num_threads")]
    pub num_threads: i32,
}

fn default_vad_threshold() -> f32 {
    0.5
}
fn default_min_silence_seconds() -> f32 {
    0.25
}
fn default_min_speech_seconds() -> f32 {
    0.25
}
fn default_sample_rate() -> i32 {
    16_000
}
fn default_num_threads() -> i32 {
    1
}

/// How much audio the detector may buffer internally.
const VAD_BUFFER_SECONDS: f32 = 30.0;

pub struct SherpaVad {
    detector: VoiceActivityDetector,
}

impl SherpaVad {
    pub fn load(config: &VadConfig) -> AppResult<Self> {
        let model_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(config.model.clone()),
                threshold: config.threshold,
                min_silence_duration: config.min_silence_seconds,
                min_speech_duration: config.min_speech_seconds,
                ..SileroVadModelConfig::default()
            },
            sample_rate: config.sample_rate_hz,
            num_threads: config.num_threads,
            ..VadModelConfig::default()
        };
        let detector = VoiceActivityDetector::create(&model_config, VAD_BUFFER_SECONDS)
            .ok_or_else(|| {
                AppError::internal(format!("failed to load VAD model: {}", config.model))
            })?;
        Ok(Self { detector })
    }
}

impl Vad for SherpaVad {
    fn push(&mut self, frame: &[i16]) -> AppResult<VadState> {
        self.detector.accept_waveform(&to_f32(frame));
        Ok(if self.detector.detected() {
            VadState::Speech
        } else {
            VadState::Silence
        })
    }

    fn reset(&mut self) {
        self.detector.reset();
    }
}
