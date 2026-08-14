//! Driving the running service as a caller.
//!
//! This is the measurement that counts. The in-process solver exercises the
//! components; this exercises the deployed system — the real transport, the real
//! mixer, and, the reason it exists here, the real endpointing under real frame
//! arrival. Frames on a perfect 20 ms clock are not the same input as frames off
//! a WebRTC connection, and endpointing decides from frame arrival.
//!
//! It publishes the clip as a participant's microphone track and listens for the
//! bot to speak. Everything else is read afterwards from the call's own events
//! (see [`super::timeline`]); the solver instruments nothing.
//!
//! `libwebrtc` links only on Linux, so this is behind the `live` feature and
//! runs through `scripts/dev.sh`.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use libwebrtc::{
    audio_source::native::NativeAudioSource,
    prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
};
use livekit::{
    Room, RoomEvent, RoomOptions,
    options::TrackPublishOptions,
    prelude::{LocalAudioTrack, LocalTrack},
};

use crate::{
    generate::audio::SAMPLE_RATE_HZ,
    manifest::Sample,
    solver::{Outcome, Solver, SolverError, api::Api, timeline},
};

/// 20 ms at 16 kHz — the frame size the pipeline delivers.
const FRAME_SAMPLES: usize = 320;

pub struct LiveCallSolver {
    base_url: String,
    /// Where LiveKit is *from here*, when that differs from the address the
    /// service hands to browsers. A stack whose clients connect on
    /// `ws://localhost:7880` is reached from inside the network as
    /// `ws://livekit:7880`, and the eval client is not a browser.
    livekit_url: Option<String>,
    uid: String,
    character_id: i64,
    /// How long to wait after the audio ends for the reply to begin. Generous:
    /// a slow turn is a result, a hung one is a failure, and only the clock
    /// tells them apart.
    reply_timeout: Duration,
}

impl LiveCallSolver {
    pub fn from_environment() -> Result<Self> {
        let base_url = std::env::var("SONARI_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .context("SONARI_BASE_URL must be set to drive a running service")?;
        let character_id = std::env::var("SONARI_CHARACTER_ID")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(1);
        Ok(Self {
            base_url,
            livekit_url: std::env::var("SONARI_LIVEKIT_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            uid: "eval-harness".to_owned(),
            character_id,
            reply_timeout: Duration::from_secs(20),
        })
    }
}

#[async_trait]
impl Solver for LiveCallSolver {
    async fn run(&self, sample: &Sample) -> Result<Outcome, SolverError> {
        self.run_inner(sample)
            .await
            .map_err(|error| SolverError::Failed(format!("{error:#}")))
    }
}

impl LiveCallSolver {
    async fn run_inner(&self, sample: &Sample) -> Result<Outcome> {
        let frames = read_frames(&sample.audio)?;

        let mut api = Api::new(&self.base_url);
        api.create_session(&self.uid).await?;
        let call = api.start_call(self.character_id).await?;

        // Ending the call has to happen whatever the outcome, or the next
        // sample joins a service still holding the last one.
        let result = self.speak_and_listen(&call.realtime, &frames).await;
        let _ = api.end_call(call.session_id).await;
        result?;

        // Events reach PostgreSQL through the outbox, so they are not
        // necessarily written the instant the call ends.
        let events = poll_timeline(&api, call.session_id).await?;
        Ok(timeline::assemble(&events)?)
    }

    /// Joins the room, publishes the clip at the pace it was spoken, and waits
    /// for the bot's track to produce audio.
    async fn speak_and_listen(
        &self,
        realtime: &super::api::Realtime,
        frames: &[Vec<i16>],
    ) -> Result<()> {
        let endpoint = self.livekit_url.as_deref().unwrap_or(&realtime.endpoint);
        let (room, mut room_events) =
            Room::connect(endpoint, &realtime.access_token, RoomOptions::default())
                .await
                .with_context(|| {
                    format!("failed to join room {} at {endpoint}", realtime.room_name)
                })?;
        let room = Arc::new(room);

        let source =
            NativeAudioSource::new(AudioSourceOptions::default(), SAMPLE_RATE_HZ, 1, 1_000);
        let track = LocalAudioTrack::create_audio_track(
            "eval-microphone",
            RtcAudioSource::Native(source.clone()),
        );
        room.local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions::default(),
            )
            .await
            .context("failed to publish the caller's audio track")?;

        // Wait for the bot to be in the room before speaking — and only that.
        //
        // Waiting for the bot's *track* would deadlock: the runtime subscribes
        // to the caller's audio first and publishes its own only afterwards, so
        // a caller who waits to hear the bot before speaking waits forever. The
        // bot's presence is the real precondition; a client cannot observe
        // whether anyone has subscribed to it, and nothing here can. Publishing a
        // track does not mean anyone has subscribed to it, and frames sent
        // before the worker subscribes are simply dropped — which would show up
        // as the clip's opening words missing, indistinguishable from a
        // recognition failure, and worst on exactly the short clips where the
        // opening words are the whole utterance.
        // A track is subscribed once. If the readiness wait sees it, the later
        // wait never will, and treating that as "the bot never spoke" would
        // report a service that answered as one that did not.
        let mut bot_present = room.remote_participants().values().any(|participant| {
            participant.identity().as_str() == realtime.bot_participant_identity
        });
        let ready_by = tokio::time::Instant::now() + Duration::from_secs(15);
        while tokio::time::Instant::now() < ready_by && !bot_present {
            match tokio::time::timeout(Duration::from_millis(250), room_events.recv()).await {
                Ok(Some(RoomEvent::ParticipantConnected(participant)))
                    if participant.identity().as_str() == realtime.bot_participant_identity =>
                {
                    bot_present = true;
                }
                Ok(Some(RoomEvent::TrackSubscribed { participant, .. }))
                    if participant.identity().as_str() == realtime.bot_participant_identity =>
                {
                    bot_present = true;
                }
                _ => {}
            }
        }

        if !bot_present {
            // Speaking to a room nobody is listening in loses the opening words
            // and reports it as a recognition failure. A readiness problem
            // should say it is one.
            anyhow::bail!(
                "the bot never joined room {} within the readiness window; the clip was not sent",
                realtime.room_name
            );
        }

        // At the pace a caller speaks. Reading the file as fast as it comes off
        // disk is not a faster version of this test; it is a different one, in
        // which frames never arrive the way they do on a call and endpointing
        // sees an input no caller produces.
        let frame_period = Duration::from_millis(20);
        let mut next = tokio::time::Instant::now();
        for frame in frames {
            next += frame_period;
            source
                .capture_frame(&AudioFrame {
                    data: frame.as_slice().into(),
                    sample_rate: SAMPLE_RATE_HZ,
                    num_channels: 1,
                    samples_per_channel: frame.len() as u32,
                })
                .await
                .map_err(|error| anyhow!("failed to send a frame: {error}"))?;
            tokio::time::sleep_until(next).await;
        }

        // Hold the room open until the deadline. Leaving early would end the
        // call before the service had finished answering — and the answer, with
        // its markers, is read from the call's own events afterwards, not from
        // anything observed here.
        //
        // Waiting on the event stream alone is not enough: if it yields nothing
        // the loop would fall straight through, the room would close the moment
        // the audio ended, and the call would be over before the service had
        // subscribed to the track, let alone recognised anything. The service
        // needs the caller to stay on the line, exactly as a caller would.
        let deadline = tokio::time::Instant::now() + self.reply_timeout;
        while tokio::time::Instant::now() < deadline {
            // Draining keeps the connection healthy; nothing here decides
            // anything, because the turn summary is published only after
            // synthesis finishes and travels the outbox.
            let _ = tokio::time::timeout(Duration::from_millis(250), room_events.recv()).await;
        }

        room.close().await.ok();
        Ok(())
    }
}

/// Events travel through the outbox, so the timeline can lag the call's end by
/// a moment. Polling briefly is the difference between a real result and a
/// spurious "the turn never completed".
async fn poll_timeline(api: &Api, session_id: i64) -> Result<Vec<timeline::TimelineEvent>> {
    const ATTEMPTS: usize = 20;
    let mut last = Vec::new();
    for _ in 0..ATTEMPTS {
        last = api.timeline(session_id).await?;
        if last
            .iter()
            .any(|entry| entry.event == "speech_turn_latency")
        {
            return Ok(last);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(last)
}

fn read_frames(path: &std::path::Path) -> Result<Vec<Vec<i16>>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE_HZ || spec.channels != 1 {
        anyhow::bail!(
            "{} is {} Hz with {} channels; the pipeline carries {SAMPLE_RATE_HZ} Hz mono",
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
