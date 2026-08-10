-- A persona names the voice it speaks with, as the provider names it. The
-- numeric handle pointed at a `voiceprints` table that no longer exists, and
-- carrying it meant the persona's actual voice had no way to reach synthesis.
--
-- Written defensively: these tables were inherited and the column is not on all
-- of them.
do $$
begin
  if exists (select 1 from information_schema.columns
             where table_name = 'call_sessions' and column_name = 'voiceprint_id') then
    alter table call_sessions rename column voiceprint_id to voice;
  end if;
  if exists (select 1 from information_schema.columns
             where table_name = 'call_sessions' and column_name = 'voice') then
    alter table call_sessions alter column voice type text using voice::text;
  end if;

  if exists (select 1 from information_schema.columns
             where table_name = 'llm_sessions' and column_name = 'voiceprint_id') then
    alter table llm_sessions rename column voiceprint_id to voice;
  end if;
  if exists (select 1 from information_schema.columns
             where table_name = 'llm_sessions' and column_name = 'voice') then
    alter table llm_sessions alter column voice type text using voice::text;
  end if;
end $$;

-- The SDK caller identity is gone; these columns recorded it.
alter table call_sessions drop column if exists sdk_partner_id;
alter table call_sessions drop column if exists sdk_app_id;
alter table call_sessions drop column if exists sdk_user_id;
alter table call_sessions drop column if exists sdk_session_id;
alter table call_sessions drop column if exists runtime_snapshot_id;
alter table call_sessions drop column if exists external_user_id_hash;
