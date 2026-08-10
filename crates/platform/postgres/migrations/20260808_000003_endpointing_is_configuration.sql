-- Endpointing parameters moved into sonari.toml. They are tuned by ear against
-- real calls, and a change should be a reviewable diff rather than a row nobody
-- can attribute.
drop table if exists speech_runtime_configs cascade;

-- The last of the SDK columns on the session tables. The caller identity that
-- filled them no longer has an SDK variant.
alter table llm_sessions drop column if exists sdk_partner_id;
alter table llm_sessions drop column if exists sdk_app_id;
alter table llm_sessions drop column if exists sdk_user_id;
alter table llm_sessions drop column if exists sdk_session_id;
alter table llm_sessions drop column if exists runtime_snapshot_id;
alter table llm_sessions drop column if exists external_user_id_hash;
