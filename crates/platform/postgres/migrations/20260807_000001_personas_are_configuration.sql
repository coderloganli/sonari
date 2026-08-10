-- Personas moved from the database into `sonari.toml`, so the `characters` table
-- no longer defines what a persona is and nothing can be keyed to it.
--
-- The columns stay: they still record which persona a session used, and the id
-- is derived from the persona's name so it remains stable across edits. What
-- goes is the constraint that the row must exist here.

alter table llm_sessions drop constraint if exists llm_sessions_character_id_fkey;
alter table llm_messages drop constraint if exists llm_messages_character_id_fkey;
alter table call_sessions drop constraint if exists call_sessions_character_id_fkey;
alter table call_sessions drop constraint if exists call_sessions_scene_id_fkey;
alter table llm_sessions drop constraint if exists llm_sessions_scene_id_fkey;
