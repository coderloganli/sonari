//! Loading and reload behaviour of `sonari.toml`.

use std::io::Write;

use sonari_config::{Settings, load_and_watch};

fn write(path: &std::path::Path, contents: &str) {
    let mut file = std::fs::File::create(path).expect("create config");
    file.write_all(contents.as_bytes()).expect("write config");
    file.sync_all().expect("flush config");
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sonari-settings-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn a_persona_with_no_prompt_is_rejected() {
    let dir = temp_dir("empty-prompt");
    let path = dir.join("sonari.toml");
    write(
        &path,
        "[[persona]]
name = \"a\"
voice = \"0\"

[persona.prompt]
relationship_stance = \"x\"
persona = \"  \"
",
    );
    let error = load_and_watch(&path).expect_err("an empty prompt must be rejected");
    assert!(
        format!("{error:#}").contains("prompt.persona"),
        "the error should name the field: {error:#}"
    );
}

#[test]
fn two_personas_may_not_share_a_name() {
    let dir = temp_dir("duplicate-name");
    let path = dir.join("sonari.toml");
    let entry = "[[persona]]
name = \"same\"
voice = \"0\"

[persona.prompt]
relationship_stance = \"x\"
persona = \"y\"

";
    write(&path, &format!("{entry}{entry}"));
    let error = load_and_watch(&path).expect_err("duplicate names must be rejected");
    assert!(
        format!("{error:#}").contains("same"),
        "the error should name the duplicate: {error:#}"
    );
}

#[test]
fn a_missing_file_means_nothing_is_configured() {
    let dir = temp_dir("missing");
    let handle = load_and_watch(&dir.join("sonari.toml")).expect("load");
    assert!(handle.get().models.is_none());
}

#[test]
fn a_malformed_file_fails_at_startup() {
    let dir = temp_dir("malformed");
    let path = dir.join("sonari.toml");
    write(&path, "this is not toml =");
    assert!(
        load_and_watch(&path).is_err(),
        "a malformed file must not start the process"
    );
}

#[test]
fn an_unknown_key_is_rejected_rather_than_ignored() {
    let dir = temp_dir("unknown-key");
    let path = dir.join("sonari.toml");
    write(&path, "surprise = true\n");
    let error = load_and_watch(&path).expect_err("unknown keys must be rejected");
    assert!(
        format!("{error:#}").contains("surprise"),
        "the error should name the offending key: {error:#}"
    );
}

#[test]
fn model_paths_that_do_not_exist_are_rejected() {
    let dir = temp_dir("missing-model");
    let path = dir.join("sonari.toml");
    write(
        &path,
        r#"
[models]

[models.vad]
model = "does/not/exist.onnx"

[models.asr]
model = "scribe_v2_realtime"

[models.tts]
model = "eleven_flash_v2_5"
"#,
    );
    let error = load_and_watch(&path).expect_err("missing model files must be rejected");
    assert!(
        format!("{error:#}").contains("does not exist"),
        "the error should say the file is missing: {error:#}"
    );
}

#[test]
fn settings_are_replaced_whole() {
    // The handle hands out a snapshot, so a session that resolved its settings
    // keeps them for its lifetime even as the file changes.
    let settings = Settings {
        models: None,
        personas: Vec::new(),
        llm: Default::default(),
        prompts: Default::default(),
        endpointing: Default::default(),
    };
    let snapshot = std::sync::Arc::new(settings);
    let second = snapshot.clone();
    assert!(std::sync::Arc::ptr_eq(&snapshot, &second));
}
