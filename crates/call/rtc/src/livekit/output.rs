use std::sync::Arc;

use libwebrtc::{
    audio_source::native::NativeAudioSource,
    prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
};
use livekit::{
    options::TrackPublishOptions,
    prelude::{LocalAudioTrack, LocalTrack, Room},
};
use shared_kernel::{AppError, AppResult};

use super::pcm::PcmFrame;

#[derive(Clone)]
pub struct BotAudioOutputSink {
    room: Arc<Room>,
    rtc_source: NativeAudioSource,
    track: LocalAudioTrack,
    expected_sample_rate: u32,
    expected_num_channels: u32,
}

pub type BotPcmFrame = PcmFrame;

impl BotAudioOutputSink {
    pub async fn publish(
        room: Arc<Room>,
        room_name: &str,
        participant_identity: &str,
        track_name: &str,
        sample_rate: u32,
        num_channels: u32,
    ) -> AppResult<Self> {
        let rtc_source = NativeAudioSource::new(
            AudioSourceOptions::default(),
            sample_rate,
            num_channels,
            1000,
        );
        let track = LocalAudioTrack::create_audio_track(
            track_name,
            RtcAudioSource::Native(rtc_source.clone()),
        );

        room.local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions::default(),
            )
            .await
            .map_err(|error| {
                AppError::unavailable(format!(
                    "failed to publish bot audio track in room {room_name}: {error}"
                ))
            })?;

        tracing::debug!(
            room_name = %room_name,
            participant_identity = %participant_identity,
            track_sid = %track.sid().to_string(),
            sample_rate,
            num_channels,
            "livekit runtime published bot audio track"
        );

        Ok(Self {
            room,
            rtc_source,
            track,
            expected_sample_rate: sample_rate,
            expected_num_channels: num_channels,
        })
    }

    pub async fn write_pcm_frame(&self, frame: &BotPcmFrame) -> AppResult<()> {
        if frame.sample_rate != self.expected_sample_rate {
            return Err(AppError::invalid_input(format!(
                "unexpected sample rate {}, expected {}",
                frame.sample_rate, self.expected_sample_rate
            )));
        }

        if frame.num_channels != self.expected_num_channels {
            return Err(AppError::invalid_input(format!(
                "unexpected channel count {}, expected {}",
                frame.num_channels, self.expected_num_channels
            )));
        }

        let expected_len = frame.samples_per_channel as usize * frame.num_channels as usize;
        if frame.data.len() != expected_len {
            return Err(AppError::invalid_input(format!(
                "pcm frame length mismatch: got {}, expected {}",
                frame.data.len(),
                expected_len
            )));
        }

        let audio_frame = AudioFrame {
            data: frame.data.as_slice().into(),
            sample_rate: frame.sample_rate,
            num_channels: frame.num_channels,
            samples_per_channel: frame.samples_per_channel,
        };

        self.rtc_source
            .capture_frame(&audio_frame)
            .await
            .map_err(|error| {
                AppError::unavailable(format!(
                    "failed to capture pcm frame into livekit audio source: {error}"
                ))
            })
    }

    pub fn clear_buffer(&self) {
        self.rtc_source.clear_buffer();
    }

    pub async fn close(&self) -> AppResult<()> {
        self.room
            .local_participant()
            .unpublish_track(&self.track.sid())
            .await
            .map(|_| ())
            .map_err(|error| {
                AppError::unavailable(format!(
                    "failed to unpublish bot audio track {}: {error}",
                    self.track.sid()
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::BotPcmFrame;

    #[test]
    fn bot_pcm_frame_preserves_shape() {
        let frame = BotPcmFrame::new(vec![1, 2, 3, 4], 48_000, 2, 2);
        assert_eq!(frame.data.len(), 4);
        assert_eq!(frame.sample_rate, 48_000);
        assert_eq!(frame.num_channels, 2);
        assert_eq!(frame.samples_per_channel, 2);
    }
}
