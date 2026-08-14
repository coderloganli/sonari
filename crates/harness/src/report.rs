//! What a run leaves behind.
//!
//! One file per run, carrying enough to be read months later: the per-sample
//! rows, the aggregates, how many samples failed, and a snapshot of what was
//! being measured.
//!
//! `samples_failed` sits beside every aggregate on purpose. A batch that lost
//! its three slowest recordings would otherwise report excellent percentiles —
//! the same principle as `docs/architecture.md` §8, where a failure is returned
//! rather than made to look like silence.

use serde::{Deserialize, Serialize};

use crate::{markers::Markers, score::wer::WordErrors};

/// Stated in every report. Fifteen recordings cannot resolve a difference of a
/// point or two, and a number without that caveat invites conclusions it cannot
/// support.
pub const PRECISION_NOTE: &str = "At this set size the confidence interval on WER is roughly ±5-10 points absolute. This is a regression tripwire and a category-failure detector, not an instrument for ranking systems a point apart.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SampleStatus {
    Ok,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleReport {
    pub id: String,
    #[serde(flatten)]
    pub status: SampleStatus,
    pub transcript: Option<String>,
    /// `None` when the reference is empty — there is nothing to align against,
    /// and the clip is asking whether anything happened rather than how
    /// accurate it was.
    pub wer: Option<f64>,
    pub errors: Option<WordErrors>,
    /// Medians across the epochs that succeeded.
    pub markers: Markers,
    pub system_response_ms: Option<f64>,
    pub perceived_latency_ms: Option<f64>,
    pub hangover_cost_ms: Option<f64>,
    pub utterance_count: Option<usize>,
    pub turn_opened: Option<bool>,
    pub epochs_ok: usize,
    pub epochs_failed: usize,
}

/// p50 and p95 of one measurement across the run.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Spread {
    pub p50: Option<f64>,
    pub p95: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Summary {
    pub corpus_wer: Option<f64>,
    pub wer_p50: Option<f64>,
    pub wer_p90: Option<f64>,
    pub wer_max: Option<f64>,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub reference_words: usize,
    pub system_response: Spread,
    pub perceived_latency: Spread,
    pub asr_final: Spread,
    pub llm_first_token: Spread,
    pub llm_first_sentence: Spread,
    pub tts_first_chunk: Spread,
    pub samples_ok: usize,
    pub samples_failed: usize,
    /// Of the clips whose reference is empty — the ones that should have
    /// produced nothing — how many produced something. Reported apart from WER
    /// because it is a different failure: mishearing a word degrades an answer,
    /// answering a cough is the system talking to itself.
    pub false_triggers: usize,
    pub silent_clips: usize,
}

/// What was being measured, so a report read later is interpretable.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub asr_model: String,
    pub tts_model: String,
    pub llm_model: String,
    pub silence_flush_ms: u32,
    pub min_utterance_ms: u32,
    pub min_speech_confirm_ms: u32,
    pub voice_activity_threshold: i16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchReport {
    pub run_at: String,
    /// Which solver produced these numbers. `single-turn` commits at the end of
    /// the file and never exercises endpointing; `live` drives the running
    /// service and does. A reader who does not know which is which can draw a
    /// conclusion the run cannot support.
    pub solver: String,
    pub epochs: usize,
    pub note: String,
    pub config: ConfigSnapshot,
    pub summary: Summary,
    pub samples: Vec<SampleReport>,
}
