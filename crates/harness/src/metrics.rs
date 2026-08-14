//! Reduction: across the epochs of one sample, and across the samples of a run.
//!
//! Two choices here are load-bearing.
//!
//! Epochs reduce by **median**. Network jitter is the dominant noise in these
//! measurements and a single spike would drag a mean somewhere no run ever went.
//!
//! Accuracy aggregates as **corpus WER** — total errors over total reference
//! words — not as the mean of per-sample rates. On a set this short the two
//! diverge by multiples: one wrong word in a two-word clip is a rate of 1.0, and
//! averaging rates lets that clip outweigh a twenty-word one.

use crate::score::wer::WordErrors;

/// Linear-interpolated percentile over a sample, `p` in `0.0..=1.0`.
///
/// Interpolation between the two neighbouring ranks, which is the definition
/// pinned by the tests; percentile definitions differ and an unstated one is a
/// number nobody can reproduce.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("measurements are never NaN"));

    let rank = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return Some(sorted[lower]);
    }
    let weight = rank - lower as f64;
    Some(sorted[lower] + (sorted[upper] - sorted[lower]) * weight)
}

pub fn median(values: &[f64]) -> Option<f64> {
    percentile(values, 0.5)
}

/// Total errors over total reference words.
pub fn corpus_wer(errors: &[WordErrors]) -> Option<f64> {
    let words: usize = errors.iter().map(|entry| entry.reference_words).sum();
    if words == 0 {
        return None;
    }
    let total: usize = errors.iter().map(WordErrors::total).sum();
    Some(total as f64 / words as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 25.
    #[test]
    fn percentiles_over_a_sample() {
        let values = [10.0, 20.0, 30.0, 40.0, 100.0];

        assert_eq!(percentile(&values, 0.5), Some(30.0));
        // Linear interpolation between ranks: 0.95 * (5 - 1) = 3.8, so 80% of
        // the way from 40 to 100. Compared with a tolerance because the
        // interpolation lands a fraction below 88 in binary floating point, and
        // rounding belongs in the report rather than in the measurement.
        let p95 = percentile(&values, 0.95).expect("five values");
        assert!((p95 - 88.0).abs() < 1e-9, "expected 88, got {p95}");
    }

    /// Order must not matter — the caller collects results as they finish.
    #[test]
    fn percentiles_do_not_depend_on_input_order() {
        let ordered = [10.0, 20.0, 30.0, 40.0, 100.0];
        let shuffled = [100.0, 10.0, 40.0, 20.0, 30.0];

        assert_eq!(percentile(&ordered, 0.5), percentile(&shuffled, 0.5));
        assert_eq!(percentile(&ordered, 0.95), percentile(&shuffled, 0.95));
    }

    /// Spec 26. With one observation every percentile is that observation.
    #[test]
    fn a_single_value_is_every_percentile() {
        assert_eq!(percentile(&[42.0], 0.5), Some(42.0));
        assert_eq!(percentile(&[42.0], 0.95), Some(42.0));
    }

    /// Spec 27. No figure, rather than a zero that would read as "instant".
    #[test]
    fn no_values_yield_no_percentile() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(median(&[]), None);
    }

    /// Spec 29. Three epochs, one of them a network spike.
    #[test]
    fn median_resists_a_single_spike() {
        assert_eq!(median(&[100.0, 110.0, 900.0]), Some(110.0));
    }

    #[test]
    fn median_of_an_even_count_interpolates() {
        assert_eq!(median(&[10.0, 20.0]), Some(15.0));
    }

    /// Spec 28. The whole reason the mean of per-sample rates is not reported.
    #[test]
    fn corpus_wer_weights_by_reference_length() {
        let errors = [
            WordErrors {
                substitutions: 2,
                deletions: 0,
                insertions: 0,
                reference_words: 10,
            },
            WordErrors {
                substitutions: 1,
                deletions: 0,
                insertions: 0,
                reference_words: 2,
            },
        ];

        let corpus = corpus_wer(&errors).expect("two scored samples");
        assert_eq!(corpus, 0.25, "3 errors over 12 reference words");

        let mean_of_rates: f64 = errors
            .iter()
            .map(|entry| entry.rate().expect("non-empty reference"))
            .sum::<f64>()
            / errors.len() as f64;
        assert!(
            (mean_of_rates - 0.35).abs() < 1e-9,
            "the mean of rates is a different number"
        );
        assert_ne!(corpus, mean_of_rates);
    }

    #[test]
    fn corpus_wer_of_nothing_is_no_figure() {
        assert_eq!(corpus_wer(&[]), None);
    }
}
