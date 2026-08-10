do $$
begin
  if exists (
    select 1
    from information_schema.tables
    where table_schema = 'public'
      and table_name = 'app_error_reports'
  ) then
    alter table app_error_reports rename to app_error_occurrences;
  end if;
end
$$;

create table if not exists app_error_occurrences (
  id bigserial primary key,
  group_id bigint,
  request_id text not null default '',
  trace_id text,
  fingerprint text,
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

alter table app_error_occurrences
  add column if not exists group_id bigint;
alter table app_error_occurrences
  add column if not exists trace_id text;
alter table app_error_occurrences
  add column if not exists fingerprint text;
alter table app_error_occurrences
  alter column request_id set default '';

create table if not exists app_error_groups (
  id bigserial primary key,
  fingerprint text not null unique,
  category text not null,
  latest_level text not null,
  sample_message text not null,
  sample_stack_trace text not null default '',
  latest_request_id text not null default '',
  latest_trace_id text,
  latest_session_id text,
  latest_device_name text,
  latest_app_version text not null,
  latest_os_version text not null,
  first_seen_at timestamptz not null,
  last_seen_at timestamptz not null,
  occurrence_count bigint not null
);

update app_error_occurrences
set fingerprint = md5(
  category || '|' || message || '|' ||
  split_part(coalesce(stack_trace, ''), E'\n', 1)
)
where fingerprint is null or fingerprint = '';

with latest as (
  select distinct on (fingerprint)
    fingerprint,
    category,
    level,
    message,
    stack_trace,
    request_id,
    trace_id,
    session_id,
    device_name,
    app_version,
    os_version,
    created_at
  from app_error_occurrences
  where fingerprint is not null and fingerprint <> ''
  order by fingerprint, created_at desc, id desc
),
counts as (
  select
    fingerprint,
    min(created_at) as first_seen_at,
    max(created_at) as last_seen_at,
    count(*)::bigint as occurrence_count
  from app_error_occurrences
  where fingerprint is not null and fingerprint <> ''
  group by fingerprint
)
insert into app_error_groups (
  fingerprint,
  category,
  latest_level,
  sample_message,
  sample_stack_trace,
  latest_request_id,
  latest_trace_id,
  latest_session_id,
  latest_device_name,
  latest_app_version,
  latest_os_version,
  first_seen_at,
  last_seen_at,
  occurrence_count
)
select
  latest.fingerprint,
  latest.category,
  latest.level,
  latest.message,
  latest.stack_trace,
  latest.request_id,
  latest.trace_id,
  latest.session_id,
  latest.device_name,
  latest.app_version,
  latest.os_version,
  counts.first_seen_at,
  counts.last_seen_at,
  counts.occurrence_count
from latest
join counts using (fingerprint)
on conflict (fingerprint) do update set
  category = excluded.category,
  latest_level = excluded.latest_level,
  sample_message = excluded.sample_message,
  sample_stack_trace = excluded.sample_stack_trace,
  latest_request_id = excluded.latest_request_id,
  latest_trace_id = excluded.latest_trace_id,
  latest_session_id = excluded.latest_session_id,
  latest_device_name = excluded.latest_device_name,
  latest_app_version = excluded.latest_app_version,
  latest_os_version = excluded.latest_os_version,
  first_seen_at = excluded.first_seen_at,
  last_seen_at = excluded.last_seen_at,
  occurrence_count = excluded.occurrence_count;

update app_error_occurrences occurrences
set group_id = groups.id
from app_error_groups groups
where occurrences.fingerprint = groups.fingerprint
  and occurrences.group_id is null;

do $$
begin
  if not exists (
    select 1
    from information_schema.table_constraints
    where table_schema = 'public'
      and table_name = 'app_error_occurrences'
      and constraint_name = 'fk_app_error_occurrences_group_id'
  ) then
    alter table app_error_occurrences
      add constraint fk_app_error_occurrences_group_id
      foreign key (group_id) references app_error_groups(id);
  end if;
end
$$;

create index if not exists idx_app_error_occurrences_created_at
  on app_error_occurrences (created_at desc);
create index if not exists idx_app_error_occurrences_group_id_created_at
  on app_error_occurrences (group_id, created_at desc);
create index if not exists idx_app_error_occurrences_request_id
  on app_error_occurrences (request_id);
create index if not exists idx_app_error_occurrences_session_id
  on app_error_occurrences (session_id)
  where session_id is not null;
create index if not exists idx_app_error_occurrences_fingerprint
  on app_error_occurrences (fingerprint);

create index if not exists idx_app_error_groups_last_seen_at
  on app_error_groups (last_seen_at desc);
create index if not exists idx_app_error_groups_category_last_seen_at
  on app_error_groups (category, last_seen_at desc);
create index if not exists idx_app_error_groups_latest_level_last_seen_at
  on app_error_groups (latest_level, last_seen_at desc);
