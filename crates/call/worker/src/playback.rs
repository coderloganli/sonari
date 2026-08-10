use anyhow::Result;
use rtc::livekit::{output::BotAudioOutputSink, pcm::PcmFrame};

use crate::mixer::BotAudioMixer;

#[derive(Clone)]
pub struct BotPlayback {
    mixer: BotAudioMixer,
}

impl BotPlayback {
    pub fn new(sink: BotAudioOutputSink) -> Self {
        Self {
            mixer: BotAudioMixer::start(sink),
        }
    }

    pub async fn play_speech_frame(&self, frame: PcmFrame) -> Result<()> {
        self.mixer.enqueue_speech_frame(frame).await
    }

    pub async fn play_pre_recorded_frame(&self, frame: PcmFrame) -> Result<()> {
        self.mixer.enqueue_pre_recorded_frame(frame).await
    }

    pub fn interrupt_speech(&self) {
        self.mixer.clear_output_buffer();
        self.mixer.interrupt_speech();
    }

    pub async fn wait_until_drained(&self) -> Result<()> {
        self.mixer.wait_until_drained().await
    }

    pub fn current_epoch(&self) -> u64 {
        self.mixer.current_epoch()
    }

    pub fn is_current_epoch(&self, epoch: u64) -> bool {
        self.mixer.is_current_epoch(epoch)
    }

    pub async fn close(&self) -> Result<()> {
        self.mixer.close().await
    }
}
