create table if not exists sdk_partners (
  id bigserial primary key,
  display_name text not null,
  environment varchar(32) not null,
  status varchar(32) not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint chk_sdk_partners_environment check (environment in ('development', 'staging', 'production')),
  constraint chk_sdk_partners_status check (status in ('pending', 'active', 'suspended', 'disabled'))
);

create index if not exists idx_sdk_partners_status on sdk_partners (status);

create table if not exists sdk_apps (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  environment varchar(32) not null,
  status varchar(32) not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint uq_sdk_apps_partner_id_id unique (partner_id, id),
  constraint chk_sdk_apps_environment check (environment in ('development', 'staging', 'production')),
  constraint chk_sdk_apps_status check (status in ('active', 'suspended', 'disabled'))
);

create index if not exists idx_sdk_apps_partner_status on sdk_apps (partner_id, status);

create table if not exists sdk_credentials (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  client_identifier text not null unique,
  secret_key_id text not null,
  secret_ciphertext text,
  secret_material_ref text,
  secret_hash text not null,
  secret_prefix text not null,
  secret_version int not null,
  verifier_strategy varchar(32) not null,
  status varchar(32) not null,
  active_from timestamptz not null,
  expires_at timestamptz,
  last_used_at timestamptz,
  rotated_from_id bigint references sdk_credentials(id),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_sdk_credentials_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint uq_sdk_credentials_partner_id_id unique (partner_id, id),
  constraint uq_sdk_credentials_partner_app_id unique (partner_id, sdk_app_id, id),
  constraint chk_sdk_credentials_status check (status in ('active', 'disabled', 'rotating', 'revoked')),
  constraint chk_sdk_credentials_verifier_strategy check (verifier_strategy in ('hmac_sha256', 'external_reference')),
  constraint chk_sdk_credentials_secret_version check (secret_version > 0),
  constraint chk_sdk_credentials_secret_material check (
    (secret_ciphertext is not null and secret_material_ref is null)
    or (secret_ciphertext is null and secret_material_ref is not null)
  )
);

create index if not exists idx_sdk_credentials_partner_app on sdk_credentials (partner_id, sdk_app_id);
create index if not exists idx_sdk_credentials_status_expires on sdk_credentials (status, expires_at);
create index if not exists idx_sdk_credentials_app_status on sdk_credentials (sdk_app_id, status);

create table if not exists sdk_credential_secret_reveals (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  credential_id bigint not null,
  secret_version int not null,
  status varchar(32) not null,
  created_by text not null,
  created_reason text not null,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null,
  consumed_by text,
  consumed_at timestamptz,
  revoked_at timestamptz,
  constraint fk_sdk_credential_secret_reveals_credential foreign key (partner_id, sdk_app_id, credential_id) references sdk_credentials(partner_id, sdk_app_id, id),
  constraint uq_sdk_credential_secret_reveals_version unique (credential_id, secret_version),
  constraint chk_sdk_credential_secret_reveals_status check (status in ('pending', 'consumed', 'expired', 'revoked')),
  constraint chk_sdk_credential_secret_reveals_secret_version check (secret_version > 0)
);

create index if not exists idx_sdk_credential_secret_reveals_credential_status_expires
  on sdk_credential_secret_reveals (credential_id, status, expires_at);
create index if not exists idx_sdk_credential_secret_reveals_partner_app_created
  on sdk_credential_secret_reveals (partner_id, sdk_app_id, created_at desc);

create table if not exists sdk_credential_nonces (
  credential_id bigint not null references sdk_credentials(id),
  nonce_hash text not null,
  seen_at timestamptz not null default now(),
  expires_at timestamptz not null,
  primary key (credential_id, nonce_hash)
);

create index if not exists idx_sdk_credential_nonces_expires on sdk_credential_nonces (expires_at);

create table if not exists sdk_signing_keys (
  id bigserial primary key,
  kid text not null unique,
  algorithm text not null,
  public_material text not null,
  private_material_ref text not null,
  status varchar(32) not null,
  active_from timestamptz not null,
  active_until timestamptz,
  verify_until timestamptz,
  revoked_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint chk_sdk_signing_keys_status check (status in ('pending', 'active', 'verifying_only', 'retired', 'revoked'))
);

create index if not exists idx_sdk_signing_keys_status_active_verify
  on sdk_signing_keys (status, active_from, verify_until);
create unique index if not exists idx_sdk_signing_keys_single_active
  on sdk_signing_keys ((true))
  where status = 'active';

create table if not exists sdk_credential_abuse_counters (
  id bigserial primary key,
  partner_id bigint references sdk_partners(id),
  sdk_app_id bigint,
  credential_id bigint,
  credential_lookup_hash text not null,
  resolution_scope varchar(64) not null,
  bucket_start timestamptz not null,
  bucket_seconds bigint not null,
  signal_type varchar(64) not null,
  count bigint not null default 0,
  last_seen_at timestamptz not null,
  constraint fk_sdk_credential_abuse_counters_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_credential_abuse_counters_credential_partner foreign key (partner_id, credential_id) references sdk_credentials(partner_id, id),
  constraint fk_sdk_credential_abuse_counters_credential_app foreign key (partner_id, sdk_app_id, credential_id) references sdk_credentials(partner_id, sdk_app_id, id),
  constraint chk_sdk_credential_abuse_counters_scope check (resolution_scope in ('credential', 'partner_app_unknown_credential', 'global_unknown')),
  constraint chk_sdk_credential_abuse_counters_signal check (signal_type in (
    'invalid_signature',
    'timestamp_window_failure',
    'nonce_replay',
    'unknown_credential',
    'disabled_credential',
    'revoked_credential',
    'rate_limit_exceeded',
    'cooldown_decision',
    'suspension_decision'
  )),
  constraint chk_sdk_credential_abuse_counters_scope_refs check (
    (resolution_scope = 'credential' and credential_id is not null and partner_id is not null and sdk_app_id is not null)
    or (resolution_scope = 'partner_app_unknown_credential' and credential_id is null and partner_id is not null and sdk_app_id is not null)
    or (resolution_scope = 'global_unknown' and credential_id is null and partner_id is null and sdk_app_id is null)
  )
);

create unique index if not exists uq_sdk_credential_abuse_counters_credential
  on sdk_credential_abuse_counters (credential_id, signal_type, bucket_start, bucket_seconds)
  where credential_id is not null;
create unique index if not exists uq_sdk_credential_abuse_counters_partner_unknown
  on sdk_credential_abuse_counters (partner_id, sdk_app_id, credential_lookup_hash, signal_type, bucket_start, bucket_seconds)
  where credential_id is null and partner_id is not null and sdk_app_id is not null;
create unique index if not exists uq_sdk_credential_abuse_counters_global_unknown
  on sdk_credential_abuse_counters (credential_lookup_hash, signal_type, bucket_start, bucket_seconds)
  where credential_id is null and partner_id is null;
create index if not exists idx_sdk_credential_abuse_counters_partner_signal_bucket
  on sdk_credential_abuse_counters (partner_id, signal_type, bucket_start);

create table if not exists sdk_credential_abuse_enforcements (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  credential_id bigint,
  enforcement_type varchar(32) not null,
  status varchar(32) not null,
  source varchar(32) not null,
  reason text not null,
  effective_from timestamptz not null,
  effective_until timestamptz,
  created_by text not null,
  cleared_by text,
  cleared_at timestamptz,
  created_at timestamptz not null default now(),
  constraint fk_sdk_credential_abuse_enforcements_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_credential_abuse_enforcements_credential foreign key (partner_id, sdk_app_id, credential_id) references sdk_credentials(partner_id, sdk_app_id, id),
  constraint chk_sdk_credential_abuse_enforcements_type check (enforcement_type in ('cooldown', 'suspension', 'operator_override')),
  constraint chk_sdk_credential_abuse_enforcements_status check (status in ('active', 'expired', 'cleared')),
  constraint chk_sdk_credential_abuse_enforcements_source check (source in ('abuse_policy', 'operator'))
);

create index if not exists idx_sdk_credential_abuse_enforcements_partner_app_status_until
  on sdk_credential_abuse_enforcements (partner_id, sdk_app_id, status, effective_until);
create index if not exists idx_sdk_credential_abuse_enforcements_credential_status_until
  on sdk_credential_abuse_enforcements (credential_id, status, effective_until);

create table if not exists sdk_users (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  external_user_id_hash text not null,
  external_user_id_ciphertext text not null,
  profile_snapshot jsonb not null default '{}'::jsonb,
  status varchar(32) not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint uq_sdk_users_partner_external_hash unique (partner_id, external_user_id_hash),
  constraint uq_sdk_users_partner_id_id unique (partner_id, id),
  constraint chk_sdk_users_status check (status in ('active', 'disabled', 'revoked'))
);

create table if not exists sdk_config_drafts (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  status varchar(32) not null,
  draft_payload jsonb not null default '{}'::jsonb,
  created_by text not null,
  updated_by text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_sdk_config_drafts_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint chk_sdk_config_drafts_status check (status in ('draft', 'validating', 'validation_failed', 'published', 'discarded'))
);

create index if not exists idx_sdk_config_drafts_app_status_updated
  on sdk_config_drafts (sdk_app_id, status, updated_at desc);

create table if not exists sdk_config_versions (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  version bigint not null,
  published_payload jsonb not null default '{}'::jsonb,
  published_by text not null,
  published_at timestamptz not null default now(),
  constraint fk_sdk_config_versions_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint uq_sdk_config_versions_partner_app_version unique (partner_id, sdk_app_id, version),
  constraint uq_sdk_config_versions_partner_id_id unique (partner_id, id),
  constraint uq_sdk_config_versions_partner_app_id unique (partner_id, sdk_app_id, id)
);

create index if not exists idx_sdk_config_versions_partner_app_published
  on sdk_config_versions (partner_id, sdk_app_id, published_at desc);

create table if not exists sdk_active_configs (
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  active_config_version_id bigint not null,
  revision bigint not null,
  activated_by text not null,
  activated_at timestamptz not null,
  primary key (partner_id, sdk_app_id),
  constraint fk_sdk_active_configs_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_active_configs_version foreign key (partner_id, sdk_app_id, active_config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id)
);

create table if not exists sdk_config_activation_events (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  from_config_version_id bigint,
  to_config_version_id bigint not null,
  action varchar(32) not null,
  revision bigint not null,
  activated_by text not null,
  activated_at timestamptz not null,
  constraint fk_sdk_config_activation_events_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_config_activation_events_from_version foreign key (partner_id, sdk_app_id, from_config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_sdk_config_activation_events_to_version foreign key (partner_id, sdk_app_id, to_config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint uq_sdk_config_activation_events_revision unique (partner_id, sdk_app_id, revision),
  constraint chk_sdk_config_activation_events_action check (action in ('activate', 'rollback'))
);

create index if not exists idx_sdk_config_activation_events_app_activated
  on sdk_config_activation_events (sdk_app_id, activated_at desc);

create table if not exists sdk_sessions (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  credential_id bigint not null,
  external_user_id_hash text not null,
  platform text not null,
  sdk_version text not null,
  app_version text,
  status varchar(32) not null,
  pinned_config_version bigint,
  latest_permission_snapshot_id bigint,
  latest_runtime_snapshot_id bigint,
  created_at timestamptz not null default now(),
  expires_at timestamptz not null,
  revoked_at timestamptz,
  closed_at timestamptz,
  constraint fk_sdk_sessions_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_sessions_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_sessions_credential foreign key (partner_id, sdk_app_id, credential_id) references sdk_credentials(partner_id, sdk_app_id, id),
  constraint fk_sdk_sessions_pinned_config foreign key (partner_id, sdk_app_id, pinned_config_version) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint uq_sdk_sessions_partner_id_id unique (partner_id, id),
  constraint uq_sdk_sessions_partner_app_id unique (partner_id, sdk_app_id, id),
  constraint uq_sdk_sessions_partner_app_user_id unique (partner_id, sdk_app_id, sdk_user_id, id),
  constraint chk_sdk_sessions_status check (status in ('active', 'expired', 'revoked', 'closed'))
);

create index if not exists idx_sdk_sessions_partner_user_created
  on sdk_sessions (partner_id, sdk_user_id, created_at desc);
create index if not exists idx_sdk_sessions_credential_status on sdk_sessions (credential_id, status);
create index if not exists idx_sdk_sessions_status_expires on sdk_sessions (status, expires_at);
create index if not exists idx_sdk_sessions_app_status on sdk_sessions (sdk_app_id, status);

create table if not exists sdk_permission_snapshots (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_session_id bigint not null,
  config_version_id bigint not null,
  module_gate_summary jsonb not null default '{}'::jsonb,
  content_scope_summary jsonb not null default '{}'::jsonb,
  hard_deny_reasons text[] not null default array[]::text[],
  generated_at timestamptz not null default now(),
  constraint fk_sdk_permission_snapshots_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_permission_snapshots_session foreign key (partner_id, sdk_app_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, id),
  constraint fk_sdk_permission_snapshots_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint uq_sdk_permission_snapshots_partner_id_id unique (partner_id, id),
  constraint uq_sdk_permission_snapshots_partner_session_id unique (partner_id, sdk_session_id, id)
);

create index if not exists idx_sdk_permission_snapshots_session_generated
  on sdk_permission_snapshots (sdk_session_id, generated_at desc);
create index if not exists idx_sdk_permission_snapshots_config on sdk_permission_snapshots (config_version_id);

create table if not exists sdk_runtime_snapshots (
  id bigserial primary key,
  snapshot_schema_version int not null,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  modules jsonb not null default '{}'::jsonb,
  content_scope_summary jsonb not null default '{}'::jsonb,
  webview_policy jsonb not null default '{}'::jsonb,
  event_policy jsonb not null default '{}'::jsonb,
  entitlement_summary jsonb not null default '{}'::jsonb,
  device_policy_summary jsonb not null default '{}'::jsonb,
  version_policy jsonb not null default '{}'::jsonb,
  generated_at timestamptz not null default now(),
  constraint fk_sdk_runtime_snapshots_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_runtime_snapshots_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_runtime_snapshots_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_runtime_snapshots_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_sdk_runtime_snapshots_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint uq_sdk_runtime_snapshots_partner_id_id unique (partner_id, id),
  constraint uq_sdk_runtime_snapshots_partner_session_id unique (partner_id, sdk_session_id, id),
  constraint uq_sdk_runtime_snapshots_audit_binding unique (partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id)
);

create index if not exists idx_sdk_runtime_snapshots_session_generated
  on sdk_runtime_snapshots (sdk_session_id, generated_at desc);
create index if not exists idx_sdk_runtime_snapshots_config on sdk_runtime_snapshots (config_version_id);

alter table sdk_sessions
  add constraint fk_sdk_sessions_latest_permission_snapshot foreign key (partner_id, id, latest_permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  add constraint fk_sdk_sessions_latest_runtime_snapshot foreign key (partner_id, id, latest_runtime_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id);

create table if not exists sdk_token_records (
  jti text primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  credential_id bigint not null,
  status varchar(32) not null,
  issued_at timestamptz not null,
  expires_at timestamptz not null,
  revoked_at timestamptz,
  signing_key_id bigint not null references sdk_signing_keys(id),
  constraint fk_sdk_token_records_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_token_records_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_token_records_session foreign key (partner_id, sdk_session_id) references sdk_sessions(partner_id, id),
  constraint fk_sdk_token_records_session_identity foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_token_records_credential foreign key (partner_id, sdk_app_id, credential_id) references sdk_credentials(partner_id, sdk_app_id, id),
  constraint uq_sdk_token_records_partner_jti unique (partner_id, jti),
  constraint uq_sdk_token_records_session_jti unique (partner_id, sdk_app_id, sdk_user_id, sdk_session_id, jti),
  constraint chk_sdk_token_records_status check (status in ('active', 'expired', 'revoked'))
);

create index if not exists idx_sdk_token_records_session_status on sdk_token_records (sdk_session_id, status);
create index if not exists idx_sdk_token_records_credential_status on sdk_token_records (credential_id, status);
create index if not exists idx_sdk_token_records_signing_key_status on sdk_token_records (signing_key_id, status);
create index if not exists idx_sdk_token_records_partner_status_expires on sdk_token_records (partner_id, status, expires_at);

create table if not exists sdk_security_audit_logs (
  id bigserial primary key,
  partner_id bigint references sdk_partners(id),
  sdk_app_id bigint,
  sdk_user_id bigint,
  sdk_session_id bigint,
  credential_id bigint,
  token_jti text,
  event_type text not null,
  result text not null,
  internal_reason text not null,
  actor_type text not null,
  actor_ref text,
  operator_role text,
  operator_scope text,
  operator_reason text,
  request_id text not null,
  trace_id text,
  ip_hash text,
  user_agent_hash text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  constraint fk_sdk_security_audit_logs_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_security_audit_logs_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_security_audit_logs_session foreign key (partner_id, sdk_session_id) references sdk_sessions(partner_id, id),
  constraint fk_sdk_security_audit_logs_session_identity foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_security_audit_logs_credential foreign key (partner_id, credential_id) references sdk_credentials(partner_id, id),
  constraint fk_sdk_security_audit_logs_token foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id, token_jti) references sdk_token_records(partner_id, sdk_app_id, sdk_user_id, sdk_session_id, jti),
  constraint chk_sdk_security_audit_logs_partner_scope check (
    (sdk_app_id is null and sdk_user_id is null and sdk_session_id is null and credential_id is null and token_jti is null)
    or partner_id is not null
  ),
  constraint chk_sdk_security_audit_logs_session_scope check (
    sdk_session_id is null or (sdk_app_id is not null and sdk_user_id is not null)
  ),
  constraint chk_sdk_security_audit_logs_token_scope check (
    token_jti is null or (sdk_app_id is not null and sdk_user_id is not null and sdk_session_id is not null)
  ),
  constraint chk_sdk_security_audit_logs_operator check (
    actor_type <> 'operator'
    or (actor_ref is not null and operator_role is not null and operator_scope is not null and operator_reason is not null and request_id is not null and trace_id is not null)
  )
);

create index if not exists idx_sdk_security_audit_logs_partner_created
  on sdk_security_audit_logs (partner_id, created_at desc);
create index if not exists idx_sdk_security_audit_logs_credential_created
  on sdk_security_audit_logs (credential_id, created_at desc);
create index if not exists idx_sdk_security_audit_logs_event_created
  on sdk_security_audit_logs (event_type, created_at desc);
create index if not exists idx_sdk_security_audit_logs_request on sdk_security_audit_logs (request_id);

create table if not exists sdk_policy_audit_logs (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint,
  sdk_session_id bigint,
  capability text not null,
  action text not null,
  resource_ref text,
  config_version_id bigint,
  permission_snapshot_id bigint,
  decision varchar(32) not null,
  internal_reason text not null,
  request_id text not null,
  created_at timestamptz not null default now(),
  constraint fk_sdk_policy_audit_logs_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_policy_audit_logs_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_policy_audit_logs_session foreign key (partner_id, sdk_session_id) references sdk_sessions(partner_id, id),
  constraint fk_sdk_policy_audit_logs_session_identity foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_policy_audit_logs_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_sdk_policy_audit_logs_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint chk_sdk_policy_audit_logs_session_user check (sdk_session_id is null or sdk_user_id is not null),
  constraint chk_sdk_policy_audit_logs_permission_session check (permission_snapshot_id is null or sdk_session_id is not null),
  constraint chk_sdk_policy_audit_logs_decision check (decision in ('allow', 'deny', 'degrade'))
);

create index if not exists idx_sdk_policy_audit_logs_partner_capability_created
  on sdk_policy_audit_logs (partner_id, capability, created_at desc);
create index if not exists idx_sdk_policy_audit_logs_session_created
  on sdk_policy_audit_logs (sdk_session_id, created_at desc);

create table if not exists sdk_capability_audit_logs (
  id bigserial primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  capability text not null,
  resource_ref text,
  result text not null,
  internal_reason text not null,
  latency_ms bigint not null,
  request_id text not null,
  config_version_id bigint not null,
  permission_snapshot_id bigint not null,
  runtime_snapshot_id bigint not null,
  created_at timestamptz not null default now(),
  constraint fk_sdk_capability_audit_logs_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_capability_audit_logs_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_capability_audit_logs_session foreign key (partner_id, sdk_session_id) references sdk_sessions(partner_id, id),
  constraint fk_sdk_capability_audit_logs_session_identity foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_capability_audit_logs_config foreign key (partner_id, sdk_app_id, config_version_id) references sdk_config_versions(partner_id, sdk_app_id, id),
  constraint fk_sdk_capability_audit_logs_permission foreign key (partner_id, sdk_session_id, permission_snapshot_id) references sdk_permission_snapshots(partner_id, sdk_session_id, id),
  constraint fk_sdk_capability_audit_logs_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id, config_version_id, permission_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id),
  constraint chk_sdk_capability_audit_logs_result check (result in ('success', 'denied', 'failure', 'warning'))
);

create index if not exists idx_sdk_capability_audit_logs_partner_capability_created
  on sdk_capability_audit_logs (partner_id, capability, created_at desc);
create index if not exists idx_sdk_capability_audit_logs_session_created
  on sdk_capability_audit_logs (sdk_session_id, created_at desc);
create index if not exists idx_sdk_capability_audit_logs_request on sdk_capability_audit_logs (request_id);
