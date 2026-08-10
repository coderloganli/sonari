//! Loads pre-recorded bot speech assets into the unified worker playback queue.

use std::io::Cursor;

use anyhow::{Context, Result, anyhow};
use hound::{SampleFormat, WavReader};
use reqwest::Client;
use rtc::livekit::pcm::PcmFrame;

pub async fn load_pre_recorded_frames(
    http: &Client,
    audio_url: &str,
    target_sample_rate_hz: u32,
    target_num_channels: u16,
) -> Result<Vec<PcmFrame>> {
    let response = http
        .get(audio_url)
        .send()
        .await
        .with_context(|| format!("failed to fetch pre-recorded audio {audio_url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "pre-recorded audio fetch failed for {audio_url} with status {}",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read pre-recorded audio body {audio_url}"))?;
    decode_wav_frames(&bytes, target_sample_rate_hz, target_num_channels)
}

fn decode_wav_frames(
    wav_bytes: &[u8],
    target_sample_rate_hz: u32,
    target_num_channels: u16,
) -> Result<Vec<PcmFrame>> {
    let cursor = Cursor::new(wav_bytes.to_vec());
    let mut reader = WavReader::new(cursor).context("failed to open pre-recorded wav bytes")?;
    let spec = reader.spec();
    let src_channels = spec.channels.max(1);
    let mut samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode 16-bit wav samples")?,
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|sample| sample.map(|value| (value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to decode float wav samples")?,
        _ => {
            return Err(anyhow!(
                "unsupported pre-recorded wav format: {:?} {}-bit",
                spec.sample_format,
                spec.bits_per_sample
            ));
        }
    };

    samples = remap_channels(samples, src_channels, target_num_channels);
    samples = resample_linear(
        samples,
        spec.sample_rate,
        target_sample_rate_hz,
        target_num_channels,
    );

    const CHUNK_MS: u32 = 20;
    let samples_per_chunk = ((target_sample_rate_hz / 1000) * CHUNK_MS).max(1) as usize
        * usize::from(target_num_channels);
    let mut frames = Vec::new();
    for chunk in samples.chunks(samples_per_chunk) {
        let samples_per_channel = (chunk.len() / usize::from(target_num_channels.max(1))) as u32;
        frames.push(PcmFrame::new(
            chunk.to_vec(),
            target_sample_rate_hz,
            u32::from(target_num_channels),
            samples_per_channel,
        ));
    }
    Ok(frames)
}

fn remap_channels(samples: Vec<i16>, src_channels: u16, dst_channels: u16) -> Vec<i16> {
    if src_channels == dst_channels {
        return samples;
    }

    let src_channels = usize::from(src_channels.max(1));
    let dst_channels = usize::from(dst_channels.max(1));
    let frames = samples.chunks(src_channels);
    let mut output = Vec::new();
    for frame in frames {
        match (src_channels, dst_channels) {
            (1, 2) => {
                let sample = frame.first().copied().unwrap_or_default();
                output.push(sample);
                output.push(sample);
            }
            (2, 1) => {
                let left = frame.first().copied().unwrap_or_default() as i32;
                let right = frame.get(1).copied().unwrap_or_default() as i32;
                output.push(((left + right) / 2) as i16);
            }
            _ => {
                for channel in 0..dst_channels {
                    output.push(
                        frame
                            .get(channel % src_channels)
                            .copied()
                            .unwrap_or_default(),
                    );
                }
            }
        }
    }
    output
}

fn resample_linear(samples: Vec<i16>, src_rate: u32, dst_rate: u32, channels: u16) -> Vec<i16> {
    if src_rate == dst_rate {
        return samples;
    }

    let channels = usize::from(channels.max(1));
    let src_frames = samples.len() / channels;
    if src_frames <= 1 {
        return samples;
    }
    let ratio = dst_rate as f64 / src_rate as f64;
    let dst_frames = ((src_frames as f64) * ratio).round().max(1.0) as usize;
    let mut output = Vec::with_capacity(dst_frames * channels);

    for dst_frame in 0..dst_frames {
        let src_pos = (dst_frame as f64) / ratio;
        let src_index = src_pos.floor() as usize;
        let next_index = (src_index + 1).min(src_frames - 1);
        let frac = (src_pos - src_index as f64) as f32;
        for channel in 0..channels {
            let a = samples[src_index * channels + channel] as f32;
            let b = samples[next_index * channels + channel] as f32;
            let mixed = a + (b - a) * frac;
            output.push(mixed.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
        }
    }

    output
}
