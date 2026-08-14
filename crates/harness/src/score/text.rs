//! What counts as an error.
//!
//! Reference transcripts are written verbatim — fillers, repeats and repairs
//! included — because a cleaned view can be derived from a verbatim one and not
//! the other way round. Normalisation then decides which of those differences
//! are real.
//!
//! Fillers and partial words are removed from **both** sides, which makes them
//! free: getting them right earns nothing, getting them wrong costs nothing.
//! That is the Hub5 convention, and Kaldi's Switchboard recipe states the reason
//! plainly — "if we delete these there is no loss, while if we get them correct
//! there is no gain". It also avoids measuring the wrong thing: commercial
//! recognisers strip fillers by default, so scoring a verbatim reference
//! strictly would report a large deletion rate that reflects a provider setting
//! rather than recognition quality.
//!
//! Apostrophes survive. `can` and `can't` are a reversal of meaning, and a
//! normaliser that ate the suffix would hide it.

/// Words removed from both sides before comparison.
const FILLERS: &[&str] = &[
    "uh", "um", "umm", "uhm", "er", "erm", "ah", "mm", "hmm", "mhm", "huh",
];

/// Colloquial contractions, expanded on both sides. Recognition returns the
/// short form and a verbatim reference usually carries the long one; the
/// difference is spelling, not hearing.
const CONTRACTIONS: &[(&str, &str)] = &[
    ("wanna", "want to"),
    ("gonna", "going to"),
    ("gotta", "got to"),
    ("lemme", "let me"),
    ("gimme", "give me"),
    ("kinda", "kind of"),
    ("sorta", "sort of"),
];

/// Reduces text to the form comparisons are made in.
pub fn normalize(text: &str) -> String {
    let without_events = strip_bracketed(text);

    let mut words: Vec<String> = Vec::new();
    for raw in without_events.split_whitespace() {
        let lowered = raw.to_lowercase();
        // A trailing hyphen marks a partial word — "basi-", "to-". Checked
        // before punctuation is stripped, because stripping would erase the
        // very mark that identifies it, and before interior hyphens are split,
        // because splitting would turn the mark into a separate token.
        if lowered
            .trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '-')
            .ends_with('-')
        {
            continue;
        }
        // An interior hyphen joins two words: "forty-two" is forty and two, and
        // separating them lets the number rule see both.
        for token in lowered.split('-').filter(|part| !part.is_empty()) {
            let cleaned: String = token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .collect();
            if cleaned.is_empty() || FILLERS.contains(&cleaned.as_str()) {
                continue;
            }
            match CONTRACTIONS.iter().find(|(short, _)| *short == cleaned) {
                Some((_, long)) => words.extend(long.split(' ').map(ToOwned::to_owned)),
                None => words.push(cleaned),
            }
        }
    }
    fold_numbers(&words).join(" ")
}

/// Renders spoken numbers the way recognition writes them.
///
/// ElevenLabs Scribe applies inverse text normalisation: "eight two nine one"
/// comes back as `8291`, "forty two" as `42`, "seven thirty" as `7:30`. A
/// verbatim reference writes what was said. Without this, four of the fifteen
/// clips scored errors for recognition that was perfect — the harness was
/// measuring transcript style, which is exactly the failure the normalisation
/// convention exists to prevent.
///
/// Handles what a voice agent actually hears: digit strings, and tens plus a
/// unit. Hundreds and thousands are not spelled out here, and a set that needs
/// them needs a real number grammar rather than a longer table.
fn fold_numbers(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<u32> = Vec::new();

    let flush = |run: &mut Vec<u32>, out: &mut Vec<String>| {
        if run.is_empty() {
            return;
        }
        let mut digits = String::new();
        let mut index = 0;
        while index < run.len() {
            let value = run[index];
            // "forty two" is forty-two; "seven thirty" is not, and reads as the
            // two of them written side by side, which is what `7:30` becomes
            // once punctuation is stripped.
            let merged = (20..=90).contains(&value)
                && value.is_multiple_of(10)
                && run
                    .get(index + 1)
                    .is_some_and(|next| (1..=9).contains(next));
            if merged {
                digits.push_str(&(value + run[index + 1]).to_string());
                index += 2;
            } else {
                digits.push_str(&value.to_string());
                index += 1;
            }
        }
        out.push(digits);
        run.clear();
    };

    for word in words {
        match spoken_number(word) {
            Some(value) => run.push(value),
            None => {
                flush(&mut run, &mut out);
                out.push(word.clone());
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

/// A number word or a run of digits, as a value.
fn spoken_number(word: &str) -> Option<u32> {
    const WORDS: &[(&str, u32)] = &[
        ("zero", 0),
        ("oh", 0),
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
        ("eleven", 11),
        ("twelve", 12),
        ("thirteen", 13),
        ("fourteen", 14),
        ("fifteen", 15),
        ("sixteen", 16),
        ("seventeen", 17),
        ("eighteen", 18),
        ("nineteen", 19),
        ("twenty", 20),
        ("thirty", 30),
        ("forty", 40),
        ("fifty", 50),
        ("sixty", 60),
        ("seventy", 70),
        ("eighty", 80),
        ("ninety", 90),
    ];
    if word.chars().all(|c| c.is_ascii_digit()) && !word.is_empty() {
        return word.parse().ok();
    }
    WORDS
        .iter()
        .find(|(spelling, _)| *spelling == word)
        .map(|(_, value)| *value)
}

/// Removes `[laughter]`, `(coughs)` and anything else bracketed. Unclosed
/// brackets consume the rest of the text, which is the conservative reading:
/// a transcript with a dangling bracket has no reliable content after it.
fn strip_bracketed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0_usize;
    for character in text.chars() {
        match character {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(character),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 1.
    #[test]
    fn lowercases_and_drops_punctuation() {
        assert_eq!(normalize("Hello, World!"), "hello world");
    }

    /// Spec 2.
    #[test]
    fn collapses_whitespace() {
        assert_eq!(normalize("  a   b  "), "a b");
    }

    /// Spec 3. Digits are left alone: the reference writes numbers as spoken,
    /// so there is nothing to convert.
    #[test]
    fn keeps_apostrophes_and_digits() {
        assert_eq!(normalize("It's 25 degrees."), "it's 25 degrees");
    }

    /// Spec 4.
    #[test]
    fn removes_filler_words() {
        assert_eq!(normalize("I uh want um coffee"), "i want coffee");
        assert_eq!(normalize("hmm er ah okay"), "okay");
    }

    /// Spec 5. A partial word is written with a trailing hyphen, the AMI
    /// convention.
    #[test]
    fn removes_partial_words() {
        assert_eq!(
            normalize("I want to go to- to the store"),
            "i want to go to the store"
        );
        assert_eq!(normalize("basi- basically"), "basically");
    }

    /// Spec 6.
    #[test]
    fn removes_bracketed_non_speech_events() {
        assert_eq!(normalize("okay [laughter] sure"), "okay sure");
        assert_eq!(normalize("yes [background noise] please"), "yes please");
    }

    /// Spec 7.
    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
    }

    /// Measured, not assumed: ElevenLabs Scribe returns `8291` for "eight two
    /// nine one", `42` for "forty two" and `7:30` for "seven thirty". Without
    /// folding those to one form, four of fifteen clips scored errors for
    /// recognition that was perfect.
    #[test]
    fn spoken_and_written_numbers_agree() {
        assert_eq!(normalize("eight two nine one"), normalize("8291"));
        assert_eq!(
            normalize("forty two oak street"),
            normalize("42 Oak Street")
        );
        assert_eq!(normalize("seven thirty"), normalize("7:30"));
        assert_eq!(normalize("forty-two"), normalize("42"));
    }

    /// Colloquial contractions are spelling, not hearing.
    #[test]
    fn colloquial_contractions_are_expanded() {
        assert_eq!(normalize("i wanna go"), normalize("i want to go"));
        assert_eq!(normalize("gonna"), "going to");
    }

    /// A number that is not part of a run keeps its own value, and words
    /// between runs keep them apart.
    #[test]
    fn separate_numbers_do_not_merge() {
        assert_eq!(normalize("four people"), "4 people");
        assert_eq!(normalize("two of the four"), "2 of the 4");
    }

    /// Spec 8. The one contraction that must not be flattened: dropping the
    /// suffix turns a refusal into an agreement.
    #[test]
    fn negation_survives_normalisation() {
        assert_ne!(normalize("I can't"), normalize("I can"));
        assert_eq!(normalize("I can't"), "i can't");
    }
}
