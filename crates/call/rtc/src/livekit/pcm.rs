/// A fixed-format PCM frame intended for bot audio output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmFrame {
    pub data: Vec<i16>,
    pub sample_rate: u32,
    pub num_channels: u32,
    pub samples_per_channel: u32,
}

impl PcmFrame {
    pub fn new(
        data: Vec<i16>,
        sample_rate: u32,
        num_channels: u32,
        samples_per_channel: u32,
    ) -> Self {
        Self {
            data,
            sample_rate,
            num_channels,
            samples_per_channel,
        }
    }
}
