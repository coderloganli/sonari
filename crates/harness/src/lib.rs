//! Evaluation harness: runs a set of recordings through sonari and reports
//! accuracy and latency as distributions.
//!
//! The pieces are deliberately separable. A `Solver` drives one sample and
//! returns an `Outcome` — pure data, no judgement. Scorers turn a Sample and an
//! Outcome into named scores, and metrics reduce those across epochs and across
//! samples. An agent-quality evaluation adds a Solver and a Scorer; it does not
//! touch the manifest, the runner or the report.

pub mod generate;
pub mod manifest;
pub mod markers;
pub mod metrics;
pub mod render;
pub mod report;
pub mod runner;
pub mod score;
pub mod solver;
