//! Streaming recognition at ElevenLabs.
//!
//! Frames go up a WebSocket as they arrive and transcripts come back. The
//! session is opened with `commit_strategy=manual`: the provider offers its own
//! voice-activity segmentation, and we decline it, because the end of a turn has
//! to come from the same signal that drives interruption (ADR-0016).
//!
//! The trait this implements is synchronous — it is called from the blocking
//! pool with a frame and asked for whatever is ready. The socket is therefore
//! driven by a task of its own, with channels between.

use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use shared_kernel::{AppError, AppResult};
use tokio::sync::mpsc;
use voice::{AsrEngine, AsrEvent, AsrStream, AsrStreamConfig};

const REALTIME_URL: &str = "wss://api.elevenlabs.io/v1/speech-to-text/realtime";

/// Frames waiting to go up. Bounded: if the socket cannot keep up, dropping is
/// better than growing without limit, and the counter says it happened.
const OUTBOUND_DEPTH: usize = 64;
/// Transcripts waiting to be polled. Small — they arrive at speaking pace.
const INBOUND_DEPTH: usize = 32;
/// How many times to retry queueing the commit before giving up on the turn.
const COMMIT_ATTEMPTS: usize = 1_000;
/// A call cannot wait longer than this for recognition to become available.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfig {
    /// Recognition model, e.g. `scribe_v2_realtime`.
    pub model: String,
    /// ISO 639-1 or 639-3. Leave unset to let the provider detect it.
    #[serde(default)]
    pub language: Option<String>,
}

pub struct ElevenLabsAsrEngine {
    config: AsrConfig,
    api_key: Arc<String>,
}

impl ElevenLabsAsrEngine {
    /// The key is held by the adapter, not passed through the port: a credential
    /// in a port signature spreads to everything that calls it.
    pub fn new(config: AsrConfig, api_key: String) -> AppResult<Self> {
        if api_key.trim().is_empty() {
            return Err(AppError::invalid_input(
                "ELEVENLABS_API_KEY must be set for recognition",
            ));
        }
        Ok(Self {
            config,
            api_key: Arc::new(api_key),
        })
    }

    fn url(&self, sample_rate_hz: u32) -> String {
        let mut url = format!(
            "{REALTIME_URL}?model_id={}&audio_format=pcm_{sample_rate_hz}&commit_strategy=manual",
            self.config.model
        );
        if let Some(language) = &self.config.language {
            url.push_str("&language_code=");
            url.push_str(language);
        }
        url
    }
}

impl AsrEngine for ElevenLabsAsrEngine {
    fn open(&self, config: &AsrStreamConfig) -> AppResult<Box<dyn AsrStream>> {
        let (outbound, outbound_rx) = mpsc::channel(OUTBOUND_DEPTH);
        let (inbound_tx, inbound) = mpsc::channel(INBOUND_DEPTH);
        let url = self.url(config.sample_rate_hz);
        let api_key = self.api_key.clone();
        let sample_rate_hz = config.sample_rate_hz;

        // The socket lives in its own task. `open` is called from a blocking
        // context and must not wait for a connection to be established, so
        // failures surface on the first poll rather than here.
        tokio::spawn(async move {
            tracing::info!("recognition task started");
            if let Err(error) =
                run_socket(url, api_key, sample_rate_hz, outbound_rx, &inbound_tx).await
            {
                tracing::error!(%error, "recognition session failed");
                let _ = inbound_tx.send(Err(error)).await;
            }
        });

        Ok(Box::new(ElevenLabsAsrStream {
            outbound,
            inbound,
            dropped_frames: 0,
            failure: None,
        }))
    }
}

pub struct ElevenLabsAsrStream {
    outbound: mpsc::Sender<Outbound>,
    inbound: mpsc::Receiver<AppResult<AsrEvent>>,
    /// Frames the socket could not keep up with. Never silently zero.
    dropped_frames: u64,
    /// Set when the socket reports a failure. Polling returns it once rather
    /// than looking like "nothing yet", which would hang the turn until a
    /// timeout somewhere else noticed.
    failure: Option<AppError>,
}

enum Outbound {
    Frame(Vec<i16>),
    Commit,
}

impl AsrStream for ElevenLabsAsrStream {
    fn push(&mut self, frame: &[i16]) -> AppResult<()> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        match self.outbound.try_send(Outbound::Frame(frame.to_vec())) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_frames += 1;
                // Dropping is deliberate: buffering without limit turns a
                // throughput problem into a memory problem, and the audio is
                // stale by the time the queue drains anyway.
                tracing::warn!(
                    dropped_frames = self.dropped_frames,
                    "recognition upload is behind; dropped a frame"
                );
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(AppError::unavailable("recognition session is closed"))
            }
        }
    }

    fn poll(&mut self) -> Option<AsrEvent> {
        match self.inbound.try_recv() {
            Ok(Ok(event)) => Some(event),
            Ok(Err(error)) => {
                // Remembered rather than logged and forgotten: a caller that
                // cannot tell failure from "no result yet" waits forever.
                tracing::error!(%error, "recognition failed");
                self.failure = Some(error);
                None
            }
            Err(_) => None,
        }
    }

    fn finish(&mut self) -> AppResult<()> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        // Unlike a frame, this must not be dropped: losing a frame costs a word,
        // losing the commit means the utterance never ends and the caller waits
        // for a reply that will not come.
        //
        // It also must not block. This is called from the async runtime, and
        // blocking there stalls every other session on the same thread — the
        // first attempt at this used `blocking_send` and panicked outright.
        //
        // So: retry into the queue, which drains at network speed. The bound
        // exists because failing loudly beats spinning forever.
        for _ in 0..COMMIT_ATTEMPTS {
            match self.outbound.try_send(Outbound::Commit) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => std::thread::yield_now(),
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(AppError::unavailable("recognition session is closed"));
                }
            }
        }
        Err(AppError::unavailable(
            "recognition upload is too far behind to end the turn",
        ))
    }
}

#[derive(Serialize)]
struct InputAudioChunk {
    message_type: &'static str,
    audio_base_64: String,
    sample_rate: u32,
    /// True on the last chunk of an utterance. This is the whole point of
    /// `commit_strategy=manual`: the caller decides when the turn ended.
    commit: bool,
}

#[derive(Deserialize)]
struct ServerMessage {
    message_type: String,
    #[serde(default)]
    text: Option<String>,
}

async fn run_socket(
    url: String,
    api_key: Arc<String>,
    sample_rate_hz: u32,
    mut outbound: mpsc::Receiver<Outbound>,
    inbound: &mpsc::Sender<AppResult<AsrEvent>>,
) -> AppResult<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let request = build_request(&url, &api_key)?;
    // A handshake that never completes would otherwise look exactly like an
    // upload that is merely slow: frames pile up and the turn dies with a
    // confusing message instead of the real one.
    let socket = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| AppError::unavailable("recognition session did not open in time"))?
        .map_err(|error| {
            AppError::unavailable(format!("failed to open recognition session: {error}"))
        })?
        .0;
    tracing::info!("recognition session open");
    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            outgoing = outbound.recv() => {
                let Some(outgoing) = outgoing else { break };
                let (samples, commit) = match outgoing {
                    Outbound::Frame(samples) => (samples, false),
                    // A commit with no audio still closes the utterance.
                    Outbound::Commit => (Vec::new(), true),
                };
                let chunk = InputAudioChunk {
                    message_type: "input_audio_chunk",
                    audio_base_64: encode_pcm(&samples),
                    sample_rate: sample_rate_hz,
                    commit,
                };
                let payload = serde_json::to_string(&chunk).map_err(|error| {
                    AppError::internal(format!("failed to encode audio chunk: {error}"))
                })?;
                sink.send(Message::Text(payload.into())).await.map_err(|error| {
                    AppError::unavailable(format!("failed to send audio: {error}"))
                })?;
            }
            incoming = stream.next() => {
                let Some(incoming) = incoming else { break };
                let message = incoming.map_err(|error| {
                    AppError::unavailable(format!("recognition socket failed: {error}"))
                })?;
                let payload = match message {
                    Message::Text(payload) => payload,
                    // A close frame carries the reason the session ended.
                    // Discarding it turns a stated error into an unexplained
                    // disconnect.
                    Message::Close(frame) => {
                        let reason = frame
                            .map(|frame| format!("{}: {}", frame.code, frame.reason))
                            .unwrap_or_else(|| "no reason given".to_owned());
                        tracing::error!(%reason, "recognition session closed by the server");
                        return Err(AppError::unavailable(format!(
                            "recognition session closed by the server ({reason})"
                        )));
                    }
                    other => {
                        tracing::debug!(kind = ?std::mem::discriminant(&other), "ignoring frame");
                        continue;
                    }
                };
                tracing::debug!(%payload, "recognition message");
                let parsed: ServerMessage = match serde_json::from_str(&payload) {
                    Ok(parsed) => parsed,
                    // An unrecognised message is not fatal: the provider may add
                    // types, and a turn should not fail because of one.
                    Err(error) => {
                        tracing::debug!(%error, "ignoring unrecognised recognition message");
                        continue;
                    }
                };
                let event = match parsed.message_type.as_str() {
                    "partial_transcript" => {
                        parsed.text.map(|transcript| AsrEvent::Partial { transcript })
                    }
                    // The reference documents `final_transcript`; the service
                    // sends `committed_transcript` for a manually committed
                    // utterance. Both are accepted, because recognising only the
                    // documented one produced a turn that silently ended with
                    // nothing said.
                    "final_transcript" | "committed_transcript" => {
                        parsed.text.map(|transcript| AsrEvent::Final { transcript })
                    }
                    _ => None,
                };
                if let Some(event) = event
                    && inbound.send(Ok(event)).await.is_err()
                {
                    // Nobody is listening; the call ended.
                    break;
                }
            }
        }
    }
    Ok(())
}

fn build_request(
    url: &str,
    api_key: &str,
) -> AppResult<tokio_tungstenite::tungstenite::handshake::client::Request> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mut request = url
        .into_client_request()
        .map_err(|error| AppError::internal(format!("invalid recognition url: {error}")))?;
    // The key goes in a header rather than the query string so it stays out of
    // logs and proxy access records.
    request.headers_mut().insert(
        "xi-api-key",
        api_key
            .parse()
            .map_err(|_| AppError::invalid_input("ELEVENLABS_API_KEY is not a valid header"))?,
    );
    Ok(request)
}

/// 16-bit little-endian PCM, base64 — the wire format the API expects.
fn encode_pcm(samples: &[i16]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_is_encoded_little_endian() {
        // 0x0102 little-endian is 0x02 0x01; getting this backwards produces
        // audio that is noise, which recognition reports as silence.
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encode_pcm(&[0x0102]))
                .unwrap(),
            vec![0x02, 0x01]
        );
    }

    #[test]
    fn the_session_declines_provider_side_segmentation() {
        let engine = ElevenLabsAsrEngine::new(
            AsrConfig {
                model: "scribe_v2_realtime".to_owned(),
                language: Some("en".to_owned()),
            },
            "key".to_owned(),
        )
        .unwrap();
        let url = engine.url(16_000);
        assert!(
            url.contains("commit_strategy=manual"),
            "endpointing must stay ours: {url}"
        );
        assert!(url.contains("audio_format=pcm_16000"), "{url}");
    }

    #[test]
    fn a_missing_key_is_refused_at_construction() {
        assert!(
            ElevenLabsAsrEngine::new(
                AsrConfig {
                    model: "scribe_v2_realtime".to_owned(),
                    language: None,
                },
                "  ".to_owned(),
            )
            .is_err()
        );
    }
}
