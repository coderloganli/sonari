//! Reading a finished call back out of its recorded events.
//!
//! The live solver drives the real service, so it sees only audio going in and
//! audio coming out. Everything else — where the turn ended, what was
//! recognised, how long each stage took — is read afterwards from the call
//! events the runtime already publishes, over
//! `/api/admin/call-logs/{session_id}/timeline`.
//!
//! Nothing here is specific to how the call was made, which is the point: an
//! `Outcome` assembled from a timeline is the same shape as one measured in
//! process, so scoring, aggregation and the report do not know the difference.

use serde::Deserialize;

use crate::{markers::Markers, solver::Outcome};

/// One recorded event, as the timeline endpoint returns it.
#[derive(Debug, Clone, Deserialize)]
pub struct TimelineEvent {
    pub event: String,
    #[serde(default)]
    pub round_id: Option<String>,
    #[serde(default)]
    pub ts_ms: i64,
    #[serde(default)]
    pub fields: serde_json::Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error(
        "speech was detected but no speech_turn_latency event followed; the turn started and never completed"
    )]
    TurnNeverCompleted,
}

/// Names, kept together because they are a contract with the runtime rather
/// than local detail.
const TURN_LATENCY: &str = "speech_turn_latency";
const SPEECH_DETECTED: &str = "speech_detected";
const UTTERANCE_FLUSHING: &str = "speech_utterance_flushing";
const ASR_FINAL: &str = "speech_asr_final_received";
/// When the runtime gives up waiting and commits the partial it has, the
/// transcript arrives under this name instead. It is the text the agent was
/// actually driven with, so scoring without it would report a recognition
/// failure for a turn that recognised something.
const ASR_FORCED: &str = "speech_asr_forced_finalized";

/// Assembles the first completed turn in the timeline.
///
/// A clip is one utterance from the caller, so the first turn is the one being
/// measured; later turns, if the endpoint split the recording, are counted but
/// not scored.
pub fn assemble(events: &[TimelineEvent]) -> Result<Outcome, TimelineError> {
    let turn_opened = events.iter().any(|entry| entry.event == SPEECH_DETECTED);
    let Some(turn) = events.iter().find(|entry| entry.event == TURN_LATENCY) else {
        // No turn and no speech is the correct outcome for a clip that should
        // produce nothing — silence, a cough — and reporting it as a failure
        // would lose the false-trigger measurement those clips exist for.
        //
        // No turn *after* speech was detected is a real failure: something
        // started and never finished.
        if turn_opened {
            return Err(TimelineError::TurnNeverCompleted);
        }
        return Ok(Outcome {
            transcript: String::new(),
            reply: String::new(),
            markers: Markers::default(),
            utterance_count: 0,
            turn_opened: false,
        });
    };

    let round = turn.round_id.as_deref();
    let ms = |key: &str| turn.fields.get(key).and_then(serde_json::Value::as_f64);

    let transcript = events
        .iter()
        .filter(|entry| {
            (entry.event == ASR_FINAL || entry.event == ASR_FORCED) && matches_round(entry, round)
        })
        .find_map(|entry| {
            entry
                .fields
                .get("transcript")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or_default()
        .to_owned();

    // How many times the runtime decided an utterance had ended. One means the
    // recording was never cut; two means endpointing split it, which is the
    // observation the pause clips exist to make.
    let utterance_count = events
        .iter()
        .filter(|entry| entry.event == UTTERANCE_FLUSHING)
        .count();

    Ok(Outcome {
        transcript,
        reply: String::new(),
        markers: Markers {
            speech_start_ms: ms("speech_start_ms"),
            speech_last_voiced_ms: ms("speech_last_voiced_ms"),
            speech_end_ms: ms("speech_end_ms"),
            asr_final_ms: ms("asr_final_ms"),
            llm_first_token_ms: ms("llm_first_token_ms"),
            llm_first_sentence_ms: ms("llm_first_sentence_ms"),
            tts_first_chunk_ms: ms("tts_first_chunk_ms"),
            // Not wired in this version; see the known gaps in spec.md. Falling
            // back to the synthesis marker keeps system response computable, and
            // makes it optimistic by the mixer and playback, which the report
            // says.
            audio_first_frame_ms: ms("audio_first_frame_ms").or_else(|| ms("tts_first_chunk_ms")),
        },
        utterance_count,
        turn_opened: events.iter().any(|entry| entry.event == SPEECH_DETECTED),
    })
}

/// An event belongs to the turn if it names the same round. Events without a
/// round — session-level ones — belong to whatever turn is being read.
fn matches_round(entry: &TimelineEvent, round: Option<&str>) -> bool {
    match (entry.round_id.as_deref(), round) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, round: Option<&str>, fields: serde_json::Value) -> TimelineEvent {
        TimelineEvent {
            event: name.to_owned(),
            round_id: round.map(ToOwned::to_owned),
            ts_ms: 0,
            fields,
        }
    }

    fn latency(round: &str, speech_end: f64) -> TimelineEvent {
        event(
            TURN_LATENCY,
            Some(round),
            serde_json::json!({
                "speech_start_ms": 0.0,
                "speech_last_voiced_ms": 1_300.0,
                "speech_end_ms": speech_end,
                "asr_final_ms": 2_120.0,
                "llm_first_token_ms": 2_589.0,
                "llm_first_sentence_ms": 2_656.0,
                "tts_first_chunk_ms": 3_290.0,
                "audio_first_frame_ms": serde_json::Value::Null,
                "system_response_ms": 1_290.0,
                "perceived_latency_ms": 1_990.0,
            }),
        )
    }

    /// Spec 48. The shape has to match what the in-process solver produces, or
    /// the two are not comparable.
    #[test]
    fn a_timeline_becomes_an_outcome() {
        let events = [
            event(SPEECH_DETECTED, Some("r1"), serde_json::json!({})),
            event(UTTERANCE_FLUSHING, Some("r1"), serde_json::json!({})),
            event(
                ASR_FINAL,
                Some("r1"),
                serde_json::json!({ "transcript": "my order number is eight two nine one" }),
            ),
            latency("r1", 2_000.0),
        ];

        let outcome = assemble(&events).expect("a completed turn");

        assert_eq!(outcome.transcript, "my order number is eight two nine one");
        assert_eq!(outcome.utterance_count, 1);
        assert!(outcome.turn_opened);
        assert_eq!(outcome.markers.speech_end_ms, Some(2_000.0));
        assert_eq!(
            outcome.markers.audio_first_frame_ms,
            Some(3_290.0),
            "falls back to the synthesis marker while the playback one is unwired"
        );
        assert_eq!(outcome.markers.system_response_ms(), Some(1_290.0));
    }

    /// Spec 49. Speech that started and never finished is a failure with a
    /// reason, not an empty result that would quietly score as a total
    /// recognition failure.
    #[test]
    fn speech_that_never_completed_is_an_error() {
        let events = [
            event(SPEECH_DETECTED, Some("r1"), serde_json::json!({})),
            event(UTTERANCE_FLUSHING, Some("r1"), serde_json::json!({})),
        ];

        assert_eq!(assemble(&events), Err(TimelineError::TurnNeverCompleted));
    }

    /// A clip that should produce nothing and produced nothing is the answer,
    /// not a failure. Reporting it as failed would drop it out of the aggregate
    /// and take the false-trigger rate with it — losing the measurement the
    /// silence and cough clips exist to make.
    #[test]
    fn a_clip_that_produced_nothing_is_a_result() {
        let events = [event("call_start_requested", None, serde_json::json!({}))];

        let outcome = assemble(&events).expect("nothing happening is an outcome");

        assert!(!outcome.turn_opened);
        assert!(outcome.transcript.is_empty());
        assert_eq!(outcome.utterance_count, 0);
        assert_eq!(outcome.markers.speech_end_ms, None);
    }

    /// Spec 50. When endpointing split the recording, the first turn is the one
    /// scored — and the split itself is visible in `utterance_count`.
    #[test]
    fn a_split_recording_scores_the_first_turn_and_counts_both() {
        let events = [
            event(SPEECH_DETECTED, Some("r1"), serde_json::json!({})),
            event(UTTERANCE_FLUSHING, Some("r1"), serde_json::json!({})),
            event(
                ASR_FINAL,
                Some("r1"),
                serde_json::json!({ "transcript": "my order number is" }),
            ),
            latency("r1", 2_000.0),
            event(UTTERANCE_FLUSHING, Some("r2"), serde_json::json!({})),
            event(
                ASR_FINAL,
                Some("r2"),
                serde_json::json!({ "transcript": "eight two nine one" }),
            ),
            latency("r2", 4_500.0),
        ];

        let outcome = assemble(&events).expect("a completed turn");

        assert_eq!(
            outcome.transcript, "my order number is",
            "the first turn is the one measured"
        );
        assert_eq!(
            outcome.utterance_count, 2,
            "and the split is what the pause clips are looking for"
        );
        assert_eq!(outcome.markers.speech_end_ms, Some(2_000.0));
    }

    /// A turn the runtime forced to a close carries its transcript under a
    /// different name. Ignoring it would score a turn that recognised something
    /// as a total recognition failure.
    #[test]
    fn a_forced_final_still_supplies_the_transcript() {
        let events = [
            event(SPEECH_DETECTED, Some("r1"), serde_json::json!({})),
            event(
                ASR_FORCED,
                Some("r1"),
                serde_json::json!({ "transcript": "my order number is" }),
            ),
            latency("r1", 2_000.0),
        ];

        let outcome = assemble(&events).expect("a completed turn");

        assert_eq!(outcome.transcript, "my order number is");
    }

    /// A recording that never opened a turn — silence, a cough — is not an
    /// error; it is the answer those clips are asking for.
    #[test]
    fn a_turn_that_never_opened_is_reported_as_such() {
        let events = [latency("r1", 0.0)];

        let outcome = assemble(&events).expect("an event exists");

        assert!(!outcome.turn_opened);
        assert!(outcome.transcript.is_empty());
        assert_eq!(outcome.utterance_count, 0);
    }
}
