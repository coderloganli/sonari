create table if not exists call_events (
  id bigserial primary key,
  session_id bigint not null references call_sessions(id) on delete cascade,
  round_id varchar(128) null,
  source varchar(64) not null,
  event varchar(128) not null,
  ts_ms bigint not null,
  fields jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create index if not exists idx_call_events_session_ts
  on call_events (session_id, ts_ms, id);

create index if not exists idx_call_events_created_at
  on call_events (created_at desc);
