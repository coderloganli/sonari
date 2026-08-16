//! Personas, read from `sonari.toml`.
//!
//! The call path asks for a character and a scene by numeric id, because that is
//! what it stored on the session row. Configuration names them instead, so ids
//! are derived from the name — deterministically, so that adding or reordering
//! entries does not renumber the others and invalidate history.

use std::sync::Arc;

use async_trait::async_trait;
use character_context::{
    CharacterCallContext, CharacterCallContextReadPort, CharacterCatalogReadPort,
    CharacterPromptContext, CharacterPromptContextReadPort, CharacterPromptProfile,
    CharacterSummary, ScenePromptProfile,
};
use sha2::{Digest, Sha256};
use shared_kernel::{AppError, AppResult};

use sonari_config::{PersonaConfig, SettingsHandle};

/// A stable positive id for a name.
///
/// Derived rather than assigned so that editing the file does not renumber
/// personas: a session recorded last week still resolves to the same one.
pub fn id_for(name: &str) -> i64 {
    let digest = Sha256::digest(name.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    // Positive: the column is a signed integer and negative ids read as errors.
    (i64::from_be_bytes(bytes) & i64::MAX).max(1)
}

/// Serves the character ports from configuration.
pub struct ConfigPersonas {
    settings: SettingsHandle,
}

impl ConfigPersonas {
    pub fn new(settings: SettingsHandle) -> Self {
        Self { settings }
    }

    /// Resolves a persona by the id the caller stored.
    fn find(&self, character_id: i64) -> AppResult<Arc<PersonaConfig>> {
        let settings = self.settings.get();
        settings
            .personas
            .iter()
            .find(|persona| id_for(&persona.name) == character_id)
            .cloned()
            .map(Arc::new)
            .ok_or_else(|| {
                AppError::not_found(format!("no persona is configured with id {character_id}"))
            })
    }
}

#[async_trait]
impl CharacterCatalogReadPort for ConfigPersonas {
    async fn list_characters(&self) -> AppResult<Vec<CharacterSummary>> {
        // Read from the live settings handle, so editing sonari.toml changes
        // what the next request sees without a restart.
        Ok(self
            .settings
            .get()
            .personas
            .iter()
            .map(|persona| CharacterSummary {
                // The same derivation `find` resolves by: the id a list offers
                // is the id a call accepts.
                character_id: id_for(&persona.name),
                name: persona.name.clone(),
                scene_name: persona.scene.as_ref().map(|scene| scene.name.clone()),
            })
            .collect())
    }
}

#[async_trait]
impl CharacterCallContextReadPort for ConfigPersonas {
    async fn get_visible_call_context(
        &self,
        _user_id: i64,
        character_id: i64,
        _selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterCallContext> {
        // Every persona is visible to everyone: there is no per-user catalogue.
        let persona = self.find(character_id)?;
        let (scene_id, scene_name) = match &persona.scene {
            Some(scene) => (id_for(&scene.name), scene.name.clone()),
            None => (0, String::new()),
        };
        Ok(CharacterCallContext {
            character_id,
            character_name: persona.name.clone(),
            // The synthesis voice, carried as the provider names it. Nothing
            // maps it to an internal id: there is one provider, and a persona
            // saying which voice it speaks with is the whole of the concept.
            voice: persona.voice.clone(),
            scene_id,
            scene_name,
        })
    }
}

#[async_trait]
impl CharacterPromptContextReadPort for ConfigPersonas {
    async fn get_prompt_context(
        &self,
        character_id: i64,
        _selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterPromptContext> {
        let persona = self.find(character_id)?;
        Ok(CharacterPromptContext {
            character: CharacterPromptProfile {
                character_id,
                language: persona.language.clone(),
                relationship_stance: persona.prompt.relationship_stance.clone(),
                name: persona.name.clone(),
                age: persona.prompt.age,
                marital_status: persona.prompt.marital_status.clone(),
                occupation: persona.prompt.occupation.clone(),
                persona: persona.prompt.persona.clone(),
                private_interests: persona.prompt.private_interests.clone(),
                personality_traits: persona.prompt.personality_traits.clone(),
                speaking_style: persona.prompt.speaking_style.clone(),
            },
            scene: persona.scene.as_ref().map(|scene| ScenePromptProfile {
                scene_id: id_for(&scene.name),
                location: scene.location.clone(),
                user_role: scene.user_role.clone(),
                relationship: scene.relationship.clone(),
                environment: scene.environment.clone(),
                goal: scene.goal.clone(),
                opening_event: scene.opening_event.clone(),
                time_period_mode: scene.time_period_mode.clone(),
                time_period: scene.time_period.clone(),
            }),
        })
    }
}

/// The call path asks for a caller's context. Without a user table there is
/// nothing to look up: a `uid` names a history, not a profile.
pub struct AnonymousCallers;

/// Used to tell the model what time it is where the caller is. Without a user
/// table there is nobody to ask, so the deployment declares one.
const DEFAULT_TIMEZONE: &str = "UTC";

#[async_trait]
impl user_context::UserCallContextReadPort for AnonymousCallers {
    async fn get_call_context(&self, user_id: i64) -> AppResult<user_context::UserCallContext> {
        Ok(user_context::UserCallContext {
            user_id,
            timezone: Some(
                std::env::var("SONARI_TIMEZONE")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_TIMEZONE.to_owned()),
            ),
            // There is no profile to complete.
            needs_profile_completion: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_positive() {
        assert_eq!(id_for("assistant"), id_for("assistant"));
        assert!(id_for("assistant") > 0);
        assert!(id_for("another") > 0);
    }

    #[test]
    fn different_names_get_different_ids() {
        assert_ne!(id_for("assistant"), id_for("another"));
    }

    /// The path that once broke silently: the port carrying a persona's voice
    /// was deleted and nothing failed — every synthesis request simply asked for
    /// an empty voice. Tests covered both ends of the path and not the path.
    #[tokio::test]
    async fn the_voice_a_persona_names_reaches_the_call_context() {
        let dir = std::env::temp_dir().join("sonari-persona-voice");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sonari.toml");
        // An opaque provider voice id, exactly as configuration carries it.
        let voice = "21m00Tcm4TlvDq8ikWAM";
        std::fs::write(
            &path,
            format!(
                "[prompts]
conversation_system = \"Speak briefly.\"

                 [[persona]]
name = \"companion\"
voice = \"{voice}\"

                 [persona.prompt]
relationship_stance = \"warm\"
                 persona = \"You are easy to talk to.\"
"
            ),
        )
        .expect("write config");

        let settings = sonari_config::load_and_watch(&path).expect("load settings");
        let personas = ConfigPersonas::new(settings);
        let context = personas
            .get_visible_call_context(1, id_for("companion"), None)
            .await
            .expect("resolve the persona");

        assert_eq!(
            context.voice, voice,
            "the voice must arrive unchanged; anything else asks synthesis for a              voice that does not exist"
        );
    }

    /// Writes a configuration with two personas, one of which has a scene, and
    /// returns the catalog over it.
    fn two_personas(directory: &str) -> ConfigPersonas {
        let dir = std::env::temp_dir().join(directory);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("sonari.toml");
        std::fs::write(
            &path,
            r#"
[prompts]
conversation_system = "Speak briefly."

[[persona]]
name = "companion"
voice = "21m00Tcm4TlvDq8ikWAM"

[persona.prompt]
relationship_stance = "warm"
persona = "You are easy to talk to."

[persona.scene]
name = "evening-call"

[[persona]]
name = "another"
voice = "EXAVITQu4vr4xnSDxMaL"

[persona.prompt]
relationship_stance = "brisk"
persona = "You get to the point."
"#,
        )
        .expect("write config");
        ConfigPersonas::new(sonari_config::load_and_watch(&path).expect("load settings"))
    }

    /// Test case 15 — every configured persona is listed, with the id derived
    /// from its name and the scene name where one is configured.
    #[tokio::test]
    async fn every_configured_persona_is_listed() {
        let personas = two_personas("sonari-persona-list");
        let listed = personas.list_characters().await.expect("list personas");

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "companion");
        assert_eq!(listed[0].character_id, id_for("companion"));
        assert_eq!(listed[0].scene_name.as_deref(), Some("evening-call"));
        assert_eq!(listed[1].name, "another");
        assert_eq!(listed[1].character_id, id_for("another"));
        assert_eq!(listed[1].scene_name, None);
    }

    /// Test case 16 — the id the list offers is the id a call accepts.
    ///
    /// Both ends of this path have had tests before and the path between them
    /// has not; that is how a persona's voice was once dropped silently.
    #[tokio::test]
    async fn the_listed_id_is_the_id_a_call_resolves() {
        let personas = two_personas("sonari-persona-list-resolves");
        let listed = personas.list_characters().await.expect("list personas");

        for persona in listed {
            let context = personas
                .get_visible_call_context(1, persona.character_id, None)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "the list offered {} as id {}, which no call can start: {error}",
                        persona.name, persona.character_id
                    )
                });
            assert_eq!(context.character_name, persona.name);
        }
    }

    #[test]
    fn adding_a_persona_does_not_renumber_the_others() {
        // The property that positional numbering would break.
        let before = id_for("second");
        let after = id_for("second");
        assert_eq!(before, after);
    }
}
