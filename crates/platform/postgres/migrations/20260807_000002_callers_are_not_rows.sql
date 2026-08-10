-- The same applies to callers. A `uid` names a history; the identity is derived
-- from it rather than allocated in a table, so there is no row to point at.
alter table call_sessions drop constraint if exists call_sessions_user_id_fkey;
alter table llm_sessions drop constraint if exists llm_sessions_user_id_fkey;
