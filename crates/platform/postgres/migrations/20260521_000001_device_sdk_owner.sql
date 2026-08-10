create table if not exists device_sdk_events (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  runtime_snapshot_id bigint not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  owner_device_event_id text not null unique,
  device_id_hash text,
  device_type text not null,
  firmware_version text,
  protocol_version text,
  event_kind text not null,
  event_payload jsonb not null,
  raw_protocol_payload jsonb,
  request_id text not null,
  trace_id text,
  user_agent_hash text,
  source_ip_hash text,
  event_time timestamptz not null,
  received_at timestamptz not null,
  created_at timestamptz not null default now(),
  constraint fk_device_sdk_events_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_device_sdk_events_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_device_sdk_events_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_device_sdk_events_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_device_sdk_events_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint fk_device_sdk_events_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id, config_version_id, permission_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id),
  constraint chk_device_sdk_events_type_not_empty check (length(trim(device_type)) > 0),
  constraint chk_device_sdk_events_request_id_not_empty check (length(trim(request_id)) > 0)
);

create index if not exists idx_device_sdk_events_app_created
  on device_sdk_events (partner_id, sdk_app_id, created_at desc);

create index if not exists idx_device_sdk_events_device_created
  on device_sdk_events (partner_id, sdk_app_id, device_type, device_id_hash, created_at desc);

create table if not exists device_sdk_support_policies (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  device_type text not null,
  protocol_version text,
  firmware_version text,
  supported boolean not null,
  reason text,
  capabilities jsonb not null default '[]'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_device_sdk_support_policies_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint chk_device_sdk_support_policies_type_not_empty check (length(trim(device_type)) > 0),
  constraint chk_device_sdk_support_policies_capabilities_array check (jsonb_typeof(capabilities) = 'array')
);

create unique index if not exists uq_device_sdk_support_policies_exact
  on device_sdk_support_policies (
    partner_id,
    sdk_app_id,
    device_type,
    coalesce(protocol_version, ''),
    coalesce(firmware_version, '')
  );

create table if not exists device_sdk_output_commands (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  runtime_snapshot_id bigint not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  producer_ref text not null,
  producer_request_id text not null,
  command_id text not null,
  command_type text not null,
  parameters jsonb not null,
  target_device_id_hash text,
  target_device_type text,
  target_protocol_version text,
  command_version bigint not null,
  state text not null,
  available_at timestamptz not null,
  expires_at timestamptz not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_device_sdk_output_commands_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_device_sdk_output_commands_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_device_sdk_output_commands_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_device_sdk_output_commands_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_device_sdk_output_commands_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint fk_device_sdk_output_commands_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id, config_version_id, permission_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id),
  constraint chk_device_sdk_output_commands_state check (state in ('pending', 'delivered', 'sent', 'completed', 'failed', 'unsupported', 'timeout', 'expired')),
  constraint chk_device_sdk_output_commands_producer_ref_not_empty check (length(trim(producer_ref)) > 0),
  constraint chk_device_sdk_output_commands_producer_request_id_not_empty check (length(trim(producer_request_id)) > 0),
  constraint chk_device_sdk_output_commands_id_not_empty check (length(trim(command_id)) > 0),
  constraint chk_device_sdk_output_commands_type_not_empty check (length(trim(command_type)) > 0),
  constraint chk_device_sdk_output_commands_parameters_array check (jsonb_typeof(parameters) = 'array'),
  constraint chk_device_sdk_output_commands_version_positive check (command_version > 0),
  constraint chk_device_sdk_output_commands_expiry check (expires_at > available_at)
);

create index if not exists idx_device_sdk_output_commands_lease
  on device_sdk_output_commands (partner_id, sdk_app_id, state, available_at, expires_at);

create unique index if not exists uq_device_sdk_output_commands_partner_command
  on device_sdk_output_commands (partner_id, command_id);

create table if not exists device_sdk_output_delivery_attempts (
  id bigserial primary key,
  command_id text not null,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  runtime_snapshot_id bigint not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  delivery_attempt_id text not null,
  lease_idempotency_key text not null,
  status text not null,
  leased_at timestamptz not null,
  lease_expires_at timestamptz not null,
  result_time timestamptz,
  result_reason text,
  raw_protocol_payload jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_device_sdk_output_attempts_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_device_sdk_output_attempts_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_device_sdk_output_attempts_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_device_sdk_output_attempts_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_device_sdk_output_attempts_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint fk_device_sdk_output_attempts_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id, config_version_id, permission_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id),
  constraint fk_device_sdk_output_attempts_command foreign key (partner_id, command_id) references device_sdk_output_commands(partner_id, command_id),
  constraint chk_device_sdk_output_attempts_status check (status in ('delivered', 'sent', 'completed', 'failed', 'unsupported', 'timeout')),
  constraint chk_device_sdk_output_attempts_id_not_empty check (length(trim(delivery_attempt_id)) > 0),
  constraint chk_device_sdk_output_attempts_key_not_empty check (length(trim(lease_idempotency_key)) > 0),
  constraint chk_device_sdk_output_attempts_lease_expiry check (lease_expires_at > leased_at)
);

create unique index if not exists uq_device_sdk_output_attempts_idempotency
  on device_sdk_output_delivery_attempts (partner_id, sdk_app_id, sdk_user_id, sdk_session_id, runtime_snapshot_id, lease_idempotency_key, command_id);

create unique index if not exists uq_device_sdk_output_attempts_delivery_command
  on device_sdk_output_delivery_attempts (partner_id, delivery_attempt_id, command_id);

create index if not exists idx_device_sdk_output_attempts_command_created
  on device_sdk_output_delivery_attempts (partner_id, command_id, created_at desc);

create index if not exists idx_device_sdk_output_attempts_lease_expiry
  on device_sdk_output_delivery_attempts (status, lease_expires_at);
