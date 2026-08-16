use async_trait::async_trait;
use shared_kernel::AppResult;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterCallContext {
    pub character_id: i64,
    pub character_name: String,
    /// The synthesis voice this persona speaks with.
    pub voice: String,
    pub scene_id: i64,
    pub scene_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterPromptProfile {
    pub character_id: i64,
    pub language: String,
    pub relationship_stance: String,
    pub name: String,
    pub age: i32,
    pub marital_status: String,
    pub occupation: String,
    pub persona: String,
    pub private_interests: Vec<String>,
    pub personality_traits: String,
    pub speaking_style: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenePromptProfile {
    pub scene_id: i64,
    pub location: String,
    pub user_role: String,
    pub relationship: String,
    pub environment: String,
    pub goal: String,
    pub opening_event: String,
    pub time_period_mode: String,
    pub time_period: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterPromptContext {
    pub character: CharacterPromptProfile,
    pub scene: Option<ScenePromptProfile>,
}

/// A persona as a client needs to see it: enough to offer a choice and start a
/// call, and nothing else. The prompts and the synthesis voice are the
/// operator's material (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSummary {
    pub character_id: i64,
    pub name: String,
    pub scene_name: Option<String>,
}

#[async_trait]
pub trait CharacterCallContextReadPort: Send + Sync {
    async fn get_visible_call_context(
        &self,
        user_id: i64,
        character_id: i64,
        selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterCallContext>;
}

#[async_trait]
pub trait CharacterPromptContextReadPort: Send + Sync {
    async fn get_prompt_context(
        &self,
        character_id: i64,
        selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterPromptContext>;
}

/// Reading the catalogue of personas a caller may choose from.
#[async_trait]
pub trait CharacterCatalogReadPort: Send + Sync {
    async fn list_characters(&self) -> AppResult<Vec<CharacterSummary>>;
}

#[async_trait]
impl<T> CharacterCatalogReadPort for Arc<T>
where
    T: CharacterCatalogReadPort + Send + Sync + ?Sized,
{
    async fn list_characters(&self) -> AppResult<Vec<CharacterSummary>> {
        (**self).list_characters().await
    }
}

#[async_trait]
impl<T> CharacterCallContextReadPort for Arc<T>
where
    T: CharacterCallContextReadPort + Send + Sync + ?Sized,
{
    async fn get_visible_call_context(
        &self,
        user_id: i64,
        character_id: i64,
        selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterCallContext> {
        (**self)
            .get_visible_call_context(user_id, character_id, selected_scene_id)
            .await
    }
}

#[async_trait]
impl<T> CharacterPromptContextReadPort for Arc<T>
where
    T: CharacterPromptContextReadPort + Send + Sync + ?Sized,
{
    async fn get_prompt_context(
        &self,
        character_id: i64,
        selected_scene_id: Option<i64>,
    ) -> AppResult<CharacterPromptContext> {
        (**self)
            .get_prompt_context(character_id, selected_scene_id)
            .await
    }
}
