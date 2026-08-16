//! The evaluation harness.
//!
//!     sonari-eval generate                          build the clip set
//!     sonari-eval run evals/set.jsonl --epochs 3    measure the components
//!     sonari-eval run evals/set.jsonl --live        measure the running service
//!     sonari-eval recording.wav                     one clip, as before
//!
//! Timings are only meaningful from a release build. A debug build inflated one
//! stage by half again, which would point optimisation at the wrong place.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use harness::{
    manifest, render,
    runner::{BatchConfig, run_batch},
    solver::{Solver, single_turn::SingleTurnSolver},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    // Without this the adapters' warnings vanish, and a stalled upload looks
    // like a mystery instead of a logged one.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("generate") => generate(&args[1..]).await,
        Some("run") => run(&args[1..]).await,
        Some(path) if path.ends_with(".wav") => run_single(PathBuf::from(path)).await,
        _ => {
            bail!(
                "usage:\n  \
                 sonari-eval generate [--out evals]\n  \
                 sonari-eval run <manifest.jsonl> [--epochs N] [--out DIR] [--live]\n  \
                 sonari-eval <recording.wav>"
            )
        }
    }
}

async fn generate(args: &[String]) -> Result<()> {
    let out_dir = flag(args, "--out").map_or_else(|| PathBuf::from("evals"), PathBuf::from);
    let voice = harness::generate::ElevenLabsVoice::from_environment()?;

    println!("building the evaluation set into {}", out_dir.display());
    let manifest = harness::generate::build(&voice, &out_dir).await?;
    let path = out_dir.join("set.jsonl");
    std::fs::write(&path, &manifest)
        .with_context(|| format!("failed to write {}", path.display()))?;
    println!(
        "{} clips, manifest at {}",
        manifest.lines().count(),
        path.display()
    );
    Ok(())
}

async fn run(args: &[String]) -> Result<()> {
    let manifest_path = positional(args)
        .map(PathBuf::from)
        .context("usage: sonari-eval run <manifest.jsonl>")?;
    let epochs = match flag(args, "--epochs") {
        // Zero would run once and report none, which is a report that
        // contradicts itself.
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|epochs| *epochs > 0)
            .context("--epochs takes a whole number of runs, at least one")?,
        None => 1,
    };
    let out_dir = flag(args, "--out").map_or_else(|| PathBuf::from("evals/runs"), PathBuf::from);
    let live = args.iter().any(|value| value == "--live");

    let contents = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or(std::path::Path::new("."));
    let samples = manifest::parse(&contents, base_dir)?;

    let solver_name = if live { "live" } else { "single-turn" };
    let solver: Box<dyn Solver> = if live {
        #[cfg(feature = "live")]
        {
            Box::new(harness::solver::live_call::LiveCallSolver::from_environment()?)
        }
        #[cfg(not(feature = "live"))]
        {
            bail!(
                "this binary was built without the live solver; rebuild with \
                 --features live (Linux only, see docs/architecture.md §10)"
            )
        }
    } else {
        Box::new(SingleTurnSolver::from_environment()?)
    };

    let report = run_batch(
        &samples,
        solver.as_ref(),
        BatchConfig {
            solver: solver_name,
            snapshot: snapshot(),
            epochs,
            sample_timeout: Duration::from_secs(60),
        },
    )
    .await?;

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;
    let path = out_dir.join(format!("{}.json", report.run_at.replace(':', "-")));
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("failed to write {}", path.display()))?;

    // The file is the record; this is what a person reads.
    print!("{}", render::table(&report, &samples));
    println!("\nreport written to {}", path.display());
    Ok(())
}

/// The original single-clip form, kept because it is still the quickest way to
/// look at one recording.
async fn run_single(path: PathBuf) -> Result<()> {
    let sample = manifest::Sample {
        id: path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("clip")
            .to_owned(),
        audio: path,
        reference: String::new(),
        probe: None,
    };
    let solver = SingleTurnSolver::from_environment()?;
    let outcome = solver
        .run(&sample)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    // A clip handed over by name is speech somebody wanted transcribed; the
    // empty-reference contract that lets silence be a result belongs to the
    // evaluation set, not here.
    if outcome.transcript.trim().is_empty() {
        bail!("recognition produced nothing from {}", sample.id);
    }

    println!(
        "{}",
        serde_json::json!({
            "event": "eval_turn",
            "recording": sample.id,
            "transcript": outcome.transcript.trim(),
            "reply": outcome.reply.trim(),
            "markers": outcome.markers,
            "system_response_ms": outcome.markers.system_response_ms(),
            "perceived_latency_ms": outcome.markers.perceived_latency_ms(),
        })
    );
    Ok(())
}

/// What is being measured, read from the same settings the service reads.
///
/// For `--live` this describes the settings file the harness can see; the
/// authoritative per-turn values travel with each `speech_turn_latency` event,
/// which is what a run against a differently configured host should be read
/// against.
fn snapshot() -> harness::report::ConfigSnapshot {
    let Ok(settings) = sonari_config::load_and_watch(&sonari_config::config_path()) else {
        return harness::report::ConfigSnapshot::default();
    };
    let settings = settings.get();
    let endpointing = &settings.endpointing;
    harness::report::ConfigSnapshot {
        asr_model: settings
            .models
            .as_ref()
            .map(|models| models.asr.model.clone())
            .unwrap_or_default(),
        tts_model: settings
            .models
            .as_ref()
            .map(|models| models.tts.model.clone())
            .unwrap_or_default(),
        llm_model: settings.llm.model.clone(),
        silence_flush_ms: endpointing.silence_flush_ms,
        min_utterance_ms: endpointing.min_utterance_ms,
        min_speech_confirm_ms: endpointing.min_speech_confirm_ms,
        voice_activity_threshold: 0,
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let index = args.iter().position(|value| value == name)?;
    args.get(index + 1).map(String::as_str)
}

/// The first argument that is neither a flag nor a flag's value.
///
/// Skipping only `--`-prefixed words would take the `3` in
/// `run --epochs 3 set.jsonl` for the manifest — a natural way to type the
/// command, and one that would otherwise fail looking for a file called `3`.
fn positional(args: &[String]) -> Option<&str> {
    const TAKES_A_VALUE: &[&str] = &["--epochs", "--out"];

    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if TAKES_A_VALUE.contains(&argument) {
            index += 2;
        } else if argument.starts_with("--") {
            index += 1;
        } else {
            return Some(argument);
        }
    }
    None
}
