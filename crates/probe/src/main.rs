//! A caller that is not a person.
//!
//! Joins a call the way a client does — over WebRTC, publishing a microphone
//! track and subscribing to the reply — but the microphone is a WAV file and
//! the ear is a counter. It exercises everything the eval harness deliberately
//! skips: LiveKit in both directions, the turn state machine, endpointing and
//! playback.
//!
//!     sonari-probe recording.wav
//!
//! Environment: `SONARI_URL` (default `http://localhost:8080`), `SONARI_UID`,
//! `SONARI_PERSONA`, and `SONARI_LIVEKIT_URL` to override the address the
//! service advertises.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use libwebrtc::{
    audio_source::native::NativeAudioSource,
    prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
};
use livekit::{
    options::TrackPublishOptions,
    prelude::{LocalAudioTrack, LocalTrack, RemoteTrack, Room, RoomEvent, RoomOptions},
};
use serde::Deserialize;
use tokio_stream::StreamExt;

/// The pipeline's rate and frame size: 20 ms at 16 kHz.
const SAMPLE_RATE_HZ: u32 = 16_000;
const FRAME_SAMPLES: usize = 320;
/// How long to wait for the agent to say anything after the utterance ends.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,livekit=warn,libwebrtc=warn".into()),
        )
        .init();

    let wav = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .context("usage: sonari-probe <recording.wav>")?;
    let frames = read_frames(&wav)?;

    let base = std::env::var("SONARI_URL").unwrap_or_else(|_| "http://localhost:8080".to_owned());
    let uid = std::env::var("SONARI_UID").unwrap_or_else(|_| "probe-caller".to_owned());
    let persona = std::env::var("SONARI_PERSONA").unwrap_or_else(|_| "companion".to_owned());

    let http = reqwest::Client::new();
    let token = create_session(&http, &base, &uid).await?;
    let call = start_call(&http, &base, &token, &persona).await?;
    tracing::info!(room = %call.room_name, "call started");

    let (room, mut events) =
        Room::connect(&call.endpoint, &call.access_token, RoomOptions::default())
            .await
            .context("failed to join the room")?;
    let room = Arc::new(room);

    // The agent waits for a caller's track before it will do anything.
    let source = NativeAudioSource::new(AudioSourceOptions::default(), SAMPLE_RATE_HZ, 1, 1000);
    let track =
        LocalAudioTrack::create_audio_track("caller-audio", RtcAudioSource::Native(source.clone()));
    room.local_participant()
        .publish_track(
            LocalTrack::Audio(track.clone()),
            TrackPublishOptions::default(),
        )
        .await
        .context("failed to publish the caller track")?;
    tracing::info!("caller track published");

    // Reply audio is counted on its own task: it starts arriving while the
    // utterance is still being spoken if the agent interrupts.
    let (reply_tx, mut reply_rx) = tokio::sync::mpsc::channel::<usize>(64);
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let RoomEvent::TrackSubscribed {
                track: RemoteTrack::Audio(audio),
                ..
            } = event
            {
                tracing::info!("subscribed to the agent's track");
                let mut stream = libwebrtc::audio_stream::native::NativeAudioStream::new(
                    audio.rtc_track(),
                    SAMPLE_RATE_HZ as i32,
                    1,
                );
                while let Some(frame) = stream.next().await {
                    if reply_tx.send(frame.data.len()).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    // Speak at the pace a caller speaks. Pushing faster is a different test:
    // endpointing is decided from silence, and silence needs elapsed time.
    let frame_period = Duration::from_secs_f32(FRAME_SAMPLES as f32 / SAMPLE_RATE_HZ as f32);
    let mut next = Instant::now();
    for frame in &frames {
        next += frame_period;
        source
            .capture_frame(&AudioFrame {
                data: frame.as_slice().into(),
                sample_rate: SAMPLE_RATE_HZ,
                num_channels: 1,
                samples_per_channel: FRAME_SAMPLES as u32,
            })
            .await
            .context("failed to send a frame")?;
        tokio::time::sleep_until(next.into()).await;
    }
    let spoke_until = Instant::now();
    tracing::info!("utterance finished; waiting for a reply");

    // Silence after the utterance, so endpointing fires the way it would if the
    // caller had simply stopped talking.
    let silence = vec![0_i16; FRAME_SAMPLES];
    let mut first_reply_at = None;
    let mut reply_samples = 0_usize;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    while Instant::now() < deadline {
        next += frame_period;
        source
            .capture_frame(&AudioFrame {
                data: silence.as_slice().into(),
                sample_rate: SAMPLE_RATE_HZ,
                num_channels: 1,
                samples_per_channel: FRAME_SAMPLES as u32,
            })
            .await
            .ok();
        while let Ok(samples) = reply_rx.try_recv() {
            if first_reply_at.is_none() {
                first_reply_at = Some(spoke_until.elapsed());
            }
            reply_samples += samples;
        }
        // Stop once the reply has clearly finished.
        if first_reply_at.is_some_and(|at| spoke_until.elapsed() > at + Duration::from_secs(3)) {
            break;
        }
        tokio::time::sleep_until(next.into()).await;
    }

    room.close().await.ok();

    let Some(first_reply_at) = first_reply_at else {
        bail!("the agent never spoke: no audio arrived within {REPLY_TIMEOUT:?}");
    };
    println!(
        "{}",
        serde_json::json!({
            "event": "probe_call",
            "recording": wav.file_name().and_then(std::ffi::OsStr::to_str),
            "room": call.room_name,
            // speech end → first audio frame received over WebRTC. Unlike the
            // harness's figure, this includes transport in both directions.
            "perceived_response_ms": (first_reply_at.as_secs_f64() * 1000.0).round(),
            "reply_seconds": (reply_samples as f64 / SAMPLE_RATE_HZ as f64 * 10.0).round() / 10.0,
        })
    );
    Ok(())
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct SessionData {
    access_token: String,
}

#[derive(Deserialize)]
struct CallData {
    realtime: Realtime,
}

#[derive(Deserialize)]
struct Realtime {
    endpoint: String,
    room_name: String,
    access_token: String,
}

struct Call {
    endpoint: String,
    room_name: String,
    access_token: String,
}

async fn create_session(http: &reqwest::Client, base: &str, uid: &str) -> Result<String> {
    let response: Envelope<SessionData> = http
        .post(format!("{base}/api/session"))
        .json(&serde_json::json!({ "uid": uid }))
        .send()
        .await
        .context("failed to create a session")?
        .error_for_status()
        .context("session creation was refused")?
        .json()
        .await
        .context("failed to decode the session response")?;
    Ok(response.data.access_token)
}

async fn start_call(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    persona: &str,
) -> Result<Call> {
    // The id is derived from the persona's name, the same way the service does
    // it, so the probe needs nothing but the name from configuration.
    let character_id = persona_id(persona);
    let response: Envelope<CallData> = http
        .post(format!("{base}/api/call/{character_id}/start"))
        .bearer_auth(token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .context("failed to start a call")?
        .error_for_status()
        .context("the call was refused")?
        .json()
        .await
        .context("failed to decode the call response")?;
    Ok(Call {
        // The service hands out the address it advertises to clients, which is
        // where a browser on the host would find LiveKit. A probe inside the
        // compose network reaches it by service name instead.
        endpoint: std::env::var("SONARI_LIVEKIT_URL").unwrap_or(response.data.realtime.endpoint),
        room_name: response.data.realtime.room_name,
        access_token: response.data.realtime.access_token,
    })
}

fn persona_id(name: &str) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(name.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    (i64::from_be_bytes(bytes) & i64::MAX).max(1)
}

fn read_frames(path: &Path) -> Result<Vec<Vec<i16>>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE_HZ || spec.channels != 1 {
        bail!(
            "{} is {} Hz with {} channels; a caller speaks {SAMPLE_RATE_HZ} Hz mono",
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
