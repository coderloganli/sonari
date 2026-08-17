//! Running the set.
//!
//! Every sample is attempted `epochs` times. A sample that fails is recorded as
//! failed and the batch carries on: losing one recording is not a reason to lose
//! the other fourteen, and a run that aborted halfway would leave numbers that
//! look like a complete result.
//!
//! Nothing is silently dropped. Failures are counted, reported per sample, and
//! summarised beside every aggregate.

use std::time::Duration;

use crate::{
    manifest::Sample,
    markers::Markers,
    report::{
        BUILD_PROFILE, BatchReport, ConfigSnapshot, PRECISION_NOTE, SampleReport, SampleStatus,
        Spread, Summary,
    },
    score::wer::WordErrors,
    solver::{Outcome, Solver},
};

#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Names the solver in the report. See [`crate::report::BatchReport`].
    pub solver: &'static str,
    /// What was being measured. A report without it is a page of numbers with
    /// no way to tell which system produced them.
    pub snapshot: ConfigSnapshot,
    pub epochs: usize,
    /// How long one attempt may take before it is recorded as a failure. A
    /// solver that never returns must not hold the batch open.
    pub sample_timeout: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            solver: "single-turn",
            snapshot: ConfigSnapshot::default(),
            epochs: 1,
            sample_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BatchError {
    #[error("there are no samples to run")]
    NoSamples,
}

pub async fn run_batch(
    samples: &[Sample],
    solver: &dyn Solver,
    config: BatchConfig,
) -> Result<BatchReport, BatchError> {
    if samples.is_empty() {
        return Err(BatchError::NoSamples);
    }

    let mut rows = Vec::with_capacity(samples.len());
    // Sequentially, on purpose. Running fifteen recordings at once would finish
    // sooner and measure something else: concurrent sessions contending for
    // bandwidth and provider rate limits. This is a latency benchmark.
    for sample in samples {
        rows.push(run_sample(sample, solver, &config).await);
    }

    let summary = summarise(&rows);
    Ok(BatchReport {
        run_at: chrono::Utc::now().to_rfc3339(),
        solver: config.solver.to_owned(),
        build: BUILD_PROFILE.to_owned(),
        epochs: config.epochs,
        note: PRECISION_NOTE.to_owned(),
        config: config.snapshot.clone(),
        summary,
        samples: rows,
    })
}

async fn run_sample(sample: &Sample, solver: &dyn Solver, config: &BatchConfig) -> SampleReport {
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();

    for _ in 0..config.epochs.max(1) {
        match tokio::time::timeout(config.sample_timeout, solver.run(sample)).await {
            Ok(Ok(outcome)) => outcomes.push(outcome),
            Ok(Err(error)) => failures.push(error.to_string()),
            Err(_) => failures.push(format!(
                "timed out after {} ms",
                config.sample_timeout.as_millis()
            )),
        }
    }

    let epochs_ok = outcomes.len();
    let epochs_failed = failures.len();

    let Some(first) = outcomes.first().cloned() else {
        return SampleReport {
            id: sample.id.clone(),
            status: SampleStatus::Failed {
                // Epochs can fail differently, and every distinct reason is
                // worth keeping — but repeating the same one per epoch just
                // makes the line unreadable.
                reason: {
                    let mut distinct: Vec<String> = Vec::new();
                    for reason in failures {
                        if !distinct.contains(&reason) {
                            distinct.push(reason);
                        }
                    }
                    distinct.join("; ")
                },
            },
            transcript: None,
            wer: None,
            errors: None,
            markers: Markers::default(),
            system_response_ms: None,
            perceived_latency_ms: None,
            hangover_cost_ms: None,
            utterance_count: None,
            server_turns: None,
            turn_opened: None,
            epochs_ok,
            epochs_failed,
        };
    };

    // Latency reduces by median across epochs; the transcript does not, because
    // there is no median of a sentence. The first successful epoch supplies the
    // text, which is honest as long as it is stated: repeating a run buys
    // confidence about timing, not about recognition.
    let markers = reduce_markers(&outcomes);
    let errors = (!sample.reference.trim().is_empty())
        .then(|| crate::score::wer::score(&sample.reference, &first.transcript).ok())
        .flatten();

    SampleReport {
        id: sample.id.clone(),
        status: SampleStatus::Ok,
        transcript: Some(first.transcript),
        wer: errors.and_then(|entry| entry.rate()),
        errors,
        markers,
        system_response_ms: markers.system_response_ms(),
        perceived_latency_ms: markers.perceived_latency_ms(),
        hangover_cost_ms: markers.hangover_cost_ms(),
        utterance_count: Some(first.utterance_count),
        server_turns: Some(first.server_turns),
        turn_opened: Some(first.turn_opened),
        epochs_ok,
        epochs_failed,
    }
}

/// Median of each marker across the epochs that produced it. A marker missing
/// from every epoch stays missing rather than becoming zero.
fn reduce_markers(outcomes: &[Outcome]) -> Markers {
    fn reduce(outcomes: &[Outcome], pick: fn(&Markers) -> Option<f64>) -> Option<f64> {
        let values: Vec<f64> = outcomes
            .iter()
            .filter_map(|outcome| pick(&outcome.markers))
            .collect();
        crate::metrics::median(&values)
    }

    Markers {
        speech_start_ms: reduce(outcomes, |m| m.speech_start_ms),
        speech_last_voiced_ms: reduce(outcomes, |m| m.speech_last_voiced_ms),
        speech_end_ms: reduce(outcomes, |m| m.speech_end_ms),
        asr_final_ms: reduce(outcomes, |m| m.asr_final_ms),
        llm_first_token_ms: reduce(outcomes, |m| m.llm_first_token_ms),
        llm_first_sentence_ms: reduce(outcomes, |m| m.llm_first_sentence_ms),
        tts_first_chunk_ms: reduce(outcomes, |m| m.tts_first_chunk_ms),
        audio_first_frame_ms: reduce(outcomes, |m| m.audio_first_frame_ms),
    }
}

fn summarise(rows: &[SampleReport]) -> Summary {
    let errors: Vec<WordErrors> = rows.iter().filter_map(|row| row.errors).collect();
    let rates: Vec<f64> = rows.iter().filter_map(|row| row.wer).collect();

    let spread = |pick: fn(&SampleReport) -> Option<f64>| {
        let values = crate::score::latency::series(rows, pick);
        Spread {
            p50: crate::metrics::percentile(&values, 0.5),
            p95: crate::metrics::percentile(&values, 0.95),
        }
    };

    // Clips whose reference is empty were meant to produce nothing. Anything
    // they produced is a false trigger, counted apart from accuracy.
    let silent_clips = rows
        .iter()
        .filter(|row| row.errors.is_none() && matches!(row.status, SampleStatus::Ok))
        .count();
    let false_triggers = rows
        .iter()
        .filter(|row| {
            row.errors.is_none()
                && matches!(row.status, SampleStatus::Ok)
                && (row.turn_opened == Some(true)
                    || row
                        .transcript
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty()))
        })
        .count();

    Summary {
        corpus_wer: crate::metrics::corpus_wer(&errors),
        wer_p50: crate::metrics::percentile(&rates, 0.5),
        wer_p90: crate::metrics::percentile(&rates, 0.9),
        wer_max: crate::metrics::percentile(&rates, 1.0),
        substitutions: errors.iter().map(|entry| entry.substitutions).sum(),
        deletions: errors.iter().map(|entry| entry.deletions).sum(),
        insertions: errors.iter().map(|entry| entry.insertions).sum(),
        reference_words: errors.iter().map(|entry| entry.reference_words).sum(),
        system_response: spread(|row| row.system_response_ms),
        perceived_latency: spread(|row| row.perceived_latency_ms),
        asr_final: spread(|row| row.markers.asr_final_ms),
        llm_first_token: spread(|row| row.markers.llm_first_token_ms),
        llm_first_sentence: spread(|row| row.markers.llm_first_sentence_ms),
        tts_first_chunk: spread(|row| row.markers.tts_first_chunk_ms),
        samples_ok: rows
            .iter()
            .filter(|row| matches!(row.status, SampleStatus::Ok))
            .count(),
        samples_failed: rows
            .iter()
            .filter(|row| matches!(row.status, SampleStatus::Failed { .. }))
            .count(),
        false_triggers,
        silent_clips,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{
        markers::Markers,
        report::SampleStatus,
        solver::{Outcome, SolverError},
    };

    /// What one sample does when the solver reaches it.
    #[derive(Clone)]
    enum Behaviour {
        /// Return this transcript.
        Transcribes(&'static str),
        /// Fail every time.
        Fails,
        /// Fail for the first `n` attempts, then succeed.
        FailsTimes(usize),
        /// Never return.
        Hangs,
        /// Succeed, but only after this delay — used to make results arrive out
        /// of order.
        Slow(Duration, &'static str),
    }

    /// A solver whose behaviour is dictated per sample, so failures can be
    /// triggered where a real provider could not be asked to produce them.
    struct ScriptedSolver {
        behaviours: HashMap<String, Behaviour>,
        calls: AtomicUsize,
        attempts: Mutex<HashMap<String, usize>>,
    }

    impl ScriptedSolver {
        fn new(behaviours: &[(&str, Behaviour)]) -> Self {
            Self {
                behaviours: behaviours
                    .iter()
                    .map(|(id, behaviour)| ((*id).to_owned(), behaviour.clone()))
                    .collect(),
                calls: AtomicUsize::new(0),
                attempts: Mutex::new(HashMap::new()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Solver for ScriptedSolver {
        async fn run(&self, sample: &Sample) -> Result<Outcome, SolverError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let attempt = {
                let mut attempts = self.attempts.lock().expect("attempt counter");
                let entry = attempts.entry(sample.id.clone()).or_insert(0);
                *entry += 1;
                *entry
            };

            let outcome = |transcript: &str| Outcome {
                transcript: transcript.to_owned(),
                reply: "sure".to_owned(),
                utterance_count: 1,
                server_turns: 0,
                turn_opened: true,
                markers: Markers {
                    speech_start_ms: Some(100.0),
                    speech_last_voiced_ms: Some(1_000.0),
                    speech_end_ms: Some(1_700.0),
                    asr_final_ms: Some(1_820.0),
                    llm_first_token_ms: Some(2_200.0),
                    llm_first_sentence_ms: Some(2_300.0),
                    tts_first_chunk_ms: Some(2_900.0),
                    audio_first_frame_ms: Some(2_950.0),
                },
            };

            match self.behaviours.get(&sample.id) {
                Some(Behaviour::Transcribes(text)) => Ok(outcome(text)),
                Some(Behaviour::Fails) => Err(SolverError::Failed("scripted failure".to_owned())),
                Some(Behaviour::FailsTimes(times)) if attempt <= *times => {
                    Err(SolverError::Failed("scripted failure".to_owned()))
                }
                Some(Behaviour::FailsTimes(_)) => Ok(outcome("recovered")),
                Some(Behaviour::Hangs) => {
                    futures::future::pending::<()>().await;
                    unreachable!("a pending future never resolves")
                }
                Some(Behaviour::Slow(delay, text)) => {
                    tokio::time::sleep(*delay).await;
                    Ok(outcome(text))
                }
                None => Ok(outcome("unscripted")),
            }
        }
    }

    fn samples(ids: &[&str]) -> Vec<Sample> {
        ids.iter()
            .map(|id| Sample {
                id: (*id).to_owned(),
                audio: PathBuf::from(format!("clips/{id}.wav")),
                reference: "the quick brown fox".to_owned(),
                probe: None,
            })
            .collect()
    }

    fn row<'a>(report: &'a BatchReport, id: &str) -> &'a crate::report::SampleReport {
        report
            .samples
            .iter()
            .find(|sample| sample.id == id)
            .unwrap_or_else(|| panic!("no row for {id}"))
    }

    /// Spec 30. One failure must not remove the other four from the report, and
    /// must not be quietly folded into the aggregates.
    #[tokio::test]
    async fn a_failing_sample_is_isolated() {
        let ids = ["a", "b", "c", "d", "e"];
        let solver = ScriptedSolver::new(&[
            ("a", Behaviour::Transcribes("the quick brown fox")),
            ("b", Behaviour::Fails),
            ("c", Behaviour::Transcribes("the quick brown fox")),
            ("d", Behaviour::Transcribes("the quick brown fox")),
            ("e", Behaviour::Transcribes("the quick brown fox")),
        ]);

        let report = run_batch(&samples(&ids), &solver, BatchConfig::default())
            .await
            .expect("a non-empty manifest");

        assert_eq!(report.samples.len(), 5, "every sample keeps its row");
        assert!(matches!(
            row(&report, "b").status,
            SampleStatus::Failed { .. }
        ));
        assert_eq!(report.summary.samples_ok, 4);
        assert_eq!(report.summary.samples_failed, 1);
        assert_eq!(
            report.summary.reference_words, 16,
            "the failed sample contributes nothing to the denominator"
        );
    }

    /// Spec 31.
    #[tokio::test]
    async fn a_wholly_failed_batch_reports_no_figures() {
        let ids = ["a", "b"];
        let solver = ScriptedSolver::new(&[("a", Behaviour::Fails), ("b", Behaviour::Fails)]);

        let report = run_batch(&samples(&ids), &solver, BatchConfig::default())
            .await
            .expect("a non-empty manifest");

        assert_eq!(report.summary.samples_ok, 0);
        assert_eq!(report.summary.samples_failed, 2);
        assert_eq!(report.summary.corpus_wer, None);
        assert_eq!(report.summary.system_response.p50, None);
    }

    /// Spec 32.
    #[tokio::test]
    async fn every_sample_runs_once_per_epoch() {
        let ids = ["a", "b", "c", "d", "e"];
        let solver = ScriptedSolver::new(&[]);
        let config = BatchConfig {
            epochs: 3,
            ..BatchConfig::default()
        };

        let report = run_batch(&samples(&ids), &solver, config)
            .await
            .expect("a non-empty manifest");

        assert_eq!(solver.calls(), 15);
        assert_eq!(report.epochs, 3);
        assert_eq!(row(&report, "a").epochs_ok, 3);
    }

    /// Spec 33. A sample that mostly worked is not thrown away.
    #[tokio::test]
    async fn a_sample_reduces_over_the_epochs_that_succeeded() {
        let ids = ["a"];
        let solver = ScriptedSolver::new(&[("a", Behaviour::FailsTimes(1))]);
        let config = BatchConfig {
            epochs: 3,
            ..BatchConfig::default()
        };

        let report = run_batch(&samples(&ids), &solver, config)
            .await
            .expect("a non-empty manifest");

        let row = row(&report, "a");
        assert!(matches!(row.status, SampleStatus::Ok));
        assert_eq!(row.epochs_ok, 2);
        assert_eq!(row.epochs_failed, 1);
    }

    /// Spec 34.
    #[tokio::test]
    async fn a_sample_failing_every_epoch_is_failed() {
        let ids = ["a"];
        let solver = ScriptedSolver::new(&[("a", Behaviour::Fails)]);
        let config = BatchConfig {
            epochs: 3,
            ..BatchConfig::default()
        };

        let report = run_batch(&samples(&ids), &solver, config)
            .await
            .expect("a non-empty manifest");

        let row = row(&report, "a");
        assert!(matches!(row.status, SampleStatus::Failed { .. }));
        assert_eq!(row.epochs_ok, 0);
        assert_eq!(row.epochs_failed, 3);
    }

    /// Spec 35. The failure that produces confidently wrong numbers: a
    /// transcript scored against somebody else's reference.
    #[tokio::test]
    async fn results_stay_attached_to_their_own_sample() {
        let ids = ["a", "b", "c"];
        let solver = ScriptedSolver::new(&[
            ("a", Behaviour::Slow(Duration::from_millis(60), "first")),
            ("b", Behaviour::Slow(Duration::from_millis(20), "second")),
            ("c", Behaviour::Slow(Duration::from_millis(40), "third")),
        ]);

        let report = run_batch(&samples(&ids), &solver, BatchConfig::default())
            .await
            .expect("a non-empty manifest");

        assert_eq!(row(&report, "a").transcript.as_deref(), Some("first"));
        assert_eq!(row(&report, "b").transcript.as_deref(), Some("second"));
        assert_eq!(row(&report, "c").transcript.as_deref(), Some("third"));

        let order: Vec<&str> = report
            .samples
            .iter()
            .map(|sample| sample.id.as_str())
            .collect();
        assert_eq!(order, ["a", "b", "c"], "rows follow manifest order");
    }

    /// Spec 36. Without this the whole run hangs on one bad recording, and the
    /// slowest samples are exactly the interesting ones.
    #[tokio::test]
    async fn a_hanging_sample_times_out_and_the_rest_complete() {
        let ids = ["a", "b"];
        let solver = ScriptedSolver::new(&[
            ("a", Behaviour::Hangs),
            ("b", Behaviour::Transcribes("the quick brown fox")),
        ]);
        let config = BatchConfig {
            sample_timeout: Duration::from_millis(50),
            ..BatchConfig::default()
        };

        let report = run_batch(&samples(&ids), &solver, config)
            .await
            .expect("a non-empty manifest");

        match &row(&report, "a").status {
            SampleStatus::Failed { reason } => {
                assert!(
                    reason.to_lowercase().contains("timed out"),
                    "the reason should name the timeout, got {reason:?}"
                );
            }
            SampleStatus::Ok => panic!("a hanging sample must not be reported as ok"),
        }
        assert!(matches!(row(&report, "b").status, SampleStatus::Ok));
        assert_eq!(report.summary.samples_ok, 1);
        assert_eq!(report.summary.samples_failed, 1);
    }

    /// Spec 37.
    #[tokio::test]
    async fn an_empty_manifest_is_an_error() {
        let solver = ScriptedSolver::new(&[]);

        let error = run_batch(&[], &solver, BatchConfig::default())
            .await
            .expect_err("nothing to run");

        assert_eq!(error, BatchError::NoSamples);
    }

    /// Spec 38. A report that cannot be read back is not a record.
    #[tokio::test]
    async fn a_report_survives_a_round_trip() {
        let ids = ["a", "b"];
        let solver = ScriptedSolver::new(&[
            ("a", Behaviour::Transcribes("the quick brown fox")),
            ("b", Behaviour::Fails),
        ]);

        let report = run_batch(&samples(&ids), &solver, BatchConfig::default())
            .await
            .expect("a non-empty manifest");

        let encoded = serde_json::to_string(&report).expect("serialisable");
        let decoded: BatchReport = serde_json::from_str(&encoded).expect("deserialisable");

        assert_eq!(decoded, report);
        assert!(
            !decoded.note.is_empty(),
            "the precision caveat travels with the numbers"
        );
    }
}
