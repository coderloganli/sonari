use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkerConfig {
    pub runtime_owner_id: String,
    pub poll_interval_ms: u64,
    pub control_plane_retry_initial_ms: u64,
    pub control_plane_retry_max_ms: u64,
    pub speech_poll_interval_ms: u64,
    pub track_name: String,
    pub sample_rate: u32,
    pub num_channels: u32,
    pub noise_gate_threshold: i16,
    pub interrupt_rms_threshold: f32,
    /// 进程内编排:worker 级主密钥,用于解开 dispatch 下发的 ASR/TTS api_key 密文(信封加密)。
    pub voice_secrets_key: String,
}

impl WorkerConfig {
    /// Built by the server, which has already resolved the owner id and the
    /// voice secrets key. Everything else is tuning with a default.
    pub fn from_env(runtime_owner_id: String, voice_secrets_key: String) -> Result<Self> {
        let poll_interval_ms = std::env::var("RUNTIME_POLL_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1_000);
        let control_plane_retry_initial_ms = std::env::var("CONTROL_PLANE_RETRY_INITIAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(500);
        let control_plane_retry_max_ms = std::env::var("CONTROL_PLANE_RETRY_MAX_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5_000);
        let track_name = std::env::var("BOT_TRACK_NAME").unwrap_or_else(|_| "bot-audio".to_owned());
        let speech_poll_interval_ms = std::env::var("SPEECH_POLL_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60);
        let sample_rate = std::env::var("BOT_SAMPLE_RATE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(16_000);
        let num_channels = std::env::var("BOT_NUM_CHANNELS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        let noise_gate_threshold = std::env::var("INPUT_NOISE_GATE_THRESHOLD")
            .ok()
            .and_then(|value| value.parse::<i16>().ok())
            .unwrap_or(96);
        let interrupt_rms_threshold = std::env::var("INTERRUPT_RMS_THRESHOLD")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(750.0);
        if runtime_owner_id.trim().is_empty() {
            bail!("runtime owner id is required");
        }
        if control_plane_retry_initial_ms == 0 {
            bail!("CONTROL_PLANE_RETRY_INITIAL_MS must be positive");
        }
        if control_plane_retry_max_ms < control_plane_retry_initial_ms {
            bail!("CONTROL_PLANE_RETRY_MAX_MS must be >= CONTROL_PLANE_RETRY_INITIAL_MS");
        }

        Ok(Self {
            runtime_owner_id,
            poll_interval_ms,
            control_plane_retry_initial_ms,
            control_plane_retry_max_ms,
            speech_poll_interval_ms,
            track_name,
            sample_rate,
            num_channels,
            noise_gate_threshold,
            interrupt_rms_threshold,
            voice_secrets_key,
        })
    }
}
