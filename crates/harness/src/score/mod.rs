//! Turning an outcome into named scores.
//!
//! Scorers are the layer an agent-quality evaluation extends: a judge is another
//! scorer over the same outcome, and adding one leaves the runner and the report
//! untouched.

pub mod latency;
pub mod text;
pub mod wer;
