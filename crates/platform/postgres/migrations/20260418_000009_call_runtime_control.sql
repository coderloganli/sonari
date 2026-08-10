alter table call_sessions
  add column if not exists runtime_owner_id varchar(128) null,
  add column if not exists runtime_status varchar(32) not null default 'none',
  add column if not exists runtime_failure_reason text null;

update call_sessions
set runtime_owner_id = owner_instance
where runtime_owner_id is null;

create index if not exists idx_call_sessions_runtime_owner_status_started_at
  on call_sessions (runtime_owner_id, runtime_status, started_at desc);
