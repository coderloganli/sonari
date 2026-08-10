alter table call_sessions
  add column if not exists character_name_zh text;

alter table call_sessions
  add column if not exists character_name_en text;

alter table call_sessions
  add column if not exists scene_name text;

update call_sessions cs
set
  character_name_zh = c.name_zh,
  character_name_en = c.name_en,
  scene_name = s.name
from characters c, scenes s
where cs.character_id = c.id
  and cs.scene_id = s.id
  and (
    cs.character_name_zh is null
    or cs.character_name_en is null
    or cs.scene_name is null
  );

alter table call_sessions
  alter column character_name_zh set not null;

alter table call_sessions
  alter column character_name_en set not null;

alter table call_sessions
  alter column scene_name set not null;
