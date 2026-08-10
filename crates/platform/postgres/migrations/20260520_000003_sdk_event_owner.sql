create table if not exists event_sdk_batches (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  runtime_snapshot_id bigint not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  owner_batch_id text not null unique,
  batch_status text not null,
  accepted_count bigint not null,
  rejected_count bigint not null,
  request_id text not null,
  trace_id text,
  user_agent_hash text,
  source_ip_hash text,
  received_at timestamptz not null,
  created_at timestamptz not null default now(),
  constraint fk_event_sdk_batches_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_event_sdk_batches_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_event_sdk_batches_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_event_sdk_batches_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_event_sdk_batches_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint fk_event_sdk_batches_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id, config_version_id, permission_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id),
  constraint chk_event_sdk_batches_status check (batch_status in ('accepted', 'partially_accepted', 'rejected')),
  constraint chk_event_sdk_batches_counts_non_negative check (accepted_count >= 0 and rejected_count >= 0),
  constraint chk_event_sdk_batches_non_empty check (accepted_count + rejected_count > 0),
  constraint chk_event_sdk_batches_owner_batch_id_not_empty check (length(trim(owner_batch_id)) > 0),
  constraint chk_event_sdk_batches_request_id_not_empty check (length(trim(request_id)) > 0)
);

create index if not exists idx_event_sdk_batches_app_created
  on event_sdk_batches (partner_id, sdk_app_id, created_at desc);

create index if not exists idx_event_sdk_batches_user_created
  on event_sdk_batches (partner_id, sdk_app_id, sdk_user_id, created_at desc);

create index if not exists idx_event_sdk_batches_request_id
  on event_sdk_batches (request_id);

create table if not exists event_sdk_items (
  id bigserial primary key,
  batch_id bigint not null references event_sdk_batches(id) on delete cascade,
  item_index integer not null,
  event_name text not null,
  event_time timestamptz not null,
  schema_version integer not null,
  payload jsonb not null,
  debug_context_ref text,
  created_at timestamptz not null default now(),
  constraint uq_event_sdk_items_batch_index unique (batch_id, item_index),
  constraint chk_event_sdk_items_index_non_negative check (item_index >= 0),
  constraint chk_event_sdk_items_name_not_empty check (length(trim(event_name)) > 0),
  constraint chk_event_sdk_items_schema_positive check (schema_version > 0),
  constraint chk_event_sdk_items_payload_object check (jsonb_typeof(payload) = 'object')
);

create index if not exists idx_event_sdk_items_event_time
  on event_sdk_items (event_name, event_time desc);

create table if not exists event_sdk_rejected_items (
  id bigserial primary key,
  batch_id bigint not null references event_sdk_batches(id) on delete cascade,
  item_index integer not null,
  event_name text not null,
  reason text not null,
  created_at timestamptz not null default now(),
  constraint uq_event_sdk_rejected_items_batch_index unique (batch_id, item_index),
  constraint chk_event_sdk_rejected_items_index_non_negative check (item_index >= 0),
  constraint chk_event_sdk_rejected_items_reason_not_empty check (length(trim(reason)) > 0)
);
