//! Running one sample.
//!
//! A solver drives the system and reports what happened. It does not judge:
//! scoring reads the outcome afterwards. That separation is what lets an
//! agent-quality evaluation add a multi-turn solver later without the runner,
//! the manifest or the report knowing about it.

pub mod api;
#[cfg(feature = "live")]
pub mod live_call;
pub mod single_turn;
pub mod timeline;

use async_trait::async_trait;

use crate::{manifest::Sample, markers::Markers};

/// What the system did with one sample. Pure data — no judgement about whether
/// it was right.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub transcript: String,
    pub reply: String,
    pub markers: Markers,
    /// How many separate utterances the recording produced. One means the turn
    /// was never cut; two means something ended it partway through the clip.
    pub utterance_count: usize,
    /// Whether the system judged the audio to be speech at all. False means it
    /// was discarded as noise and never reached recognition — a different
    /// failure from recognition returning nothing.
    pub turn_opened: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    #[error("{0}")]
    Failed(String),
}

#[async_trait]
pub trait Solver: Send + Sync {
    async fn run(&self, sample: &Sample) -> Result<Outcome, SolverError>;
}
