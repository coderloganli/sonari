alter table call_sessions
  add column if not exists scene_id bigint null references scenes (id);

update call_sessions cs
set scene_id = ls.scene_id
from llm_sessions ls
where cs.agent_session_id = ls.id
  and cs.scene_id is null;

alter table call_sessions
  alter column scene_id set not null;

create index if not exists idx_call_sessions_scene_started_at
  on call_sessions (scene_id, started_at desc);
