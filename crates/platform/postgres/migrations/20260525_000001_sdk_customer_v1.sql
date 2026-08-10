create table sdk_customer_session_bindings (
  id bigint primary key,
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  external_user_id_hash text not null,
  session_subject_hash text not null,
  app_context jsonb not null default '{}'::jsonb,
  consent_context jsonb not null default '{}'::jsonb,
  token_binding jsonb not null default '{}'::jsonb,
  data_lifecycle_action text,
  status varchar(32) not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  revoked_at timestamptz,
  constraint fk_sdk_customer_session_bindings_app
    foreign key (partner_id, sdk_app_id)
    references sdk_apps(partner_id, id),
  constraint fk_sdk_customer_session_bindings_session
    foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id)
    references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint uq_sdk_customer_session_bindings_session
    unique (partner_id, sdk_app_id, sdk_session_id),
  constraint chk_sdk_customer_session_bindings_status
    check (status in ('active', 'revoked')),
  constraint chk_sdk_customer_session_bindings_lifecycle_action
    check (
      data_lifecycle_action is null
      or data_lifecycle_action in ('clear_runtime_only', 'clear_all_sdk_local_state')
    )
);

create index idx_sdk_customer_session_bindings_user
  on sdk_customer_session_bindings (partner_id, sdk_user_id, created_at desc);

create index idx_sdk_customer_session_bindings_external_user
  on sdk_customer_session_bindings (partner_id, sdk_app_id, external_user_id_hash);

create table sdk_customer_nonces (
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  nonce_hash text not null,
  seen_at timestamptz not null default now(),
  expires_at timestamptz not null,
  primary key (partner_id, sdk_app_id, nonce_hash),
  constraint fk_sdk_customer_nonces_app
    foreign key (partner_id, sdk_app_id)
    references sdk_apps(partner_id, id)
);

create index idx_sdk_customer_nonces_expires
  on sdk_customer_nonces (expires_at);

create table sdk_customer_idempotency_records (
  partner_id bigint not null references sdk_partners(id),
  sdk_app_id bigint not null,
  operation text not null,
  idempotency_key text not null,
  request_digest text not null,
  response_projection jsonb not null default '{}'::jsonb,
  expires_at timestamptz not null,
  created_at timestamptz not null default now(),
  primary key (partner_id, sdk_app_id, operation, idempotency_key),
  constraint fk_sdk_customer_idempotency_records_app
    foreign key (partner_id, sdk_app_id)
    references sdk_apps(partner_id, id)
);

create index idx_sdk_customer_idempotency_records_expires
  on sdk_customer_idempotency_records (expires_at);
