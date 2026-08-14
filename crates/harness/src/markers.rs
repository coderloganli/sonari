//! The eight latency markers of `docs/architecture.md` §7, and the two figures
//! derived from them.
//!
//! Markers are elapsed milliseconds from the first frame of the recording, so a
//! report carries values to read rather than timestamps to subtract. The two
//! derived figures are always reported together (ADR-0010): system response is
//! what the target is set against, perceived latency is what the caller lives
//! through, and the gap between them is the hangover.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Markers {
    pub speech_start_ms: Option<f64>,
    pub speech_last_voiced_ms: Option<f64>,
    pub speech_end_ms: Option<f64>,
    pub asr_final_ms: Option<f64>,
    pub llm_first_token_ms: Option<f64>,
    pub llm_first_sentence_ms: Option<f64>,
    pub tts_first_chunk_ms: Option<f64>,
    pub audio_first_frame_ms: Option<f64>,
}

impl Markers {
    /// `speech_end` → first audio out. The figure the sub-2s target is set
    /// against.
    pub fn system_response_ms(&self) -> Option<f64> {
        Some(self.audio_first_frame_ms? - self.speech_end_ms?)
    }

    /// `speech_last_voiced` → first audio out. Longer than system response by
    /// however long endpointing waited before believing the caller had stopped.
    pub fn perceived_latency_ms(&self) -> Option<f64> {
        Some(self.audio_first_frame_ms? - self.speech_last_voiced_ms?)
    }

    /// What the waiting cost: the gap between the two figures above.
    pub fn hangover_cost_ms(&self) -> Option<f64> {
        Some(self.perceived_latency_ms()? - self.system_response_ms()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers() -> Markers {
        Markers {
            speech_start_ms: Some(200.0),
            speech_last_voiced_ms: Some(1_500.0),
            speech_end_ms: Some(2_200.0),
            asr_final_ms: Some(2_320.0),
            llm_first_token_ms: Some(2_789.0),
            llm_first_sentence_ms: Some(2_856.0),
            tts_first_chunk_ms: Some(3_490.0),
            audio_first_frame_ms: Some(3_525.0),
        }
    }

    #[test]
    fn system_response_is_measured_from_the_endpoint() {
        assert_eq!(markers().system_response_ms(), Some(1_325.0));
    }

    #[test]
    fn perceived_latency_is_measured_from_the_last_voiced_frame() {
        assert_eq!(markers().perceived_latency_ms(), Some(2_025.0));
    }

    /// The difference between the two is the endpointing wait, which is the
    /// whole reason ADR-0010 insists on reporting both.
    #[test]
    fn the_pair_differs_by_the_endpointing_wait() {
        let markers = markers();
        let perceived = markers.perceived_latency_ms().expect("complete markers");
        let response = markers.system_response_ms().expect("complete markers");

        assert_eq!(perceived - response, 700.0);
    }

    /// A turn that never produced audio has no figure, rather than a zero that
    /// would flatter the percentiles.
    #[test]
    fn missing_markers_yield_no_figure() {
        let incomplete = Markers {
            audio_first_frame_ms: None,
            ..markers()
        };

        assert_eq!(incomplete.system_response_ms(), None);
        assert_eq!(incomplete.perceived_latency_ms(), None);
    }
}
