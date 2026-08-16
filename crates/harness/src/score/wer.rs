//! Word error rate, with the error types kept apart.
//!
//! A single rate cannot say whether recognition misheard a word or never
//! received it. Substitutions point at the acoustic path; a run of deletions at
//! the end of an utterance points at endpointing committing early, which is the
//! failure this evaluation set is built to expose.

use serde::{Deserialize, Serialize};

/// The edit operations between a reference and a hypothesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WordErrors {
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub reference_words: usize,
}

impl WordErrors {
    pub fn total(&self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    /// Errors over reference length. `None` when the reference is empty, since
    /// there is nothing to be wrong about.
    pub fn rate(&self) -> Option<f64> {
        (self.reference_words > 0).then(|| self.total() as f64 / self.reference_words as f64)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScoreError {
    #[error("the reference is empty; there is no rate to compute")]
    EmptyReference,
}

/// Normalises both sides, then aligns them.
pub fn score(reference: &str, hypothesis: &str) -> Result<WordErrors, ScoreError> {
    let reference: Vec<String> = super::text::normalize(reference)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if reference.is_empty() {
        return Err(ScoreError::EmptyReference);
    }
    let hypothesis: Vec<String> = super::text::normalize(hypothesis)
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    Ok(align(&reference, &hypothesis))
}

/// Levenshtein over words, carrying the operation counts alongside the cost so
/// the breakdown is exact rather than inferred from the totals.
fn align(reference: &[String], hypothesis: &[String]) -> WordErrors {
    #[derive(Clone, Copy, Default)]
    struct Cell {
        cost: usize,
        substitutions: usize,
        deletions: usize,
        insertions: usize,
    }

    let rows = reference.len() + 1;
    let columns = hypothesis.len() + 1;
    let mut previous: Vec<Cell> = (0..columns)
        .map(|insertions| Cell {
            cost: insertions,
            insertions,
            ..Cell::default()
        })
        .collect();

    for row in 1..rows {
        let mut current = vec![Cell::default(); columns];
        current[0] = Cell {
            cost: row,
            deletions: row,
            ..Cell::default()
        };
        for column in 1..columns {
            let matched = reference[row - 1] == hypothesis[column - 1];
            // A match costs nothing, so it is always at least as good as any
            // edit and can be taken without comparing.
            let substitute = Cell {
                cost: previous[column - 1].cost + usize::from(!matched),
                substitutions: previous[column - 1].substitutions + usize::from(!matched),
                ..previous[column - 1]
            };
            let delete = Cell {
                cost: previous[column].cost + 1,
                deletions: previous[column].deletions + 1,
                ..previous[column]
            };
            let insert = Cell {
                cost: current[column - 1].cost + 1,
                insertions: current[column - 1].insertions + 1,
                ..current[column - 1]
            };
            current[column] = [substitute, delete, insert]
                .into_iter()
                .min_by_key(|cell| cell.cost)
                .expect("three candidates");
        }
        previous = current;
    }

    let best = previous[columns - 1];
    WordErrors {
        substitutions: best.substitutions,
        deletions: best.deletions,
        insertions: best.insertions,
        reference_words: reference.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = "the quick brown fox";

    fn rate_of(hypothesis: &str) -> f64 {
        score(REFERENCE, hypothesis)
            .expect("a four-word reference")
            .rate()
            .expect("a non-empty reference")
    }

    /// Spec 9.
    #[test]
    fn identical_text_has_no_errors() {
        let errors = score(REFERENCE, "the quick brown fox").expect("scored");

        assert_eq!(errors.total(), 0);
        assert_eq!(errors.reference_words, 4);
        assert_eq!(errors.rate(), Some(0.0));
    }

    /// Spec 10.
    #[test]
    fn a_wrong_word_is_a_substitution() {
        let errors = score(REFERENCE, "the quick brown dog").expect("scored");

        assert_eq!(errors.substitutions, 1);
        assert_eq!(errors.deletions, 0);
        assert_eq!(errors.insertions, 0);
        assert_eq!(rate_of("the quick brown dog"), 0.25);
    }

    /// Spec 11. The signal for an utterance cut short.
    #[test]
    fn a_missing_word_is_a_deletion() {
        let errors = score(REFERENCE, "the quick fox").expect("scored");

        assert_eq!(errors.deletions, 1);
        assert_eq!(errors.substitutions, 0);
        assert_eq!(errors.insertions, 0);
        assert_eq!(rate_of("the quick fox"), 0.25);
    }

    /// Spec 12.
    #[test]
    fn an_extra_word_is_an_insertion() {
        let errors = score(REFERENCE, "the quick brown red fox").expect("scored");

        assert_eq!(errors.insertions, 1);
        assert_eq!(errors.substitutions, 0);
        assert_eq!(errors.deletions, 0);
        assert_eq!(rate_of("the quick brown red fox"), 0.25);
    }

    /// Spec 13. Five wrong words against a four-word reference is four
    /// substitutions and an insertion — five errors over four words. Word error
    /// rate has no upper bound, and a report that quietly clamped it at 1.0
    /// would hide the worst results it can produce.
    #[test]
    fn unrelated_text_can_exceed_one() {
        assert_eq!(rate_of("a completely different sentence here"), 1.25);
        assert_eq!(rate_of("a completely different sentence"), 1.0);
    }

    /// Spec 14. Nothing recognised at all.
    #[test]
    fn empty_hypothesis_deletes_the_reference() {
        let errors = score(REFERENCE, "").expect("scored");

        assert_eq!(errors.deletions, 4);
        assert_eq!(errors.rate(), Some(1.0));
    }

    /// Spec 15. Not a division by zero.
    #[test]
    fn empty_reference_is_an_error() {
        assert_eq!(score("", "something"), Err(ScoreError::EmptyReference));
    }

    /// Spec 16. Both sides are normalised, so transcript style is not an error.
    #[test]
    fn casing_and_punctuation_are_not_errors() {
        assert_eq!(rate_of("The Quick, Brown Fox!"), 0.0);
    }

    /// Spec 17. A recogniser that drops fillers is not penalised for it.
    #[test]
    fn dropping_a_filler_is_free() {
        let errors = score("i uh want coffee", "i want coffee").expect("scored");

        assert_eq!(errors.total(), 0);
        assert_eq!(
            errors.reference_words, 3,
            "the filler is not counted in the denominator either"
        );
    }
}
