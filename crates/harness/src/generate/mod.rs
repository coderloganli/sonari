//! Building the evaluation set.
//!
//! The clips are synthesised rather than recorded because the set measures
//! timing. A person asked to pause for 400 ms produces something between 300 and
//! 700, and the boundary these clips exist to locate would be lost in that
//! spread. Synthesis also makes the set reproducible: a script in the repository
//! rather than audio files of uncertain provenance.
//!
//! The cost, which the report states: synthetic speech is easier to recognise
//! than human speech, so absolute word error rate is optimistic. What the set is
//! for — where a turn ends — is unaffected.

pub mod audio;
pub mod plan;

use std::path::Path;

use anyhow::{Context, Result};

use crate::generate::{
    audio::{Noise, SAMPLE_RATE_HZ, samples_to_ms},
    plan::{Clip, Shape},
};

/// Turns text into 16 kHz mono PCM.
#[async_trait::async_trait]
pub trait Voice: Send + Sync {
    async fn say(&self, text: &str) -> Result<Vec<i16>>;
}

/// Synthesis through the same provider the service speaks with.
///
/// Using one vendor at both ends may flatter recognition slightly. It does not
/// affect what this set measures — where a turn ends — and the report states
/// that absolute accuracy is optimistic regardless, because synthetic speech is
/// easier than human speech.
pub struct ElevenLabsVoice {
    engine: providers::ElevenLabsTtsEngine,
    voice: String,
}

impl ElevenLabsVoice {
    pub fn from_environment() -> Result<Self> {
        let settings = sonari_config::load_and_watch(&sonari_config::config_path())?.get();
        let models = settings
            .models
            .as_ref()
            .context("no models are configured; nothing to synthesise with")?;
        let api_key = std::env::var("ELEVENLABS_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("ELEVENLABS_API_KEY must be set")?;
        let voice = settings
            .personas
            .first()
            .context("no persona is configured; no voice to speak with")?
            .voice
            .clone();
        Ok(Self {
            engine: providers::ElevenLabsTtsEngine::new(models.tts.clone(), api_key)
                .map_err(|error| anyhow::anyhow!("{error}"))?,
            voice,
        })
    }
}

#[async_trait::async_trait]
impl Voice for ElevenLabsVoice {
    async fn say(&self, text: &str) -> Result<Vec<i16>> {
        use futures::StreamExt;
        use voice::TtsEngine;

        let mut stream = self
            .engine
            .synthesize(voice::TtsRequest {
                text: text.to_owned(),
                voice: self.voice.clone(),
            })
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let mut samples = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| anyhow::anyhow!("{error}"))?;
            if chunk.sample_rate_hz != SAMPLE_RATE_HZ {
                anyhow::bail!(
                    "synthesis returned {} Hz; the set is built at {SAMPLE_RATE_HZ} Hz",
                    chunk.sample_rate_hz
                );
            }
            samples.extend_from_slice(&chunk.pcm_s16le);
        }
        Ok(samples)
    }
}

/// Builds every clip and the manifest that describes them.
pub async fn build(voice: &dyn Voice, out_dir: &Path) -> Result<String> {
    let clips_dir = out_dir.join("clips");
    std::fs::create_dir_all(&clips_dir)
        .with_context(|| format!("failed to create {}", clips_dir.display()))?;

    let mut manifest = String::new();
    for (index, clip) in plan::clips().into_iter().enumerate() {
        // Seeded per clip by position, so regenerating one clip does not shift
        // the noise in every other.
        let mut noise = Noise::new(index as u32 + 1);
        let (samples, probe) = assemble(voice, &clip, &mut noise).await?;
        let path = clips_dir.join(format!("{}.wav", clip.id));
        write_wav(&path, &samples, clip.channels(), clip.sample_rate_hz())?;

        manifest.push_str(
            &serde_json::json!({
                "id": clip.id,
                "audio": format!("clips/{}.wav", clip.id),
                "reference": clip.reference,
                "probe": probe,
            })
            .to_string(),
        );
        manifest.push('\n');
    }
    Ok(manifest)
}

/// Assembles one clip, returning the audio and the landmarks that ended up in
/// it. The landmarks are measured, never assumed: segment lengths depend on the
/// synthesiser, and a probe that lied about where the gap started would make
/// every reading of that row wrong.
async fn assemble(
    voice: &dyn Voice,
    clip: &Clip,
    noise: &mut Noise,
) -> Result<(Vec<i16>, serde_json::Value)> {
    const LEAD_IN_MS: u32 = 500;
    // Comfortably longer than `silence_flush_ms`, so a turn that ends when the
    // audio runs out can be told from one that ends because silence was
    // detected.
    const TAIL_MS: u32 = 1_500;

    let mut samples = noise.floor(LEAD_IN_MS);
    let probe;

    match &clip.shape {
        Shape::Pause {
            first,
            gap_ms,
            second,
        } => {
            samples.extend_from_slice(audio::trim(&voice.say(first).await?));
            let gap_start_ms = samples_to_ms(samples.len());
            samples.extend(noise.floor(*gap_ms));
            samples.extend_from_slice(audio::trim(&voice.say(second).await?));
            samples.extend(noise.floor(TAIL_MS));
            probe = serde_json::json!({
                "kind": "pause",
                "gap_start_ms": gap_start_ms,
                "gap_ms": gap_ms,
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Short { word } => {
            let spoken = voice.say(word).await?;
            let trimmed = audio::trim(&spoken);
            let speech_ms = samples_to_ms(trimmed.len());
            samples.extend_from_slice(trimmed);
            samples.extend(noise.floor(TAIL_MS));
            probe = serde_json::json!({
                "kind": "short",
                "speech_ms": speech_ms,
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Decay {
            sentence,
            fade_ms,
            floor_dbfs,
        } => {
            let spoken = voice.say(sentence).await?;
            let mut trimmed = audio::trim(&spoken).to_vec();
            audio::apply_decay(&mut trimmed, *fade_ms, *floor_dbfs);
            let decay_start_ms = samples_to_ms(
                samples.len() + trimmed.len().saturating_sub(audio::ms_to_samples(*fade_ms)),
            );
            samples.extend_from_slice(&trimmed);
            samples.extend(noise.floor(TAIL_MS));
            probe = serde_json::json!({
                "kind": "decay",
                "decay_start_ms": decay_start_ms,
                "floor_dbfs": floor_dbfs,
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Plain { sentence } => {
            samples.extend_from_slice(audio::trim(&voice.say(sentence).await?));
            let speech_end_ms = samples_to_ms(samples.len());
            samples.extend(noise.floor(TAIL_MS));
            probe = serde_json::json!({
                "kind": "baseline",
                "speech_end_ms": speech_end_ms,
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Idle { ms } => {
            samples = noise.floor(*ms);
            probe = serde_json::json!({
                "kind": "idle",
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Silence { ms } => {
            samples = noise.floor(*ms);
            probe = serde_json::json!({
                "kind": "nothing",
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Burst { ms } => {
            let burst_start_ms = samples_to_ms(samples.len());
            samples.extend(noise.burst(*ms));
            samples.extend(noise.floor(TAIL_MS));
            probe = serde_json::json!({
                "kind": "nothing",
                "burst_start_ms": burst_start_ms,
                "burst_ms": ms,
                "audio_ms": samples_to_ms(samples.len()),
            });
        }
        Shape::Malformed { sentence } => {
            // Deliberately wrong: 8 kHz, two channels. Something has to exercise
            // the rejection path, and it has to be a real file.
            let spoken = voice.say(sentence).await?;
            let trimmed = audio::trim(&spoken);
            samples = trimmed
                .chunks(2)
                .flat_map(|pair| {
                    let averaged =
                        pair.iter().map(|s| i32::from(*s)).sum::<i32>() / pair.len() as i32;
                    [averaged as i16, averaged as i16]
                })
                .collect();
            probe = serde_json::json!({
                "kind": "malformed",
                "sample_rate_hz": 8_000,
                "channels": 2,
            });
        }
    }

    Ok((samples, probe))
}

fn write_wav(path: &Path, samples: &[i16], channels: u16, sample_rate_hz: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: sample_rate_hz,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("failed to create {}", path.display()))?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize().context("failed to finalise the wav")?;
    Ok(())
}

impl Clip {
    fn channels(&self) -> u16 {
        match self.shape {
            Shape::Malformed { .. } => 2,
            _ => 1,
        }
    }

    fn sample_rate_hz(&self) -> u32 {
        match self.shape {
            Shape::Malformed { .. } => 8_000,
            _ => SAMPLE_RATE_HZ,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthesiser that returns a fixed tone per word, with padding, so
    /// assembly can be tested without the network.
    struct FixedVoice;

    #[async_trait::async_trait]
    impl Voice for FixedVoice {
        async fn say(&self, text: &str) -> Result<Vec<i16>> {
            let mut noise = Noise::new(99);
            // 100 ms of padding either side, as a real synthesiser returns.
            let mut out = noise.floor(100);
            out.extend(std::iter::repeat_n(
                8_000_i16,
                audio::ms_to_samples(60 * text.split_whitespace().count() as u32),
            ));
            out.extend(noise.floor(100));
            Ok(out)
        }
    }

    /// Spec 58 and 59 together, at the level that matters: the gap in the
    /// finished clip is the gap the plan asked for, padding notwithstanding.
    #[tokio::test]
    async fn an_assembled_pause_has_the_gap_it_claims() {
        let clip = Clip {
            id: "pause-600".to_owned(),
            reference: "one two three four".to_owned(),
            shape: Shape::Pause {
                first: "one two".to_owned(),
                gap_ms: 600,
                second: "three four".to_owned(),
            },
        };

        let (samples, probe) = assemble(&FixedVoice, &clip, &mut Noise::new(1))
            .await
            .expect("assembled");

        let gap_start = probe["gap_start_ms"].as_u64().expect("a gap start");
        assert_eq!(gap_start, 620, "500 ms lead-in plus two trimmed words");
        assert_eq!(probe["gap_ms"], 600);
        // Lead-in 500 + 120 speech + 600 gap + 120 speech + 1500 tail.
        assert_eq!(samples_to_ms(samples.len()), 2_840);
        assert_eq!(probe["audio_ms"], 2_840);
    }

    /// Spec 62.
    #[tokio::test]
    async fn a_generated_clip_is_16_khz_mono() {
        let clip = Clip {
            id: "baseline".to_owned(),
            reference: "hello there".to_owned(),
            shape: Shape::Plain {
                sentence: "hello there".to_owned(),
            },
        };

        assert_eq!(clip.channels(), 1);
        assert_eq!(clip.sample_rate_hz(), 16_000);

        let (samples, probe) = assemble(&FixedVoice, &clip, &mut Noise::new(1))
            .await
            .expect("assembled");
        assert_eq!(probe["kind"], "baseline");
        assert_eq!(probe["speech_end_ms"], 620);
        assert!(!samples.is_empty());
    }

    /// Spec 63. The rejection path needs something real to reject.
    #[tokio::test]
    async fn the_malformed_fixture_is_wrong_in_the_way_it_claims() {
        let clip = Clip {
            id: "edge-8khz-stereo".to_owned(),
            reference: "hello there".to_owned(),
            shape: Shape::Malformed {
                sentence: "hello there".to_owned(),
            },
        };

        assert_eq!(clip.channels(), 2);
        assert_eq!(clip.sample_rate_hz(), 8_000);

        let (samples, probe) = assemble(&FixedVoice, &clip, &mut Noise::new(1))
            .await
            .expect("assembled");
        assert_eq!(probe["sample_rate_hz"], 8_000);
        assert_eq!(samples.len() % 2, 0, "interleaved stereo");
    }

    /// The whole set, without a network. The count is asserted so that adding a
    /// clip is a deliberate act: the set is the instrument, and it changing
    /// quietly would make two reports incomparable without saying so.
    #[tokio::test]
    async fn the_plan_covers_sixteen_clips() {
        assert_eq!(plan::clips().len(), 16);
    }
}
