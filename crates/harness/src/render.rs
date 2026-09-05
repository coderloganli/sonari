//! Making a report readable.
//!
//! The JSON file is the record; this is what a person looks at. A field name and
//! a number are not readable on their own — `speech_end_ms: 3200` means nothing
//! without knowing where that clip's pause began and how long the recording was.
//! So every measurement is stated relative to what its clip was probing, and the
//! observations that span several clips get their own line, because no single
//! row can carry them.
//!
//! Nothing here judges. It says what happened.

use crate::{
    manifest::Sample,
    report::{BatchReport, SampleReport, SampleStatus},
};

pub fn table(report: &BatchReport, samples: &[Sample]) -> String {
    let mut out = String::new();
    let find = |id: &str| samples.iter().find(|sample| sample.id == id);

    out.push_str(&group(report, samples, "pause", |row, sample| {
        pause_line(row, sample)
    }));
    // The four pause clips only mean something read together: which rung, if
    // any, got cut. That sentence cannot live on any one row.
    if let Some(note) = pause_summary(report, samples) {
        out.push_str(&format!("              {note}\n"));
    }
    out.push('\n');

    let min_utterance_ms = f64::from(report.config.min_utterance_ms);
    out.push_str(&group(report, samples, "short", |row, sample| {
        short_line(row, sample, min_utterance_ms)
    }));
    out.push('\n');

    out.push_str(&group(report, samples, "decay", |row, _| {
        format!(
            "{:<26} wer {:<6} deletions {}",
            "",
            fmt_opt(row.wer, 2),
            row.errors.map_or(0, |errors| errors.deletions)
        )
    }));
    out.push_str(&group(report, samples, "baseline", |row, sample| {
        baseline_line(row, sample)
    }));
    out.push('\n');

    out.push_str(&group(report, samples, "nothing", |row, _| {
        let opened = if row.turn_opened == Some(true) {
            "TURN OPENED"
        } else {
            "no turn"
        };
        let text = row.transcript.as_deref().unwrap_or("").trim();
        if text.is_empty() {
            format!("{opened:<26} no transcript")
        } else {
            format!("{opened:<26} TRANSCRIPT {text:?}")
        }
    }));
    // A caller who says nothing. Reported as a count rather than a verdict:
    // whether the agent ought to speak into a silence is a product question this
    // set does not get to answer, and a line that implied one nearly produced a
    // bug report about correct behaviour.
    out.push_str(&group(report, samples, "idle", |row, _| {
        match row.server_turns.unwrap_or(0) {
            0 => "no server turn at all — not even a greeting".to_owned(),
            1 => "1 server turn (the greeting); the agent did not speak again".to_owned(),
            n => format!("{n} server turns; the agent spoke again unprompted"),
        }
    }));
    out.push_str(&group(report, samples, "malformed", |row, _| {
        match &row.status {
            SampleStatus::Failed { reason } => format!("{:<26} rejected: {reason}", ""),
            SampleStatus::Ok => format!("{:<26} ACCEPTED — the format check did not fire", ""),
        }
    }));

    // Rows the manifest carried but no probe described.
    for row in &report.samples {
        if find(&row.id).and_then(kind).is_none() {
            out.push_str(&format!("{:<18}{}\n", row.id, plain_line(row)));
        }
    }

    out.push('\n');
    out.push_str(&format!(
        "corpus wer {}   ok {}   failed {}   false triggers {} of {}\n",
        fmt_opt(report.summary.corpus_wer, 3),
        report.summary.samples_ok,
        report.summary.samples_failed,
        report.summary.false_triggers,
        report.summary.silent_clips,
    ));
    out.push_str(&format!(
        "system response  p50 {}  p95 {}\n",
        fmt_opt(report.summary.system_response.p50, 0),
        fmt_opt(report.summary.system_response.p95, 0),
    ));
    out.push_str(&format!(
        "perceived        p50 {}  p95 {}\n",
        fmt_opt(report.summary.perceived_latency.p50, 0),
        fmt_opt(report.summary.perceived_latency.p95, 0),
    ));
    let hangover: Vec<f64> = report
        .samples
        .iter()
        .filter_map(|row| row.hangover_cost_ms)
        .collect();
    out.push_str(&format!(
        "hangover cost    p50 {}   — the wait before the turn was judged over\n",
        fmt_opt(crate::metrics::percentile(&hangover, 0.5), 0),
    ));
    out.push_str(&format!("\n{}\n", report.note));
    out
}

fn group(
    report: &BatchReport,
    samples: &[Sample],
    wanted: &str,
    line: impl Fn(&SampleReport, &Sample) -> String,
) -> String {
    let mut out = String::new();
    for row in &report.samples {
        let Some(sample) = samples.iter().find(|sample| sample.id == row.id) else {
            continue;
        };
        if kind(sample).as_deref() != Some(wanted) {
            continue;
        }
        if let SampleStatus::Failed { reason } = &row.status
            && wanted != "malformed"
        {
            out.push_str(&format!("{:<18}FAILED  {reason}\n", row.id));
            continue;
        }
        out.push_str(&format!("{:<18}{}\n", row.id, line(row, sample)));
    }
    out
}

/// Where the turn ended, relative to the start of the clip's gap.
///
/// **The two solvers do not share an origin.** In process, markers are elapsed
/// from the first frame of the recording, which is the same origin the probe's
/// `gap_start_ms` uses. Live, the runtime emits them from the moment it decided
/// speech had begun. Those coincide only while every frame counts as speech —
/// which is today's behaviour, and exactly what ticket 0001 questions. Until
/// that is settled the live figure carries an assumption, and the report says so
/// rather than presenting one number as if both were the same.
fn pause_line(row: &SampleReport, sample: &Sample) -> String {
    let utterances = row.utterance_count.unwrap_or(0);
    let offset = probe_f64(sample, "gap_start_ms")
        .zip(row.markers.speech_end_ms)
        .map(|(gap_start, speech_end)| speech_end - gap_start);
    let ran_out = matches!(
        (offset, probe_f64(sample, "gap_start_ms"), probe_f64(sample, "audio_ms")),
        (Some(offset), Some(start), Some(total)) if (offset - (total - start)).abs() < 250.0
    );
    format!(
        "{utterances} utterance{}  endpoint {:>+7} ms after gap{}  wer {}",
        if utterances == 1 { " " } else { "s" },
        offset.map_or("      ?".to_owned(), |value| format!("{value:+.0}")),
        if ran_out {
            " (= end of audio)"
        } else {
            "                 "
        },
        fmt_opt(row.wer, 2),
    )
}

fn short_line(row: &SampleReport, sample: &Sample, min_utterance_ms: f64) -> String {
    let opened = if row.turn_opened == Some(true) {
        "turn opened"
    } else {
        "NO TURN    "
    };
    let speech_ms = probe_f64(sample, "speech_ms");
    // A clip longer than the threshold is not probing the threshold, whatever
    // it was built for. Saying so beats leaving a reader to notice that 350 is
    // more than 300.
    let probing = match speech_ms {
        Some(ms) if min_utterance_ms > 0.0 && ms >= min_utterance_ms => {
            " (above the threshold — not probing it)"
        }
        _ => "",
    };
    format!(
        "{opened}  speech {} ms{probing}  transcript {:?}",
        fmt_opt(speech_ms, 0),
        row.transcript.as_deref().unwrap_or("").trim(),
    )
}

fn baseline_line(row: &SampleReport, sample: &Sample) -> String {
    let offset = probe_f64(sample, "speech_end_ms")
        .zip(row.markers.speech_end_ms)
        .map(|(spoken_end, decided_end)| decided_end - spoken_end);
    format!(
        "wer {:<6} endpoint {:>+7} ms after speech ended",
        fmt_opt(row.wer, 2),
        offset.map_or("      ?".to_owned(), |value| format!("{value:+.0}")),
    )
}

fn plain_line(row: &SampleReport) -> String {
    match &row.status {
        SampleStatus::Failed { reason } => format!("FAILED  {reason}"),
        SampleStatus::Ok => format!("wer {}", fmt_opt(row.wer, 2)),
    }
}

/// The observation that spans the pause ladder. Any single row leaves it
/// invisible.
fn pause_summary(report: &BatchReport, samples: &[Sample]) -> Option<String> {
    let rungs: Vec<&SampleReport> = report
        .samples
        .iter()
        .filter(|row| {
            samples
                .iter()
                .find(|sample| sample.id == row.id)
                .and_then(kind)
                .as_deref()
                == Some("pause")
        })
        .collect();
    if rungs.is_empty() {
        return None;
    }
    let cut = rungs
        .iter()
        .filter(|row| row.utterance_count.unwrap_or(0) > 1)
        .count();
    // The in-process solver commits at the end of the file, so it never
    // exercises endpointing and cannot support a statement about it. Saying so
    // is the difference between a finding and a false one.
    if report.solver != "live" {
        return Some(format!(
            "the {} solver commits at end of file — endpointing was not exercised, so these rows say nothing about it",
            report.solver
        ));
    }
    Some(match cut {
        0 => "no clip was cut at any gap length".to_owned(),
        n if n == rungs.len() => "every clip was cut, including the shortest gap".to_owned(),
        n => format!("{n} of {} clips were cut", rungs.len()),
    })
}

fn kind(sample: &Sample) -> Option<String> {
    sample
        .probe
        .as_ref()?
        .get("kind")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn probe_f64(sample: &Sample, key: &str) -> Option<f64> {
    sample.probe.as_ref()?.get(key)?.as_f64()
}

fn fmt_opt(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.decimals$}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{markers::Markers, report::Summary};

    fn sample(id: &str, probe: serde_json::Value) -> Sample {
        Sample {
            id: id.to_owned(),
            audio: std::path::PathBuf::from("clip.wav"),
            reference: "one two".to_owned(),
            probe: Some(probe),
        }
    }

    fn row(id: &str, utterances: usize, speech_end_ms: f64) -> SampleReport {
        SampleReport {
            id: id.to_owned(),
            status: SampleStatus::Ok,
            transcript: Some("one two".to_owned()),
            wer: Some(0.0),
            errors: None,
            markers: Markers {
                speech_end_ms: Some(speech_end_ms),
                ..Markers::default()
            },
            system_response_ms: None,
            perceived_latency_ms: None,
            hangover_cost_ms: None,
            utterance_count: Some(utterances),
            server_turns: Some(1),
            turn_opened: Some(true),
            epochs_ok: 1,
            epochs_failed: 0,
        }
    }

    fn report(rows: Vec<SampleReport>) -> BatchReport {
        BatchReport {
            run_at: "2026-08-13T00:00:00Z".to_owned(),
            solver: "live".to_owned(),
            build: "release".to_owned(),
            epochs: 1,
            note: "note".to_owned(),
            config: Default::default(),
            summary: Summary::default(),
            samples: rows,
        }
    }

    /// The number a reader needs is "how long after the pause began", not a
    /// timestamp they would have to place themselves.
    #[test]
    fn a_pause_row_is_measured_from_its_own_gap() {
        let samples = [sample(
            "pause-800",
            serde_json::json!({"kind": "pause", "gap_start_ms": 1340, "gap_ms": 800, "audio_ms": 4790}),
        )];
        let rendered = table(&report(vec![row("pause-800", 1, 4_790.0)]), &samples);

        assert!(
            rendered.contains("+3450 ms after gap"),
            "expected the offset from the gap, got:\n{rendered}"
        );
        assert!(
            rendered.contains("= end of audio"),
            "expected the run-to-the-end note, got:\n{rendered}"
        );
    }

    /// The finding lives across the four rungs, so it gets its own line.
    #[test]
    fn the_ladder_states_what_no_single_row_can() {
        let samples = [
            sample(
                "pause-400",
                serde_json::json!({"kind": "pause", "gap_start_ms": 1000, "gap_ms": 400, "audio_ms": 3000}),
            ),
            sample(
                "pause-800",
                serde_json::json!({"kind": "pause", "gap_start_ms": 1000, "gap_ms": 800, "audio_ms": 3400}),
            ),
        ];
        let rendered = table(
            &report(vec![
                row("pause-400", 1, 3_000.0),
                row("pause-800", 1, 3_400.0),
            ]),
            &samples,
        );

        assert!(
            rendered.contains("no clip was cut at any gap length"),
            "expected the cross-row observation, got:\n{rendered}"
        );
    }

    /// A clip that should have produced nothing and produced words is the most
    /// serious result on the page, so it is not buried in a number.
    #[test]
    fn a_hallucination_is_spelled_out() {
        let samples = [sample("edge-cough", serde_json::json!({"kind": "nothing"}))];
        let mut row = row("edge-cough", 1, 0.0);
        row.transcript = Some("hello there".to_owned());
        let rendered = table(&report(vec![row]), &samples);

        assert!(
            rendered.contains("TURN OPENED") && rendered.contains("TRANSCRIPT"),
            "expected the false trigger to be stated plainly, got:\n{rendered}"
        );
    }
}
