create table if not exists entitlement_grants (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  entitlement_ref text not null,
  capability text not null,
  mode varchar(32) not null,
  status varchar(32) not null,
  plan_ref text,
  plan_display_name text,
  quota_ref text,
  quota_remaining bigint,
  quota_limit bigint,
  quota_reset_at timestamptz,
  expires_at timestamptz,
  owner_version text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint fk_entitlement_grants_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_entitlement_grants_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint chk_entitlement_grants_mode check (mode in ('record_only', 'soft_gate', 'hard_gate')),
  constraint chk_entitlement_grants_status check (status in ('active', 'disabled', 'expired')),
  constraint chk_entitlement_grants_ref_not_empty check (length(trim(entitlement_ref)) > 0),
  constraint chk_entitlement_grants_capability_not_empty check (length(trim(capability)) > 0),
  constraint chk_entitlement_grants_owner_version_not_empty check (length(trim(owner_version)) > 0),
  constraint chk_entitlement_grants_quota_remaining_non_negative check (quota_remaining is null or quota_remaining >= 0),
  constraint chk_entitlement_grants_quota_limit_non_negative check (quota_limit is null or quota_limit >= 0)
);

create unique index if not exists uq_entitlement_grants_active_ref_capability
  on entitlement_grants (partner_id, sdk_app_id, sdk_user_id, entitlement_ref, capability)
  where status = 'active';

create index if not exists idx_entitlement_grants_identity_capability
  on entitlement_grants (partner_id, sdk_app_id, sdk_user_id, capability, status);

create index if not exists idx_entitlement_grants_expires
  on entitlement_grants (expires_at)
  where expires_at is not null;

create table if not exists entitlement_usage_events (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  capability text not null,
  usage_ref text not null,
  usage_idempotency_key text not null,
  owner_version text not null,
  created_at timestamptz not null default now(),
  constraint fk_entitlement_usage_events_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_entitlement_usage_events_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint chk_entitlement_usage_events_capability_not_empty check (length(trim(capability)) > 0),
  constraint chk_entitlement_usage_events_ref_not_empty check (length(trim(usage_ref)) > 0),
  constraint chk_entitlement_usage_events_key_not_empty check (length(trim(usage_idempotency_key)) > 0),
  constraint chk_entitlement_usage_events_owner_version_not_empty check (length(trim(owner_version)) > 0),
  constraint uq_entitlement_usage_events_idempotency unique (partner_id, sdk_app_id, sdk_user_id, usage_idempotency_key)
);

create index if not exists idx_entitlement_usage_events_identity_created
  on entitlement_usage_events (partner_id, sdk_app_id, sdk_user_id, created_at desc);
