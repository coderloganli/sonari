use rtc::livekit::pcm::PcmFrame;

#[derive(Debug, Clone)]
pub struct AudioPreprocessor {
    noise_gate_threshold: i16,
    interrupt_rms_threshold: f32,
    dc_estimate: f32,
    noise_floor_rms: f32,
    smoothed_rms: f32,
}

#[derive(Debug, Clone)]
pub struct PreprocessedFrame {
    pub frame: PcmFrame,
    pub rms: f32,
    pub likely_speech: bool,
}

impl AudioPreprocessor {
    pub fn new(noise_gate_threshold: i16, interrupt_rms_threshold: f32) -> Self {
        Self {
            noise_gate_threshold,
            interrupt_rms_threshold,
            dc_estimate: 0.0,
            noise_floor_rms: 0.0,
            smoothed_rms: 0.0,
        }
    }

    pub fn process(&mut self, frame: PcmFrame) -> PreprocessedFrame {
        // asr_audio:仅去直流、不过噪声门限(门限会清零/衰减真实语音的弱音→ASR 识别变差);
        // gated:门限后音频,仅用于 VAD/打断的 RMS 判断(barge-in 行为保持不变)。
        let (asr_audio, gated) = self.preprocess_samples(frame.data);
        let rms = rms(&gated);
        self.smoothed_rms = smooth(self.smoothed_rms, rms, 0.35);
        PreprocessedFrame {
            frame: PcmFrame::new(
                asr_audio,
                frame.sample_rate,
                frame.num_channels,
                frame.samples_per_channel,
            ),
            rms: self.smoothed_rms,
            likely_speech: self.smoothed_rms >= self.interrupt_rms_threshold,
        }
    }

    /// 返回 (送 ASR 的去直流音频, 用于 VAD RMS 的门限后音频)。
    fn preprocess_samples(&mut self, samples: Vec<i16>) -> (Vec<i16>, Vec<i16>) {
        if samples.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let mut filtered = Vec::with_capacity(samples.len());
        for sample in samples {
            let value = sample as f32;
            self.dc_estimate = smooth(self.dc_estimate, value, 0.002);
            filtered.push((value - self.dc_estimate).clamp(i16::MIN as f32, i16::MAX as f32));
        }

        let raw_rms = rms_f32(&filtered);
        if self.noise_floor_rms <= 0.0 {
            self.noise_floor_rms = raw_rms;
        }
        let floor_alpha = if raw_rms <= self.noise_floor_rms {
            0.08
        } else {
            0.01
        };
        self.noise_floor_rms = smooth(self.noise_floor_rms, raw_rms, floor_alpha);

        let gate_floor = self.noise_gate_threshold.max(1) as f32;
        let adaptive_floor = self.noise_floor_rms.max(gate_floor);
        let soft_gate = adaptive_floor * 1.35;
        let reduction = adaptive_floor * 0.65;

        // 送 ASR:去直流后直接量化,不做门限抑制(保留弱音)。
        let asr_audio: Vec<i16> = filtered
            .iter()
            .map(|&sample| sample.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            .collect();

        // VAD/打断:门限后音频(原有逻辑不变)。
        let gated: Vec<i16> = filtered
            .into_iter()
            .map(|sample| {
                let magnitude = sample.abs();
                if magnitude <= adaptive_floor {
                    0
                } else if magnitude < soft_gate {
                    let scaled = ((magnitude - adaptive_floor) / reduction).clamp(0.0, 1.0);
                    let value = sample.signum() * magnitude * scaled;
                    value.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
                } else {
                    sample.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16
                }
            })
            .collect();

        (asr_audio, gated)
    }
}

fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let energy = samples
        .iter()
        .map(|sample| {
            let value = f32::from(*sample);
            value * value
        })
        .sum::<f32>()
        / samples.len() as f32;
    energy.sqrt()
}

fn rms_f32(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let energy = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
    energy.sqrt()
}

fn smooth(current: f32, next: f32, alpha: f32) -> f32 {
    if current == 0.0 {
        return next;
    }
    current + (next - current) * alpha
}
