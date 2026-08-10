-- Removes what the extraction left behind.
--
-- These tables served features that no longer exist: user accounts and the
-- phone-number login that filled them, the character catalogue that personas
-- replaced, and the SDK's call mapping. Their columns also carried the
-- adult-content model this project does not have — an orientation on every
-- character and a set of preferences on every user.
--
-- Nothing reads them: identity is derived from a uid, and a persona is a section
-- of sonari.toml.

drop table if exists sdk_call_session_mappings cascade;

drop table if exists user_notifications cascade;
drop table if exists notifications cascade;
drop table if exists user_profiles cascade;
drop table if exists users cascade;

drop table if exists voiceprint_audio_assets cascade;
drop table if exists voiceprints cascade;
drop table if exists scenes cascade;
drop table if exists characters cascade;

-- Supplier credentials for the cloud recognition and synthesis that ADR-0014
-- replaced. Keys now come from the environment and are never stored.
drop table if exists voice_suppliers cascade;
drop table if exists llm_provider_configs cascade;
drop table if exists llm_prompt_templates cascade;
