use anyhow::Result;
use rtc::livekit::{input::UserAudioInput, pcm::PcmFrame};

pub struct StreamingUserAudioInput {
    input: UserAudioInput,
}

impl StreamingUserAudioInput {
    pub fn new(input: UserAudioInput) -> Self {
        Self { input }
    }

    pub async fn next_frame(&mut self) -> Result<Option<PcmFrame>> {
        self.input.next_frame().await.map_err(Into::into)
    }

    pub fn close(&mut self) -> Result<()> {
        self.input.close().map_err(Into::into)
    }
}
