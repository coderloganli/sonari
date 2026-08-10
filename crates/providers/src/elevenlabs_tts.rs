//! Streaming synthesis at ElevenLabs.
//!
//! One request per utterance, audio chunked back as it is produced. PCM is
//! requested at the pipeline's own rate so nothing has to be resampled and no
//! decoder sits in the audio path.

use serde::{Deserialize, Serialize};
use shared_kernel::{AppError, AppResult};
use voice::{TtsAudioChunk, TtsAudioStream, TtsEngine, TtsRequest};

const API_ROOT: &str = "https://api.elevenlabs.io/v1/text-to-speech";

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfig {
    /// Synthesis model, e.g. `eleven_flash_v2_5`.
    pub model: String,
    /// The rate the pipeline carries. Requested directly so no resampling or
    /// decoding happens between the provider and playback.
    #[serde(default = "default_sample_rate")]
    pub sample_rate_hz: u32,
}

fn default_sample_rate() -> u32 {
    16_000
}

pub struct ElevenLabsTtsEngine {
    client: reqwest::Client,
    config: TtsConfig,
    api_key: String,
}

impl ElevenLabsTtsEngine {
    pub fn new(config: TtsConfig, api_key: String) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::invalid_input(
                "ELEVENLABS_API_KEY must be set for synthesis",
            ));
        }
        Ok(Self {
            client: reqwest::Client::new(),
            config,
            api_key,
        })
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.config.sample_rate_hz
    }
}

#[derive(Serialize)]
struct SynthesisRequest<'a> {
    text: &'a str,
    model_id: &'a str,
}

#[async_trait::async_trait]
impl TtsEngine for ElevenLabsTtsEngine {
    async fn synthesize(&self, request: TtsRequest) -> AppResult<TtsAudioStream> {
        use futures::StreamExt;

        // The voice is an ElevenLabs voice id, named per persona.
        let url = format!(
            "{API_ROOT}/{}/stream?output_format=pcm_{}",
            request.voice, self.config.sample_rate_hz
        );
        let response = self
            .client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .json(&SynthesisRequest {
                text: &request.text,
                model_id: &self.config.model,
            })
            .send()
            .await
            .map_err(|error| AppError::unavailable(format!("synthesis request failed: {error}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable response body>"));
            return Err(AppError::unavailable(format!(
                "synthesis failed with status {status}: {body}"
            )));
        }

        let sample_rate_hz = self.config.sample_rate_hz;
        // Network reads do not land on sample boundaries: a chunk can end
        // mid-sample, and pairing the halves wrongly is white noise.
        let mut carry: Option<u8> = None;
        let stream = response.bytes_stream().map(move |chunk| {
            let bytes = chunk.map_err(|error| {
                AppError::unavailable(format!("synthesis stream failed: {error}"))
            })?;
            let mut pcm = Vec::with_capacity(bytes.len() / 2 + 1);
            let mut iterator = bytes.iter().copied();
            if let Some(low) = carry.take()
                && let Some(high) = iterator.next()
            {
                pcm.push(i16::from_le_bytes([low, high]));
            }
            while let Some(low) = iterator.next() {
                match iterator.next() {
                    Some(high) => pcm.push(i16::from_le_bytes([low, high])),
                    None => {
                        carry = Some(low);
                        break;
                    }
                }
            }
            Ok(TtsAudioChunk {
                pcm_s16le: pcm,
                sample_rate_hz,
                channels: 1,
            })
        });
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_key_is_refused_at_construction() {
        assert!(
            ElevenLabsTtsEngine::new(
                TtsConfig {
                    model: "eleven_flash_v2_5".to_owned(),
                    sample_rate_hz: 16_000,
                },
                String::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn the_pipeline_rate_is_requested_directly() {
        let engine = ElevenLabsTtsEngine::new(
            TtsConfig {
                model: "eleven_flash_v2_5".to_owned(),
                sample_rate_hz: 16_000,
            },
            "key".to_owned(),
        )
        .unwrap();
        // Asking for the rate we carry is what removes the resampler.
        assert_eq!(engine.sample_rate_hz(), 16_000);
    }
}
