create table if not exists app_error_reports (
  id bigserial primary key,
  level text not null,
  category text not null,
  message text not null,
  stack_trace text not null default '',
  session_id text,
  device_name text,
  app_version text not null,
  os_version text not null,
  fields jsonb not null default '{}'::jsonb,
  client_ts_ms bigint not null,
  created_at timestamptz not null default now()
);

create index if not exists idx_app_error_reports_created_at
  on app_error_reports (created_at desc);

create index if not exists idx_app_error_reports_category_created_at
  on app_error_reports (category, created_at desc);

create index if not exists idx_app_error_reports_session_id
  on app_error_reports (session_id)
  where session_id is not null;

create table if not exists client_debug_events (
  id bigserial primary key,
  name text not null,
  level text not null,
  session_id text,
  character_id text,
  device_name text,
  fields jsonb not null default '{}'::jsonb,
  merged_count integer not null default 1,
  client_ts_ms bigint not null,
  created_at timestamptz not null default now()
);

create index if not exists idx_client_debug_events_created_at
  on client_debug_events (created_at desc);

create index if not exists idx_client_debug_events_session_id_created_at
  on client_debug_events (session_id, created_at desc)
  where session_id is not null;
