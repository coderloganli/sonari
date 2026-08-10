alter table call_sessions
  add column if not exists voiceprint_id bigint;

update call_sessions cs
set voiceprint_id = c.voiceprint_id
from characters c
where cs.character_id = c.id
  and cs.voiceprint_id is null;

alter table call_sessions
  alter column voiceprint_id set not null;

create index if not exists idx_call_sessions_voiceprint_id
  on call_sessions (voiceprint_id);
