create table if not exists call_sessions (
  id bigserial primary key,
  user_id bigint not null references users (id),
  character_id bigint not null references characters (id),
  llm_session_id varchar(128) not null,
  call_type varchar(32) not null,
  voice_mode varchar(32) not null,
  status varchar(32) not null,
  owner_instance varchar(128) not null,
  duration_seconds integer not null default 0,
  started_at timestamptz not null default now(),
  ended_at timestamptz null
);

create index if not exists idx_call_sessions_user_started_at
  on call_sessions (user_id, started_at desc);

create index if not exists idx_call_sessions_character_started_at
  on call_sessions (character_id, started_at desc);

create index if not exists idx_call_sessions_llm_session_id
  on call_sessions (llm_session_id);

create index if not exists idx_call_sessions_owner_status_started_at
  on call_sessions (owner_instance, status, started_at desc);

create table if not exists call_event_outbox (
  id bigserial primary key,
  session_id bigint not null references call_sessions (id),
  round_id varchar(128) null,
  source varchar(64) not null,
  event varchar(128) not null,
  ts_ms bigint not null,
  fields jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create index if not exists idx_call_event_outbox_session_ts
  on call_event_outbox (session_id, ts_ms);

create index if not exists idx_call_event_outbox_created_at
  on call_event_outbox (created_at desc);
