use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;
use std::sync::Arc;

use crate::application::{StoredSpeechSession, frame_duration_ms, utterance_duration_ms};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechSegmentationConfig {
    pub min_utterance_ms: u32,
    pub silence_flush_ms: u32,
    pub silence_force_agent_ms: u32,
    pub voice_activity_threshold: i16,
    /// 起话前需连续累计达到的语音时长(ms)才确认为真正说话,抑制短促背景噪音误触发。
    /// 0 = 关闭确认窗(首帧语音即开始,等价改动前行为),作为回滚开关。
    #[serde(default)]
    pub min_speech_confirm_ms: u32,
}

#[async_trait]
pub trait SpeechSegmentationConfigPort: Send + Sync {
    async fn get_speech_segmentation_config(&self) -> AppResult<SpeechSegmentationConfig>;
}

#[async_trait]
impl<T> SpeechSegmentationConfigPort for Arc<T>
where
    T: SpeechSegmentationConfigPort + ?Sized,
{
    async fn get_speech_segmentation_config(&self) -> AppResult<SpeechSegmentationConfig> {
        (**self).get_speech_segmentation_config().await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechSegmentationDecision {
    Ignore,
    /// 候选累积中:疑似语音但尚未达到 min_speech_confirm_ms 的连续确认,暂不进入 ASR。
    SpeechPending,
    SpeechStarted,
    SpeechContinues,
    FlushUtterance,
}

pub trait SpeechSegmentationPolicy: Send + Sync {
    fn decide(
        &self,
        session: &StoredSpeechSession,
        pcm_s16le: &[i16],
    ) -> SpeechSegmentationDecision;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ThresholdSpeechSegmentationPolicy;

impl SpeechSegmentationPolicy for ThresholdSpeechSegmentationPolicy {
    fn decide(
        &self,
        session: &StoredSpeechSession,
        pcm_s16le: &[i16],
    ) -> SpeechSegmentationDecision {
        let frame_ms = frame_duration_ms(
            pcm_s16le.len(),
            session.sample_rate_hz,
            session.num_channels,
        );
        if has_voice_activity(
            pcm_s16le,
            session.segmentation_config.voice_activity_threshold,
        ) {
            if !session.utterance_pcm.is_empty() {
                // 已进入正式语音段
                SpeechSegmentationDecision::SpeechContinues
            } else if session.candidate_speech_ms + frame_ms
                >= session.segmentation_config.min_speech_confirm_ms
            {
                // 连续语音累计达到确认窗 → 正式开始
                // (min_speech_confirm_ms=0 时首帧即确认,等价改动前行为)
                SpeechSegmentationDecision::SpeechStarted
            } else {
                // 候选累积中,暂不进入 ASR
                SpeechSegmentationDecision::SpeechPending
            }
        } else if !session.utterance_pcm.is_empty() {
            // 正式语音段中的静音 → 尾随静音 / 切句
            let trailing_silence_ms = session.trailing_silence_ms + frame_ms;
            let utterance_ms = utterance_duration_ms(
                session.utterance_pcm.len(),
                session.sample_rate_hz,
                session.num_channels,
            );

            if trailing_silence_ms >= session.segmentation_config.silence_flush_ms
                && utterance_ms >= session.segmentation_config.min_utterance_ms
            {
                SpeechSegmentationDecision::FlushUtterance
            } else {
                SpeechSegmentationDecision::Ignore
            }
        } else if session.candidate_speech_ms > 0 {
            // 候选期内的静音 → 递减(抗抖 / hangover),仍属候选
            SpeechSegmentationDecision::SpeechPending
        } else {
            // 完全空闲
            SpeechSegmentationDecision::Ignore
        }
    }
}

pub(crate) fn has_voice_activity(pcm_s16le: &[i16], threshold: i16) -> bool {
    if pcm_s16le.is_empty() {
        return false;
    }

    let avg = pcm_s16le
        .iter()
        .map(|sample| i32::from(sample.abs()))
        .sum::<i32>()
        / pcm_s16le.len() as i32;
    avg >= i32::from(threshold)
}
