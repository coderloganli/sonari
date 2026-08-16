//! The real path: ElevenLabs recognition, the model, synthesis.
//!
//! Skipped unless `ELEVENLABS_API_KEY` is set, so a clone without credentials
//! still tests green and CI performs no network calls and incurs no provider
//! cost — the same shape as `crates/providers/tests/sherpa_vad.rs`, which skips
//! without `SONARI_MODELS_DIR`.
//!
//! The bounds asserted here are deliberately loose. Real figures move between
//! runs; asserting exact values produces a test that fails daily and teaches
//! everyone to ignore it. Regressions are caught by comparing reports.

use std::{path::PathBuf, time::Duration};

use harness::{
    manifest,
    report::SampleStatus,
    runner::{BatchConfig, run_batch},
    solver::single_turn::SingleTurnSolver,
};

/// A solver, or `None` when this machine cannot build one.
///
/// Both a key and a readable `sonari.toml` are needed, and a clone has neither.
/// Skipping on the solver itself rather than on the key alone keeps the tests
/// honest wherever they are run from: the working directory decides whether the
/// settings file is visible, and a panic there would look like a defect rather
/// than a missing environment.
fn solver() -> Option<SingleTurnSolver> {
    std::env::var("ELEVENLABS_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())?;
    SingleTurnSolver::from_environment().ok()
}

fn evals_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../evals")
}

/// The first two clips of the generated set, so the real path is exercised
/// without paying for a full run. Derived rather than a separate file: a fixture
/// nobody generates is a test that never runs.
fn smoke_manifest() -> Option<Vec<manifest::Sample>> {
    let dir = evals_dir();
    let contents = std::fs::read_to_string(dir.join("set.jsonl")).ok()?;
    let head: String = contents.lines().take(2).collect::<Vec<_>>().join(
        "
",
    );
    manifest::parse(&head, &dir).ok()
}

fn config() -> BatchConfig {
    BatchConfig {
        solver: "single-turn",
        snapshot: Default::default(),
        epochs: 1,
        sample_timeout: Duration::from_secs(30),
    }
}

/// Spec 42, 43, 44. One run, three assertions: it completed, it recognised
/// something close to the reference, and the clock ran.
#[tokio::test]
async fn a_real_run_produces_a_usable_report() {
    let Some(solver) = solver() else {
        eprintln!("skipped: no credentials or no readable sonari.toml");
        return;
    };
    let Some(samples) = smoke_manifest() else {
        eprintln!("skipped: evals/smoke.jsonl is not present");
        return;
    };
    let report = run_batch(&samples, &solver, config())
        .await
        .expect("a non-empty manifest");

    // Spec 42.
    assert_eq!(report.samples.len(), samples.len());
    for sample in &report.samples {
        assert!(
            matches!(sample.status, SampleStatus::Ok),
            "{} failed: {:?}",
            sample.id,
            sample.status
        );

        // Spec 43. A loose bound: it catches "recognised nothing" and
        // "references are mismatched", not a change in quality.
        let transcript = sample.transcript.as_deref().unwrap_or_default();
        assert!(
            !transcript.trim().is_empty(),
            "{} transcribed nothing",
            sample.id
        );
        let wer = sample.wer.expect("a scored sample");
        assert!(
            wer < 0.5,
            "{} scored {wer}, which suggests a mismatch rather than a regression",
            sample.id
        );

        // Spec 44. Catches a clock that never started and a run that hung.
        let response = sample.system_response_ms.expect("a completed turn");
        assert!(
            response > 0.0 && response < 10_000.0,
            "{} responded in {response} ms",
            sample.id
        );
        let perceived = sample.perceived_latency_ms.expect("a completed turn");
        assert!(
            perceived >= response,
            "perceived latency includes the endpointing wait, so it cannot be shorter"
        );
    }
}

/// Spec 45. A missing recording is one bad row, not a dead batch.
#[tokio::test]
async fn a_missing_recording_fails_only_its_own_sample() {
    let Some(solver) = solver() else {
        eprintln!("skipped: no credentials or no readable sonari.toml");
        return;
    };

    let dir = evals_dir();
    let contents = format!(
        "{}\n",
        r#"{"id":"absent","audio":"clips/does-not-exist.wav","reference":"the quick brown fox"}"#
    );
    let samples = manifest::parse(&contents, &dir).expect("well-formed manifest");

    let report = run_batch(&samples, &solver, config())
        .await
        .expect("a non-empty manifest");

    match &report.samples[0].status {
        SampleStatus::Failed { reason } => {
            assert!(
                reason.contains("does-not-exist"),
                "the reason should name the file, got {reason:?}"
            );
        }
        SampleStatus::Ok => panic!("a missing recording cannot succeed"),
    }
    assert_eq!(report.summary.samples_failed, 1);
}

/// Spec 46. The pipeline carries 16 kHz mono; anything else is rejected with a
/// reason rather than silently resampled into wrong numbers.
#[tokio::test]
async fn a_wrongly_formatted_recording_states_the_mismatch() {
    let Some(solver) = solver() else {
        eprintln!("skipped: no credentials or no readable sonari.toml");
        return;
    };
    let dir = evals_dir();
    if !dir.join("clips/edge-8khz-stereo.wav").exists() {
        eprintln!("skipped: the malformed fixture is not present");
        return;
    }

    let contents = format!(
        "{}\n",
        r#"{"id":"malformed","audio":"clips/edge-8khz-stereo.wav","reference":"what time do you close on sundays"}"#
    );
    let samples = manifest::parse(&contents, &dir).expect("well-formed manifest");

    let report = run_batch(&samples, &solver, config())
        .await
        .expect("a non-empty manifest");

    match &report.samples[0].status {
        SampleStatus::Failed { reason } => {
            let reason = reason.to_lowercase();
            assert!(
                reason.contains("8000") || reason.contains("channel") || reason.contains("hz"),
                "the reason should name the format mismatch, got {reason:?}"
            );
        }
        SampleStatus::Ok => panic!("an 8 kHz stereo recording cannot be measured"),
    }
}
