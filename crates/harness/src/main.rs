//! Drives one turn from a WAV file and reports what it cost.
//!
//! The conversation is exercised end to end — recognition, the model, synthesis
//! — without LiveKit, a browser or a database. That is the point: the numbers
//! come from the stages that determine them, and nothing else has to be running.
//!
//!     sonari-eval recording.wav
//!
//! Timings are only meaningful from a release build. A debug build inflated one
//! stage by half again, which would point optimisation at the wrong place.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use agent::ports::{LlmCompletionRequest, LlmDelta, LlmGateway, LlmRequestMessage};
use anyhow::{Context, Result, bail};
use futures::StreamExt;
use providers::{ElevenLabsAsrEngine, ElevenLabsTtsEngine};
use voice::{AsrEngine, AsrEvent, AsrLanguage, AsrStreamConfig, TtsEngine, TtsRequest};

/// 20 ms at 16 kHz — the frame size the pipeline delivers.
const FRAME_SAMPLES: usize = 320;
const EXPECTED_SAMPLE_RATE: u32 = 16_000;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    // Without this the adapters' warnings vanish, and a stalled upload looks
    // like a mystery instead of a logged one.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let wav = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: sonari-eval <recording.wav>")?;

    let settings_path = sonari_config::config_path();
    let settings = sonari_config::load_and_watch(&settings_path)?.get();
    let models = settings
        .models
        .as_ref()
        .context("no models are configured; nothing to measure")?;
    let persona = settings
        .personas
        .first()
        .context("no persona is configured; nothing to say")?;

    let base_url = std::env::var("LLM_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("LLM_BASE_URL must be set")?;
    if settings.llm.model.trim().is_empty() {
        bail!("llm.model must be set in {}", settings_path.display());
    }

    let api_key = std::env::var("ELEVENLABS_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("ELEVENLABS_API_KEY must be set")?;
    let asr = ElevenLabsAsrEngine::new(models.asr.clone(), api_key.clone())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let tts = ElevenLabsTtsEngine::new(models.tts.clone(), api_key)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let frames = read_frames(&wav)?;
    let speech_duration = Duration::from_secs_f32(
        (frames.len() * FRAME_SAMPLES) as f32 / EXPECTED_SAMPLE_RATE as f32,
    );

    // The clock starts where the caller stops speaking: everything before that
    // is the caller's own time, not the system's.
    let mut stream = asr
        .open(&AsrStreamConfig {
            sample_rate_hz: EXPECTED_SAMPLE_RATE,
            num_channels: 1,
            language: AsrLanguage::parse(&persona.language).unwrap_or(AsrLanguage::En),
        })
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    // Frames go in at the pace a caller speaks them. Reading the file as fast
    // as it comes off disk is not a faster version of the same test — it is a
    // different one, where the upload never gets a chance to keep up and the
    // session dies before it is even connected.
    let frame_period = Duration::from_secs_f32(FRAME_SAMPLES as f32 / EXPECTED_SAMPLE_RATE as f32);
    let mut next = Instant::now();
    for frame in &frames {
        next += frame_period;
        stream
            .push(frame)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        while stream.poll().is_some() {}
        tokio::time::sleep_until(next.into()).await;
    }

    let speech_end = Instant::now();
    stream
        .finish()
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    // The final now comes back over the network rather than from a local
    // decode, so it has to be waited for.
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
    let asr_final = speech_end.elapsed();
    if transcript.trim().is_empty() {
        bail!("recognition produced nothing from {}", wav.display());
    }

    let gateway = agent::adapters::llm::ReqwestLlmGateway::default();
    let mut deltas = gateway
        .stream(LlmCompletionRequest {
            endpoint_url: base_url,
            api_key: std::env::var("LLM_API_KEY").unwrap_or_default(),
            model_name: settings.llm.model.clone(),
            temperature: settings.llm.temperature,
            frequency_penalty: settings.llm.frequency_penalty,
            messages: vec![
                LlmRequestMessage {
                    role: "system".to_owned(),
                    content: system_prompt(&settings, persona),
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
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let mut reply = String::new();
    let mut llm_first_token = None;
    // When the first sentence completes. The gap between this and the whole
    // reply is what splitting the stream into sentences would save.
    let mut llm_first_sentence = None;
    while let Some(delta) = deltas.next().await {
        match delta.map_err(|error| anyhow::anyhow!("{error}"))? {
            LlmDelta::Token(token) => {
                if llm_first_token.is_none() {
                    llm_first_token = Some(speech_end.elapsed());
                }
                reply.push_str(&token);
                if llm_first_sentence.is_none() && ends_a_sentence(&reply) {
                    llm_first_sentence = Some(speech_end.elapsed());
                }
            }
            LlmDelta::ToolCall(_) => bail!("the model asked for a tool; v1 declares none"),
            LlmDelta::Done(_) => {}
        }
    }
    let llm_done = speech_end.elapsed();
    if reply.trim().is_empty() {
        bail!("the model returned nothing");
    }

    let mut audio: voice::TtsAudioStream = tts
        .synthesize(TtsRequest {
            text: reply.clone(),
            voice: persona.voice.clone(),
        })
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut tts_first_chunk = None;
    let mut synthesised_samples = 0_usize;
    let mut sample_rate = 0;
    while let Some(chunk) = audio.next().await {
        let chunk = chunk.map_err(|error| anyhow::anyhow!("{error}"))?;
        if tts_first_chunk.is_none() {
            tts_first_chunk = Some(speech_end.elapsed());
        }
        sample_rate = chunk.sample_rate_hz;
        synthesised_samples += chunk.pcm_s16le.len();
    }
    let audio_first_frame = tts_first_chunk.context("synthesis produced no audio")?;
    let total = speech_end.elapsed();

    // One line, structured, with elapsed values as fields rather than timestamps
    // to subtract. The same shape the running service emits.
    println!(
        "{}",
        serde_json::json!({
            "event": "eval_turn",
            "recording": wav.file_name().and_then(std::ffi::OsStr::to_str),
            "speech_seconds": round(speech_duration.as_secs_f64()),
            "transcript": transcript.trim(),
            "reply": reply.trim(),
            "asr_final_ms": round(asr_final.as_secs_f64() * 1000.0),
            "llm_first_token_ms": llm_first_token.map(elapsed_ms),
            "llm_first_sentence_ms": llm_first_sentence.map(elapsed_ms),
            "llm_done_ms": round(llm_done.as_secs_f64() * 1000.0),
            "tts_first_chunk_ms": elapsed_ms(audio_first_frame),
            // speech_end → audio_first_frame: the figure the target is set against.
            "system_response_ms": elapsed_ms(audio_first_frame),
            "turn_total_ms": round(total.as_secs_f64() * 1000.0),
            "synthesised_seconds": round(synthesised_samples as f64 / sample_rate.max(1) as f64),
        })
    );
    Ok(())
}

/// The same instructions the service assembles: the call rules, then who the
/// agent is. Measuring without them measures a different system.
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
    prompt.push_str(
        "

",
    );
    prompt.push_str(character.trim());
    if let Some(scene) = &persona.scene {
        let scene_text = settings
            .prompts
            .scene
            .replace("{{location}}", &scene.location)
            .replace("{{user_role}}", &scene.user_role)
            .replace("{{relationship}}", &scene.relationship);
        prompt.push_str(
            "

",
        );
        prompt.push_str(scene_text.trim());
    }
    prompt
}

fn elapsed_ms(elapsed: Duration) -> f64 {
    round(elapsed.as_secs_f64() * 1000.0)
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Cheap check for a completed sentence, used only to time when one exists.
fn ends_a_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['.', '!', '?', '。', '！', '？'])
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
