alter table call_sessions
  add column if not exists agent_session_id varchar(128);

update call_sessions
set agent_session_id = llm_session_id
where agent_session_id is null;

alter table call_sessions
  alter column agent_session_id set not null;

drop index if exists idx_call_sessions_llm_session_id;

create index if not exists idx_call_sessions_agent_session_id
  on call_sessions (agent_session_id);

alter table call_sessions
  drop column if exists llm_session_id;

create table if not exists speech_runtime_configs (
  config_key varchar(64) primary key,
  min_utterance_ms integer not null,
  silence_flush_ms integer not null,
  voice_activity_threshold integer not null,
  updated_at timestamptz not null default now()
);

insert into speech_runtime_configs (
  config_key,
  min_utterance_ms,
  silence_flush_ms,
  voice_activity_threshold,
  updated_at
)
values ('threshold', 250, 400, 384, now())
on conflict (config_key) do nothing;
