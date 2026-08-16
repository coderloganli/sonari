//! Driving one clip through the components, without the service.
//!
//! Recognition, the model and synthesis, reached directly. No LiveKit, no
//! browser, no database — which is the point: it runs anywhere with an API key,
//! in seconds, so tuning does not require a stack.
//!
//! What it does **not** do is decide when the caller stopped speaking. The whole
//! clip is pushed and then committed, so endpointing is bypassed entirely. Every
//! marker it reports is real, but `speech_end` is the end of the file rather
//! than a decision, and `hangover_cost` is therefore zero. The clips built to
//! probe endpointing say nothing here; they say something under
//! [`super::live_call`].

use std::{
    path::Path,
    time::{Duration, Instant},
};

use agent::ports::{LlmCompletionRequest, LlmDelta, LlmGateway, LlmRequestMessage};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures::StreamExt;
use providers::{ElevenLabsAsrEngine, ElevenLabsTtsEngine};
use voice::{AsrEngine, AsrEvent, AsrLanguage, AsrStreamConfig, TtsEngine, TtsRequest};

use crate::{
    manifest::Sample,
    markers::Markers,
    solver::{Outcome, Solver, SolverError},
};

/// 20 ms at 16 kHz — the frame size the pipeline delivers.
const FRAME_SAMPLES: usize = 320;
const EXPECTED_SAMPLE_RATE: u32 = 16_000;

pub struct SingleTurnSolver {
    asr: ElevenLabsAsrEngine,
    tts: ElevenLabsTtsEngine,
    gateway: agent::adapters::llm::ReqwestLlmGateway,
    settings: std::sync::Arc<sonari_config::Settings>,
    llm_base_url: String,
    llm_api_key: String,
}

impl SingleTurnSolver {
    /// Builds from `sonari.toml` and the environment — the same sources the
    /// service reads, so this measures the configured system rather than a
    /// convenient one.
    pub fn from_environment() -> Result<Self> {
        let settings_path = sonari_config::config_path();
        let settings = sonari_config::load_and_watch(&settings_path)?.get();
        let models = settings
            .models
            .as_ref()
            .context("no models are configured; nothing to measure")?;
        if settings.llm.model.trim().is_empty() {
            bail!("llm.model must be set in {}", settings_path.display());
        }

        let api_key = std::env::var("ELEVENLABS_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("ELEVENLABS_API_KEY must be set")?;
        let llm_base_url = std::env::var("LLM_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("LLM_BASE_URL must be set")?;

        Ok(Self {
            asr: ElevenLabsAsrEngine::new(models.asr.clone(), api_key.clone())
                .map_err(|error| anyhow!("{error}"))?,
            tts: ElevenLabsTtsEngine::new(models.tts.clone(), api_key)
                .map_err(|error| anyhow!("{error}"))?,
            gateway: agent::adapters::llm::ReqwestLlmGateway::default(),
            settings: settings.clone(),
            llm_base_url,
            llm_api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
        })
    }
}

#[async_trait]
impl Solver for SingleTurnSolver {
    async fn run(&self, sample: &Sample) -> Result<Outcome, SolverError> {
        self.run_inner(sample)
            .await
            .map_err(|error| SolverError::Failed(format!("{error:#}")))
    }
}

impl SingleTurnSolver {
    async fn run_inner(&self, sample: &Sample) -> Result<Outcome> {
        let persona = self
            .settings
            .personas
            .first()
            .context("no persona is configured; nothing to say")?;
        let frames = read_frames(&sample.audio)?;

        let started = Instant::now();
        let mut stream = self
            .asr
            .open(&AsrStreamConfig {
                sample_rate_hz: EXPECTED_SAMPLE_RATE,
                num_channels: 1,
                language: AsrLanguage::parse(&persona.language).unwrap_or(AsrLanguage::En),
            })
            .map_err(|error| anyhow!("{error}"))?;

        // At the pace a caller speaks. Reading as fast as the disk allows is a
        // different test, in which the upload never has to keep up.
        let frame_period =
            Duration::from_secs_f32(FRAME_SAMPLES as f32 / EXPECTED_SAMPLE_RATE as f32);
        let mut next = Instant::now();
        for frame in &frames {
            next += frame_period;
            stream.push(frame).map_err(|error| anyhow!("{error}"))?;
            while stream.poll().is_some() {}
            tokio::time::sleep_until(next.into()).await;
        }
        // The commit is unconditional here, which is exactly what makes this
        // solver blind to endpointing.
        let speech_end = started.elapsed();
        stream.finish().map_err(|error| anyhow!("{error}"))?;

        let mut transcript = String::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match stream.poll() {
                Some(AsrEvent::Final { transcript: text }) => {
                    transcript = text;
                    break;
                }
                Some(AsrEvent::Partial { .. }) => {}
                None => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
        let asr_final = started.elapsed();

        // A clip that should produce nothing has produced nothing: a result,
        // not a failure. The same emptiness from a clip that *was* speech is a
        // failure — a stalled recogniser must not read as a quiet room.
        if transcript.trim().is_empty() {
            if !sample.reference.trim().is_empty() {
                bail!(
                    "recognition returned nothing for {}, which was supposed to contain speech",
                    sample.audio.display()
                );
            }
            return Ok(Outcome {
                transcript: String::new(),
                reply: String::new(),
                markers: Markers {
                    speech_start_ms: Some(0.0),
                    speech_last_voiced_ms: Some(ms(speech_end)),
                    speech_end_ms: Some(ms(speech_end)),
                    asr_final_ms: Some(ms(asr_final)),
                    ..Markers::default()
                },
                utterance_count: 0,
                server_turns: 0,
                turn_opened: false,
            });
        }

        let mut deltas = self
            .gateway
            .stream(LlmCompletionRequest {
                endpoint_url: self.llm_base_url.clone(),
                api_key: self.llm_api_key.clone(),
                model_name: self.settings.llm.model.clone(),
                temperature: self.settings.llm.temperature,
                frequency_penalty: self.settings.llm.frequency_penalty,
                messages: vec![
                    LlmRequestMessage {
                        role: "system".to_owned(),
                        content: system_prompt(&self.settings, persona),
                    },
                    LlmRequestMessage {
                        role: "user".to_owned(),
                        content: transcript.clone(),
                    },
                ],
                max_tokens: None,
                tools: Vec::new(),
            })
            .await
            .map_err(|error| anyhow!("{error}"))?;

        let mut reply = String::new();
        let mut first_token = None;
        let mut first_sentence = None;
        while let Some(delta) = deltas.next().await {
            match delta.map_err(|error| anyhow!("{error}"))? {
                LlmDelta::Token(token) => {
                    first_token.get_or_insert_with(|| started.elapsed());
                    reply.push_str(&token);
                    if first_sentence.is_none() && ends_a_sentence(&reply) {
                        first_sentence = Some(started.elapsed());
                    }
                }
                LlmDelta::ToolCall(_) => bail!("the model asked for a tool; none are declared"),
                LlmDelta::Done(_) => {}
            }
        }
        if reply.trim().is_empty() {
            bail!("the model returned nothing");
        }

        let mut audio: voice::TtsAudioStream = self
            .tts
            .synthesize(TtsRequest {
                text: reply.clone(),
                voice: persona.voice.clone(),
            })
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let mut first_chunk = None;
        while let Some(chunk) = audio.next().await {
            chunk.map_err(|error| anyhow!("{error}"))?;
            first_chunk.get_or_insert_with(|| started.elapsed());
        }
        let first_chunk = first_chunk.context("synthesis produced no audio")?;

        Ok(Outcome {
            transcript,
            reply,
            markers: Markers {
                speech_start_ms: Some(0.0),
                // No voice activity detection here, so the last voiced moment is
                // taken as the end of the audio. It makes hangover cost zero,
                // which is the truth: nothing waited.
                speech_last_voiced_ms: Some(ms(speech_end)),
                speech_end_ms: Some(ms(speech_end)),
                asr_final_ms: Some(ms(asr_final)),
                llm_first_token_ms: first_token.map(ms),
                llm_first_sentence_ms: first_sentence.map(ms),
                tts_first_chunk_ms: Some(ms(first_chunk)),
                // Nothing plays it back, so the first chunk out of synthesis is
                // as close to audio leaving as this solver gets.
                audio_first_frame_ms: Some(ms(first_chunk)),
            },
            utterance_count: 1,
            // Nothing here greets or fills a silence; there is no service.
            server_turns: 0,
            turn_opened: true,
        })
    }
}

fn ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

fn ends_a_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['.', '!', '?', '。', '！', '？'])
}

/// The same instructions the service assembles. Measuring without them measures
/// a different system.
fn system_prompt(
    settings: &sonari_config::Settings,
    persona: &sonari_config::PersonaConfig,
) -> String {
    let mut prompt = settings.prompts.conversation_system.trim().to_owned();
    let character = settings
        .prompts
        .character
        .replace("{{name}}", &persona.name)
        .replace("{{persona}}", &persona.prompt.persona)
        .replace("{{personality_traits}}", &persona.prompt.personality_traits)
        .replace("{{speaking_style}}", &persona.prompt.speaking_style);
    prompt.push_str("\n\n");
    prompt.push_str(character.trim());
    if let Some(scene) = &persona.scene {
        let scene_text = settings
            .prompts
            .scene
            .replace("{{location}}", &scene.location)
            .replace("{{user_role}}", &scene.user_role)
            .replace("{{relationship}}", &scene.relationship);
        prompt.push_str("\n\n");
        prompt.push_str(scene_text.trim());
    }
    prompt
}

fn read_frames(path: &Path) -> Result<Vec<Vec<i16>>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != EXPECTED_SAMPLE_RATE || spec.channels != 1 {
        bail!(
            "{} is {} Hz with {} channels; the pipeline carries {EXPECTED_SAMPLE_RATE} Hz mono",
            path.display(),
            spec.sample_rate,
            spec.channels
        );
    }
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .collect::<std::result::Result<_, _>>()
        .context("failed to read samples")?;
    Ok(samples.chunks(FRAME_SAMPLES).map(<[i16]>::to_vec).collect())
}
