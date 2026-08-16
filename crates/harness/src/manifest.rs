//! The evaluation set: one JSON object per line, pairing a recording with what
//! was actually said.
//!
//! Paths resolve against the manifest's own directory rather than the process
//! working directory, so a manifest and its clips move together.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// One recording and its reference transcript.
///
/// `id` is the stable key. Two runs of the same set are lined up by it, so it
/// has to be unique and it has to survive editing the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub id: String,
    pub audio: PathBuf,
    pub reference: String,
    /// How the clip was built, written by the generator because only it knows
    /// where the landmarks actually fell. It is what lets a report express a
    /// measurement relative to what the clip was probing — "the turn ended
    /// 2150 ms after the gap began" rather than a bare timestamp nobody can
    /// place.
    pub probe: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("line {line}: {message}")]
    Line { line: usize, message: String },
    #[error("the manifest contains no samples")]
    Empty,
    #[error("line {line}: duplicate id {id:?}; reports are aligned by id")]
    DuplicateId { line: usize, id: String },
}

/// Parses manifest text. `base_dir` is the directory holding the manifest.
pub fn parse(contents: &str, base_dir: &Path) -> Result<Vec<Sample>, ManifestError> {
    let mut samples: Vec<Sample> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (index, raw) in contents.lines().enumerate() {
        let line = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(raw).map_err(|error| ManifestError::Line {
            line,
            message: error.to_string(),
        })?;
        if entry.id.trim().is_empty() {
            return Err(ManifestError::Line {
                line,
                message: "id must not be empty; reports are aligned by id".to_owned(),
            });
        }
        if !seen.insert(entry.id.clone()) {
            return Err(ManifestError::DuplicateId { line, id: entry.id });
        }
        samples.push(Sample {
            id: entry.id,
            // Relative to the manifest, not the working directory, so a manifest
            // and its clips travel together.
            audio: base_dir.join(entry.audio),
            reference: entry.reference,
            probe: entry.probe,
        });
    }

    if samples.is_empty() {
        return Err(ManifestError::Empty);
    }
    Ok(samples)
}

/// The wire shape. `reference` is required and may be empty — an empty string
/// means "nothing should have been recognised", which is a case in its own
/// right; a missing field means the manifest is incomplete.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    audio: String,
    reference: String,
    #[serde(default)]
    probe: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        PathBuf::from("/evals")
    }

    /// Spec 18.
    #[test]
    fn parses_every_line_in_order() {
        let contents = concat!(
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n",
            r#"{"id":"b","audio":"clips/b.wav","reference":"two"}"#,
            "\n",
            r#"{"id":"c","audio":"clips/c.wav","reference":"three"}"#,
            "\n",
        );

        let samples = parse(contents, &base()).expect("well-formed manifest");

        let ids: Vec<&str> = samples.iter().map(|sample| sample.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"], "manifest order is the report order");
        assert_eq!(samples[1].reference, "two");
    }

    /// Spec 19. Resolving against the working directory would make a manifest
    /// only runnable from one place.
    #[test]
    fn resolves_audio_against_the_manifest_directory() {
        let contents = concat!(
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n"
        );

        let samples = parse(contents, Path::new("/evals")).expect("well-formed manifest");

        assert_eq!(
            samples[0].audio,
            PathBuf::from("/evals").join("clips/a.wav")
        );
    }

    /// Spec 20.
    #[test]
    fn missing_reference_names_the_line() {
        let contents = concat!(
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n",
            r#"{"id":"b","audio":"clips/b.wav"}"#,
            "\n",
        );

        let error = parse(contents, &base()).expect_err("a sample without a reference");

        match error {
            ManifestError::Line { line, .. } => assert_eq!(line, 2),
            other => panic!("expected a line error, got {other:?}"),
        }
    }

    /// Spec 21.
    #[test]
    fn malformed_json_names_the_line() {
        let contents = concat!(
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n",
            "{not json at all\n",
        );

        let error = parse(contents, &base()).expect_err("malformed json");

        match error {
            ManifestError::Line { line, .. } => assert_eq!(line, 2),
            other => panic!("expected a line error, got {other:?}"),
        }
    }

    /// Spec 22. An empty batch would report perfect health over nothing.
    #[test]
    fn empty_manifest_is_an_error() {
        assert_eq!(parse("", &base()), Err(ManifestError::Empty));
        assert_eq!(parse("\n\n  \n", &base()), Err(ManifestError::Empty));
    }

    /// Spec 23.
    #[test]
    fn blank_lines_are_ignored() {
        let contents = concat!(
            "\n",
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n",
            "   \n",
            r#"{"id":"b","audio":"clips/b.wav","reference":"two"}"#,
            "\n",
            "\n",
        );

        let samples = parse(contents, &base()).expect("blank lines are not content");

        assert_eq!(samples.len(), 2);
    }

    /// Spec 24.
    #[test]
    fn duplicate_ids_are_rejected() {
        let contents = concat!(
            r#"{"id":"a","audio":"clips/a.wav","reference":"one"}"#,
            "\n",
            r#"{"id":"a","audio":"clips/b.wav","reference":"two"}"#,
            "\n",
        );

        let error = parse(contents, &base()).expect_err("two samples share an id");

        match error {
            ManifestError::DuplicateId { line, id } => {
                assert_eq!(line, 2);
                assert_eq!(id, "a");
            }
            other => panic!("expected a duplicate-id error, got {other:?}"),
        }
    }
}
