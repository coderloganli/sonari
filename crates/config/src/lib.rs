//! `sonari.toml` — the file-backed half of configuration.
//!
//! Environment variables carry endpoints, credentials and the database DSN.
//! This file carries everything an operator edits: model paths, voices and
//! personas. It is the source of truth, versioned in git, and watched so that
//! editing it does not require a restart.
//!
//! A change is parsed and validated before it replaces the live settings. An
//! invalid file leaves the running configuration untouched and logs loudly; an
//! invalid file at startup is fatal.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result, bail};
use providers::{AsrConfig, TtsConfig, VadConfig};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaConfig {
    /// Identifies the persona everywhere: to the client, in the session row, and
    /// in the logs.
    pub name: String,
    /// Speaker within the synthesis model.
    pub voice: String,
    #[serde(default = "default_language")]
    pub language: String,
    pub prompt: PromptConfig,
    #[serde(default)]
    pub scene: Option<SceneConfig>,
}

fn default_language() -> String {
    "en".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptConfig {
    pub relationship_stance: String,
    #[serde(default)]
    pub age: i32,
    #[serde(default)]
    pub marital_status: String,
    #[serde(default)]
    pub occupation: String,
    /// The character's own description of itself.
    pub persona: String,
    #[serde(default)]
    pub private_interests: Vec<String>,
    #[serde(default)]
    pub personality_traits: String,
    #[serde(default)]
    pub speaking_style: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneConfig {
    pub name: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub user_role: String,
    #[serde(default)]
    pub relationship: String,
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub opening_event: String,
    #[serde(default = "default_time_period_mode")]
    pub time_period_mode: String,
    #[serde(default)]
    pub time_period: Option<String>,
}

fn default_time_period_mode() -> String {
    "unspecified".to_owned()
}

/// Where the file lives unless `SONARI_CONFIG` says otherwise.
const DEFAULT_PATH: &str = "sonari.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Absent means no models are configured and voice is unavailable.
    #[serde(default)]
    pub models: Option<ModelSettings>,
    /// Who the agent is. Empty means no call can start.
    #[serde(default, rename = "persona")]
    pub personas: Vec<PersonaConfig>,
    /// How the model is asked to behave. Where it lives and how to authenticate
    /// are deployment concerns and come from the environment.
    #[serde(default)]
    pub llm: LlmSettings,
    /// The instructions wrapped around a persona.
    #[serde(default)]
    pub prompts: PromptTemplates,
    /// When a turn starts and when it ends.
    #[serde(default)]
    pub endpointing: EndpointingSettings,
    /// What the agent remembers about a caller between calls.
    #[serde(default)]
    pub memory: MemorySettings,
}

/// Long-term memory (ADR-0021, ADR-0022).
///
/// `sonari.toml.example` switches this on, because a clean clone should show
/// what the system does. The *absence* of the section leaves it off, which is a
/// different question: a configuration file written before memory existed should
/// not silently start extracting and sending notes to the model provider.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySettings {
    #[serde(default)]
    pub enabled: bool,
    /// Completed turns between extractions.
    #[serde(default = "default_extract_every_turns")]
    pub extract_every_turns: i32,
    /// The most facts one caller may have with one persona. This is what bounds
    /// how long the prompt gets.
    #[serde(default = "default_max_facts")]
    pub max_facts: usize,
    /// Per category, so a talkative week of passing news cannot evict the
    /// caller's name.
    #[serde(default = "default_max_facts_per_category")]
    pub max_facts_per_category: usize,
    /// Which model does the extracting. Empty means the conversation model.
    /// A cheaper one is usually right: this is extraction, not conversation.
    #[serde(default)]
    pub model: String,
}

fn default_extract_every_turns() -> i32 {
    4
}

fn default_max_facts() -> usize {
    40
}

fn default_max_facts_per_category() -> usize {
    12
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            extract_every_turns: default_extract_every_turns(),
            max_facts: default_max_facts(),
            max_facts_per_category: default_max_facts_per_category(),
            model: String::new(),
        }
    }
}

/// Decides the boundaries of a turn from the voice activity signal.
///
/// These are tuned by ear against real calls, so they live beside the persona
/// rather than in a table: changing one is an experiment, and an experiment
/// wants a diff.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointingSettings {
    /// Silence after speech before the turn is considered over.
    #[serde(default = "default_silence_flush_ms")]
    pub silence_flush_ms: u32,
    /// How long to wait for recognition's final result before going ahead with
    /// the partial it has.
    ///
    /// The name reads like "speak first if the caller is silent", and it is not
    /// that: the deadline is set at commit time, minus `silence_flush_ms`, and
    /// spends itself in `take_forced_transcript_turn_if_overdue`. A caller who
    /// says nothing gets no turn from it. Measured, after the name sent an
    /// evaluation clip looking for behaviour that was never promised.
    #[serde(default = "default_silence_force_agent_ms")]
    pub silence_force_agent_ms: u32,
    /// Shortest utterance worth sending. Below this it is a cough.
    #[serde(default = "default_min_utterance_ms")]
    pub min_utterance_ms: u32,
    /// Speech must persist this long before a turn opens. Without it, brief
    /// background noise starts turns nobody began.
    #[serde(default = "default_min_speech_confirm_ms")]
    pub min_speech_confirm_ms: u32,
    /// Mean absolute sample value, above which a frame counts as voice.
    ///
    /// It has to sit above the room's noise floor and below speech. At zero
    /// every frame is voice, silence is never observed, and a turn never ends on
    /// its own — which is exactly what happened until this became configurable.
    #[serde(default = "default_voice_activity_threshold")]
    pub voice_activity_threshold: i16,
}

fn default_silence_flush_ms() -> u32 {
    700
}
fn default_silence_force_agent_ms() -> u32 {
    8_000
}
fn default_min_utterance_ms() -> u32 {
    300
}
/// A −60 dBFS noise floor sits near 33 on this scale and measured speech runs in
/// the thousands, so this clears the floor without reaching for the quiet end of
/// a sentence. A starting point to be measured, not a tuned value.
fn default_voice_activity_threshold() -> i16 {
    300
}

fn default_min_speech_confirm_ms() -> u32 {
    150
}

impl Default for EndpointingSettings {
    fn default() -> Self {
        Self {
            silence_flush_ms: default_silence_flush_ms(),
            silence_force_agent_ms: default_silence_force_agent_ms(),
            min_utterance_ms: default_min_utterance_ms(),
            min_speech_confirm_ms: default_min_speech_confirm_ms(),
            voice_activity_threshold: default_voice_activity_threshold(),
        }
    }
}

/// Assembled in order into the system prompt. Placeholders in each are filled
/// from the persona.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptTemplates {
    /// How to behave in a voice call at all — length, pacing, turn-taking.
    #[serde(default)]
    pub conversation_system: String,
    /// Who the character is.
    #[serde(default)]
    pub character: String,
    /// Where the conversation takes place.
    #[serde(default)]
    pub scene: String,
    /// The opening line of a call the agent starts.
    #[serde(default)]
    pub welcome: String,
    /// How the model is asked to turn a conversation into facts. Carries
    /// `{{max_facts}}`, `{{max_facts_per_category}}` and `{{categories}}`.
    #[serde(default)]
    pub memory_extraction: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmSettings {
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub frequency_penalty: f64,
}

fn default_temperature() -> f64 {
    0.8
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            model: String::new(),
            temperature: default_temperature(),
            frequency_penalty: 0.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    /// The one model that runs in-process.
    pub vad: VadConfig,
    /// Recognition and synthesis are reached over the network; these say which
    /// models to ask for, not where they live. Keys are environment.
    pub asr: AsrConfig,
    pub tts: TtsConfig,
}

impl Settings {
    fn validate(&self) -> Result<()> {
        let Some(models) = &self.models else {
            return Ok(());
        };
        // Only the local model is a file on disk.
        if !Path::new(&models.vad.model).is_file() {
            bail!(
                "models.vad.model points at a file that does not exist: {}",
                models.vad.model
            );
        }
        if models.asr.model.trim().is_empty() {
            bail!("models.asr.model must name a recognition model");
        }
        if models.tts.model.trim().is_empty() {
            bail!("models.tts.model must name a synthesis model");
        }
        Ok(())
    }
}

impl Settings {
    fn validate_personas(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for persona in &self.personas {
            if persona.name.trim().is_empty() {
                bail!("every persona needs a name");
            }
            if !seen.insert(persona.name.as_str()) {
                bail!("two personas are both named '{}'", persona.name);
            }
            if persona.prompt.persona.trim().is_empty() {
                bail!("persona '{}' has an empty prompt.persona", persona.name);
            }
        }
        // An empty conversation prompt produces a working call with no system
        // prompt at all: the agent answers, but as nobody in particular. That
        // failure is invisible at runtime, so it is caught here.
        if !self.personas.is_empty() && self.prompts.conversation_system.trim().is_empty() {
            bail!("prompts.conversation_system must be set when a persona is configured");
        }
        Ok(())
    }

    /// Refused at startup rather than discovered mid-call: a zero interval would
    /// extract on every turn, and a zero cap would remember nothing while
    /// spending a model call finding out.
    fn validate_memory(&self) -> Result<()> {
        if !self.memory.enabled {
            return Ok(());
        }
        if self.memory.extract_every_turns < 1 {
            bail!("memory.extract_every_turns must be at least 1");
        }
        if self.memory.max_facts < 1 {
            bail!("memory.max_facts must be at least 1");
        }
        if self.memory.max_facts_per_category < 1 {
            bail!("memory.max_facts_per_category must be at least 1");
        }
        if self.memory.max_facts_per_category > self.memory.max_facts {
            bail!(
                "memory.max_facts_per_category ({}) cannot exceed memory.max_facts ({})",
                self.memory.max_facts_per_category,
                self.memory.max_facts
            );
        }
        // Without it the extraction asks for nothing in particular and stores
        // whatever comes back, which is worse than being switched off.
        if self.prompts.memory_extraction.trim().is_empty() {
            bail!("prompts.memory_extraction must be set when memory is enabled");
        }
        Ok(())
    }
}

fn read(path: &Path) -> Result<Settings> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let settings: Settings =
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
    settings.validate()?;
    settings.validate_personas()?;
    settings.validate_memory()?;
    Ok(settings)
}

/// The live settings, replaced whole on every accepted reload.
#[derive(Clone, Debug)]
pub struct SettingsHandle {
    current: Arc<RwLock<Arc<Settings>>>,
}

impl SettingsHandle {
    pub fn get(&self) -> Arc<Settings> {
        self.current
            .read()
            .expect("settings lock is poisoned")
            .clone()
    }

    fn replace(&self, settings: Settings) {
        *self.current.write().expect("settings lock is poisoned") = Arc::new(settings);
    }
}

pub fn config_path() -> PathBuf {
    std::env::var("SONARI_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_PATH))
}

/// Loads the file and starts watching it.
///
/// A missing file is not an error — it means nothing is configured yet, which
/// the caller reports as voice being unavailable. A malformed file is an error:
/// starting with half a configuration is worse than not starting.
pub fn load_and_watch(path: &Path) -> Result<SettingsHandle> {
    let settings = if path.exists() {
        read(path)?
    } else {
        tracing::warn!(path = %path.display(), "no configuration file; using defaults");
        Settings {
            models: None,
            personas: Vec::new(),
            llm: LlmSettings::default(),
            prompts: PromptTemplates::default(),
            endpointing: EndpointingSettings::default(),
            memory: MemorySettings::default(),
        }
    };

    let handle = SettingsHandle {
        current: Arc::new(RwLock::new(Arc::new(settings))),
    };
    spawn_watcher(path.to_path_buf(), handle.clone());
    Ok(handle)
}

/// Watches the file's directory rather than the file itself: editors commonly
/// save by writing a temporary file and renaming it over the original, which
/// destroys a watch registered on the file.
fn spawn_watcher(path: PathBuf, handle: SettingsHandle) {
    let Some(directory) = path.parent().map(Path::to_path_buf) else {
        tracing::warn!(path = %path.display(), "configuration path has no directory; not watching");
        return;
    };
    let directory = if directory.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        directory
    };
    let filename = path.file_name().map(std::ffi::OsStr::to_os_string);

    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::error!(%error, "failed to start configuration watcher; edits need a restart");
                return;
            }
        };
        if let Err(error) = watcher.watch(&directory, RecursiveMode::NonRecursive) {
            tracing::error!(%error, directory = %directory.display(), "failed to watch configuration directory");
            return;
        }

        for event in rx {
            let Ok(event) = event else { continue };
            let touched = event
                .paths
                .iter()
                .any(|changed| changed.file_name().map(std::ffi::OsStr::to_os_string) == filename);
            if !touched {
                continue;
            }
            match read(&path) {
                Ok(settings) => {
                    let previous = handle.get();
                    if model_paths_changed(&previous, &settings) {
                        tracing::warn!(
                            "model paths changed; models load at startup, so this needs a restart"
                        );
                    }
                    handle.replace(settings);
                    tracing::info!(path = %path.display(), "configuration reloaded");
                }
                Err(error) => {
                    tracing::error!(
                        error = %format!("{error:#}"),
                        "configuration is invalid; keeping the previous one"
                    );
                }
            }
        }
    });
}

/// The local model is loaded once at startup, so changing its path cannot take
/// effect without one. Which hosted models to ask for is read per call and needs
/// no restart.
fn model_paths_changed(previous: &Settings, next: &Settings) -> bool {
    match (&previous.models, &next.models) {
        (Some(previous), Some(next)) => previous.vad.model != next.vad.model,
        (None, None) => false,
        _ => true,
    }
}
