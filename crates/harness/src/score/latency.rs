//! Collecting one marker across the run, ready for percentiles.
//!
//! Samples that never reached a marker contribute nothing rather than a zero.
//! A turn that produced no audio did not respond instantly, and letting it enter
//! the sample as `0` would improve every percentile it touched.

use crate::report::SampleReport;

/// Every value recorded for one marker, in report order.
pub fn series(samples: &[SampleReport], marker: fn(&SampleReport) -> Option<f64>) -> Vec<f64> {
    samples.iter().filter_map(marker).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        markers::Markers,
        report::{SampleReport, SampleStatus},
    };

    fn sample(id: &str, system_response_ms: Option<f64>) -> SampleReport {
        SampleReport {
            id: id.to_owned(),
            status: if system_response_ms.is_some() {
                SampleStatus::Ok
            } else {
                SampleStatus::Failed {
                    reason: "scripted".to_owned(),
                }
            },
            transcript: None,
            wer: None,
            errors: None,
            markers: Markers::default(),
            system_response_ms,
            perceived_latency_ms: None,
            hangover_cost_ms: None,
            utterance_count: None,
            turn_opened: None,
            epochs_ok: 1,
            epochs_failed: 0,
        }
    }

    #[test]
    fn collects_the_values_that_exist() {
        let samples = [
            sample("a", Some(1_200.0)),
            sample("b", Some(1_400.0)),
            sample("c", Some(900.0)),
        ];

        let values = series(&samples, |sample| sample.system_response_ms);

        assert_eq!(values, vec![1_200.0, 1_400.0, 900.0]);
    }

    #[test]
    fn a_sample_without_the_marker_contributes_nothing() {
        let samples = [
            sample("a", Some(1_200.0)),
            sample("b", None),
            sample("c", Some(900.0)),
        ];

        let values = series(&samples, |sample| sample.system_response_ms);

        assert_eq!(
            values,
            vec![1_200.0, 900.0],
            "a missing marker is absent, not zero"
        );
    }
}
