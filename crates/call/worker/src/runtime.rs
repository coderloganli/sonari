use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rtc::livekit::{
    input::UserAudioInput,
    output::BotAudioOutputSink,
    runtime::{
        LiveKitAccessToken, LiveKitParticipantBinding, LiveKitRoomRuntime, LiveKitRuntimeConfig,
        LiveKitRuntimeEndpoint,
    },
    types::{LiveKitParticipantIdentity, LiveKitRoomName},
};

use crate::{
    client::WorkerRuntimeLaunch,
    config::WorkerConfig,
    input::StreamingUserAudioInput,
    pipeline::{SpeechHandler, SpeechPipeline, SpeechPipelineConfig},
    playback::BotPlayback,
    preprocess::AudioPreprocessor,
};

pub struct ActiveRuntime {
    room: LiveKitRoomRuntime,
    pipeline: SpeechPipeline,
}

impl ActiveRuntime {
    pub async fn start(
        config: &WorkerConfig,
        _session_id: i64,
        launch: WorkerRuntimeLaunch,
        _client: crate::client::RuntimeControlClient,
        handler: Arc<dyn SpeechHandler>,
    ) -> Result<Self> {
        let room_name = launch.room_name.clone();
        let local_participant_identity = launch.local_participant_identity.clone();
        let expected_remote_participant_identity =
            launch.expected_remote_participant_identity.clone();

        tracing::debug!(
            room_name = %room_name,
            local_participant_identity = %local_participant_identity,
            expected_remote_participant_identity = %expected_remote_participant_identity,
            sample_rate = config.sample_rate,
            num_channels = config.num_channels,
            "worker starting active runtime"
        );

        handler
            .ensure_available()
            .await
            .context("speech handler is unavailable for worker runtime")?;

        let runtime = LiveKitRoomRuntime::connect(LiveKitRuntimeConfig::new(
            LiveKitRuntimeEndpoint::new(launch.endpoint),
            LiveKitRoomName::new(launch.room_name),
            LiveKitParticipantBinding::new(
                LiveKitParticipantIdentity::new(launch.local_participant_identity),
                LiveKitParticipantIdentity::new(launch.expected_remote_participant_identity),
            ),
            LiveKitAccessToken::new(launch.access_token),
        ))
        .await
        .context("failed to connect livekit room runtime")?;

        let mut runtime = runtime;
        tracing::debug!(
            room_name = %room_name,
            expected_remote_participant_identity = %expected_remote_participant_identity,
            "worker binding user audio input"
        );
        let user_input = runtime
            .bind_user_audio_input(
                Duration::from_secs(30),
                config.sample_rate,
                config.num_channels,
            )
            .await
            .context("failed to bind user audio input")?;
        tracing::debug!(
            room_name = %room_name,
            expected_remote_participant_identity = %expected_remote_participant_identity,
            "worker bound user audio input"
        );

        let sink = runtime
            .create_bot_audio_sink(&config.track_name, config.sample_rate, config.num_channels)
            .await
            .context("failed to create bot audio sink")?;

        let (pipeline, _playback) =
            match build_pipeline(config, user_input, sink, handler.clone()).await {
                Ok(pipeline) => pipeline,
                Err(error) => {
                    runtime
                        .disconnect()
                        .await
                        .context("failed to disconnect runtime room after pipeline setup error")?;
                    return Err(error).context("failed to build worker speech pipeline");
                }
            };

        Ok(Self {
            room: runtime,
            pipeline,
        })
    }

    pub async fn stop(self) -> Result<()> {
        self.pipeline.stop().await?;
        self.room
            .disconnect()
            .await
            .context("failed to disconnect runtime room")?;
        Ok(())
    }

    pub async fn poll_health(&mut self) -> Result<Option<Result<()>>> {
        self.pipeline.poll_completion().await
    }
}

async fn build_pipeline(
    config: &WorkerConfig,
    user_input: UserAudioInput,
    sink: BotAudioOutputSink,
    handler: Arc<dyn SpeechHandler>,
) -> Result<(SpeechPipeline, BotPlayback)> {
    let streaming_input = StreamingUserAudioInput::new(user_input);
    let playback = BotPlayback::new(sink);
    let pipeline = SpeechPipeline::start(
        streaming_input,
        playback.clone(),
        handler,
        SpeechPipelineConfig {
            preprocessor: AudioPreprocessor::new(
                config.noise_gate_threshold,
                config.interrupt_rms_threshold,
            ),
            speech_poll_interval_ms: config.speech_poll_interval_ms,
            sample_rate_hz: config.sample_rate,
            num_channels: config.num_channels as u16,
        },
    )
    .await?;
    Ok((pipeline, playback))
}
