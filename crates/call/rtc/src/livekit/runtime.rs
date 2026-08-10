use std::{sync::Arc, time::Duration};

use livekit::prelude::{RemoteAudioTrack, RemoteTrack, Room, RoomEvent, RoomOptions};
use shared_kernel::{AppError, AppResult};
use tokio::{sync::mpsc::UnboundedReceiver, time::timeout};

use super::input::{UserAudioInput, build_user_audio_input};
use super::output::BotAudioOutputSink;
use super::types::{LiveKitParticipantIdentity, LiveKitRoomName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitRuntimeEndpoint(String);

impl LiveKitRuntimeEndpoint {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitAccessToken(String);

impl LiveKitAccessToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitParticipantBinding {
    local_participant_identity: LiveKitParticipantIdentity,
    expected_remote_participant_identity: LiveKitParticipantIdentity,
}

impl LiveKitParticipantBinding {
    pub fn new(
        local_participant_identity: LiveKitParticipantIdentity,
        expected_remote_participant_identity: LiveKitParticipantIdentity,
    ) -> Self {
        Self {
            local_participant_identity,
            expected_remote_participant_identity,
        }
    }

    pub fn local_participant_identity(&self) -> &LiveKitParticipantIdentity {
        &self.local_participant_identity
    }

    pub fn expected_remote_participant_identity(&self) -> &LiveKitParticipantIdentity {
        &self.expected_remote_participant_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveKitRuntimeConfig {
    endpoint: LiveKitRuntimeEndpoint,
    expected_room_name: LiveKitRoomName,
    participant_binding: LiveKitParticipantBinding,
    access_token: LiveKitAccessToken,
}

impl LiveKitRuntimeConfig {
    pub fn new(
        endpoint: LiveKitRuntimeEndpoint,
        expected_room_name: LiveKitRoomName,
        participant_binding: LiveKitParticipantBinding,
        access_token: LiveKitAccessToken,
    ) -> Self {
        Self {
            endpoint,
            expected_room_name,
            participant_binding,
            access_token,
        }
    }

    pub fn endpoint(&self) -> &LiveKitRuntimeEndpoint {
        &self.endpoint
    }

    pub fn expected_room_name(&self) -> &LiveKitRoomName {
        &self.expected_room_name
    }

    pub fn participant_binding(&self) -> &LiveKitParticipantBinding {
        &self.participant_binding
    }

    pub fn access_token(&self) -> &LiveKitAccessToken {
        &self.access_token
    }
}

pub struct LiveKitRoomRuntime {
    room: Arc<Room>,
    events: UnboundedReceiver<RoomEvent>,
    config: LiveKitRuntimeConfig,
}

pub struct RemoteAudioTrackSubscription {
    participant_identity: LiveKitParticipantIdentity,
    track_sid: String,
}

impl RemoteAudioTrackSubscription {
    pub fn participant_identity(&self) -> &LiveKitParticipantIdentity {
        &self.participant_identity
    }

    pub fn track_sid(&self) -> &str {
        &self.track_sid
    }
}

impl LiveKitRoomRuntime {
    pub async fn connect(config: LiveKitRuntimeConfig) -> AppResult<Self> {
        let (room, events) = Room::connect(
            config.endpoint().as_str(),
            config.access_token().as_str(),
            RoomOptions::default(),
        )
        .await
        .map_err(|error| {
            AppError::unavailable(format!(
                "failed to connect runtime participant {} to livekit room {}: {error}",
                config
                    .participant_binding()
                    .local_participant_identity()
                    .as_str(),
                config.expected_room_name().as_str()
            ))
        })?;

        let room = Arc::new(room);
        tracing::debug!(
            room_name = %config.expected_room_name().as_str(),
            participant_identity = %config.participant_binding().local_participant_identity().as_str(),
            expected_remote_participant_identity = %config.participant_binding().expected_remote_participant_identity().as_str(),
            "livekit runtime connected to room"
        );

        Ok(Self {
            room,
            events,
            config,
        })
    }

    pub fn room_name(&self) -> &str {
        self.config.expected_room_name().as_str()
    }

    pub fn participant_identity(&self) -> &str {
        self.config
            .participant_binding()
            .local_participant_identity()
            .as_str()
    }

    async fn wait_for_expected_remote_audio_track(
        &mut self,
        max_wait: Duration,
    ) -> AppResult<(LiveKitParticipantIdentity, String, RemoteAudioTrack)> {
        tracing::debug!(
            room_name = %self.room_name(),
            participant_identity = %self.participant_identity(),
            expected_remote_participant_identity = %self.config.participant_binding().expected_remote_participant_identity().as_str(),
            timeout_ms = max_wait.as_millis(),
            "livekit runtime waiting for expected remote audio track"
        );

        let wait_for_track = async {
            loop {
                let event = self.events.recv().await.ok_or_else(|| {
                    AppError::unavailable(
                        "livekit runtime event stream closed before remote audio track was received",
                    )
                })?;

                match event {
                    RoomEvent::ParticipantConnected(participant) => {
                        tracing::debug!(
                            room_name = %self.room_name(),
                            participant_identity = %participant.identity(),
                            expected_remote_participant_identity = %self.config.participant_binding().expected_remote_participant_identity().as_str(),
                            "livekit runtime observed remote participant connected"
                        );
                    }
                    RoomEvent::ParticipantDisconnected(participant) => {
                        tracing::debug!(
                            room_name = %self.room_name(),
                            participant_identity = %participant.identity(),
                            "livekit runtime observed remote participant disconnected"
                        );
                    }
                    RoomEvent::TrackSubscribed {
                        track,
                        publication,
                        participant,
                    } => {
                        let participant_identity =
                            LiveKitParticipantIdentity::new(participant.identity().to_string());
                        let publication_sid = publication.sid().to_string();
                        let publication_name = publication.name().to_string();
                        let publication_source = format!("{:?}", publication.source());
                        let track_kind = match &track {
                            RemoteTrack::Audio(_) => "audio",
                            RemoteTrack::Video(_) => "video",
                        };

                        tracing::debug!(
                            room_name = %self.room_name(),
                            participant_identity = %participant_identity.as_str(),
                            expected_remote_participant_identity = %self.config.participant_binding().expected_remote_participant_identity().as_str(),
                            publication_sid = %publication_sid,
                            publication_name = %publication_name,
                            publication_source = %publication_source,
                            track_kind = %track_kind,
                            "livekit runtime observed track subscribed event"
                        );

                        if participant_identity.as_str()
                            != self
                                .config
                                .participant_binding()
                                .expected_remote_participant_identity()
                                .as_str()
                        {
                            tracing::debug!(
                                room_name = %self.room_name(),
                                participant_identity = %participant_identity.as_str(),
                                expected_participant_identity = %self.config.participant_binding().expected_remote_participant_identity().as_str(),
                                "livekit runtime ignored remote track from unexpected participant"
                            );
                            continue;
                        }

                        let remote_track: RemoteTrack = track;
                        if let RemoteTrack::Audio(audio_track) = remote_track {
                            let track_sid = audio_track.sid().to_string();

                            tracing::debug!(
                                room_name = %self.room_name(),
                                participant_identity = %participant_identity.as_str(),
                                track_sid = %track_sid,
                                "livekit runtime subscribed remote audio track"
                            );

                            return Ok((participant_identity, track_sid, audio_track));
                        } else {
                            tracing::debug!(
                                room_name = %self.room_name(),
                                participant_identity = %participant_identity.as_str(),
                                publication_sid = %publication_sid,
                                "livekit runtime ignored non-audio subscribed track from expected participant"
                            );
                        }
                    }
                    RoomEvent::TrackPublished {
                        publication,
                        participant,
                    } => {
                        tracing::debug!(
                            room_name = %self.room_name(),
                            participant_identity = %participant.identity(),
                            publication_sid = %publication.sid(),
                            publication_name = %publication.name(),
                            publication_source = ?publication.source(),
                            "livekit runtime observed track published event"
                        );
                    }
                    RoomEvent::Disconnected { reason } => {
                        return Err(AppError::unavailable(format!(
                            "livekit room disconnected before remote audio track subscription: {reason:?}"
                        )));
                    }
                    _ => {
                        tracing::trace!(
                            room_name = %self.room_name(),
                            event = ?event,
                            "livekit runtime ignored room event while waiting for audio track"
                        );
                    }
                }
            }
        };

        timeout(max_wait, wait_for_track).await.map_err(|_| {
            tracing::warn!(
                room_name = %self.room_name(),
                participant_identity = %self.participant_identity(),
                expected_remote_participant_identity = %self.config.participant_binding().expected_remote_participant_identity().as_str(),
                timeout_ms = max_wait.as_millis(),
                "livekit runtime timed out waiting for expected remote audio track"
            );
            AppError::unavailable(format!(
                "timed out waiting for remote audio track in room {}",
                self.room_name()
            ))
        })?
    }

    pub async fn wait_for_remote_audio_track_subscription(
        &mut self,
        max_wait: Duration,
    ) -> AppResult<RemoteAudioTrackSubscription> {
        let (participant_identity, track_sid, _audio_track) =
            self.wait_for_expected_remote_audio_track(max_wait).await?;
        Ok(RemoteAudioTrackSubscription {
            participant_identity,
            track_sid,
        })
    }

    pub async fn bind_user_audio_input(
        &mut self,
        max_wait: Duration,
        sample_rate: u32,
        num_channels: u32,
    ) -> AppResult<UserAudioInput> {
        let (participant_identity, track_sid, audio_track) =
            self.wait_for_expected_remote_audio_track(max_wait).await?;
        tracing::debug!(
            room_name = %self.room_name(),
            participant_identity = %participant_identity.as_str(),
            track_sid = %track_sid,
            sample_rate,
            num_channels,
            "livekit runtime constructing user audio input from subscribed track"
        );
        build_user_audio_input(audio_track, sample_rate, num_channels)
    }

    pub async fn create_bot_audio_sink(
        &self,
        track_name: &str,
        sample_rate: u32,
        num_channels: u32,
    ) -> AppResult<BotAudioOutputSink> {
        BotAudioOutputSink::publish(
            self.room.clone(),
            self.room_name(),
            self.participant_identity(),
            track_name,
            sample_rate,
            num_channels,
        )
        .await
    }

    pub async fn disconnect(&self) -> AppResult<()> {
        self.room.close().await.map_err(|error| {
            AppError::unavailable(format!(
                "failed to close livekit room runtime for {}: {error}",
                self.room_name()
            ))
        })
    }
}
