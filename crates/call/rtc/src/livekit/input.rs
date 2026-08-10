use libwebrtc::audio_stream::native::NativeAudioStream;
use livekit::prelude::RemoteAudioTrack;
use shared_kernel::{AppError, AppResult};
use tokio_stream::StreamExt;

use super::pcm::PcmFrame;

pub struct UserAudioInput {
    stream: NativeAudioStream,
}

impl UserAudioInput {
    pub fn new(audio_track: RemoteAudioTrack, sample_rate: u32, num_channels: u32) -> Self {
        let track_sid = audio_track.sid().to_string();
        tracing::debug!(
            track_sid = %track_sid,
            sample_rate,
            num_channels,
            "livekit user audio input creating native audio stream"
        );
        Self {
            stream: NativeAudioStream::new(
                audio_track.rtc_track(),
                sample_rate as i32,
                num_channels as i32,
            ),
        }
    }

    pub async fn next_frame(&mut self) -> AppResult<Option<PcmFrame>> {
        let frame = self.stream.next().await.map(|frame| {
            PcmFrame::new(
                frame.data.to_vec(),
                frame.sample_rate,
                frame.num_channels,
                frame.samples_per_channel,
            )
        });
        Ok(frame)
    }

    pub fn close(&mut self) -> AppResult<()> {
        self.stream.close();
        Ok(())
    }
}

pub fn build_user_audio_input(
    audio_track: RemoteAudioTrack,
    sample_rate: u32,
    num_channels: u32,
) -> AppResult<UserAudioInput> {
    if sample_rate == 0 || num_channels == 0 {
        return Err(AppError::invalid_input(
            "user audio input requires positive sample_rate and num_channels",
        ));
    }

    tracing::debug!(
        track_sid = %audio_track.sid(),
        sample_rate,
        num_channels,
        "livekit user audio input validated remote audio track"
    );
    Ok(UserAudioInput::new(audio_track, sample_rate, num_channels))
}
