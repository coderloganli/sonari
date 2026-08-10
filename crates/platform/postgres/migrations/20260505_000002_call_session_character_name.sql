alter table call_sessions add column if not exists character_name text;

do $$
begin
  if exists (
    select 1
    from call_sessions
    where coalesce(nullif(character_name, ''), nullif(character_name_zh, ''), nullif(character_name_en, '')) is null
  ) then
    raise exception 'call_sessions.character_name requires an existing durable character snapshot before migration';
  end if;
end $$;

update call_sessions cs
set character_name = coalesce(nullif(character_name, ''), nullif(character_name_zh, ''), character_name_en)
where character_name is null or character_name = '';

alter table call_sessions alter column character_name set not null;

alter table call_sessions
  drop column if exists character_name_zh,
  drop column if exists character_name_en;
