alter table llm_sessions add column if not exists caller_kind varchar(32);
alter table llm_sessions add column if not exists sdk_partner_id bigint;
alter table llm_sessions add column if not exists sdk_app_id bigint;
alter table llm_sessions add column if not exists sdk_user_id bigint;
alter table llm_sessions add column if not exists sdk_session_id bigint;
alter table llm_sessions add column if not exists runtime_snapshot_id bigint;
alter table llm_sessions add column if not exists external_user_id_hash text;

update llm_sessions
set caller_kind = 'platform_user'
where caller_kind is null;

alter table llm_sessions alter column user_id drop not null;
alter table llm_sessions alter column scene_id drop not null;
alter table llm_sessions alter column caller_kind set not null;

alter table llm_sessions
  drop constraint if exists chk_llm_sessions_caller_identity,
  add constraint chk_llm_sessions_caller_identity check (
    (
      caller_kind = 'platform_user'
      and user_id is not null
      and sdk_partner_id is null
      and sdk_app_id is null
      and sdk_user_id is null
      and sdk_session_id is null
      and runtime_snapshot_id is null
      and external_user_id_hash is null
    )
    or
    (
      caller_kind = 'sdk_user'
      and user_id is null
      and sdk_partner_id is not null
      and sdk_app_id is not null
      and sdk_user_id is not null
      and sdk_session_id is not null
      and runtime_snapshot_id is not null
      and length(trim(external_user_id_hash)) > 0
    )
  );

alter table call_sessions add column if not exists caller_kind varchar(32);
alter table call_sessions add column if not exists sdk_partner_id bigint;
alter table call_sessions add column if not exists sdk_app_id bigint;
alter table call_sessions add column if not exists sdk_user_id bigint;
alter table call_sessions add column if not exists sdk_session_id bigint;
alter table call_sessions add column if not exists runtime_snapshot_id bigint;
alter table call_sessions add column if not exists external_user_id_hash text;
alter table call_sessions add column if not exists realtime_participant_identity text;

update call_sessions
set caller_kind = 'platform_user',
    realtime_participant_identity = concat('platform_user:', user_id)
where caller_kind is null;

alter table call_sessions alter column user_id drop not null;
alter table call_sessions alter column scene_id drop not null;
alter table call_sessions alter column scene_name drop not null;
alter table call_sessions alter column caller_kind set not null;
alter table call_sessions alter column realtime_participant_identity set not null;

alter table call_sessions
  drop constraint if exists chk_call_sessions_caller_identity,
  add constraint chk_call_sessions_caller_identity check (
    (
      caller_kind = 'platform_user'
      and user_id is not null
      and sdk_partner_id is null
      and sdk_app_id is null
      and sdk_user_id is null
      and sdk_session_id is null
      and runtime_snapshot_id is null
      and external_user_id_hash is null
    )
    or
    (
      caller_kind = 'sdk_user'
      and user_id is null
      and sdk_partner_id is not null
      and sdk_app_id is not null
      and sdk_user_id is not null
      and sdk_session_id is not null
      and runtime_snapshot_id is not null
      and length(trim(external_user_id_hash)) > 0
    )
  );

alter table call_sessions
  drop constraint if exists chk_call_sessions_realtime_participant_identity_not_empty,
  add constraint chk_call_sessions_realtime_participant_identity_not_empty
    check (length(trim(realtime_participant_identity)) > 0);

alter table call_sessions
  drop constraint if exists chk_call_sessions_scene_snapshot_pair,
  add constraint chk_call_sessions_scene_snapshot_pair check (
    (
      scene_id is null
      and scene_name is null
    )
    or
    (
      scene_id is not null
      and scene_name is not null
      and length(trim(scene_name)) > 0
    )
  );

create table if not exists sdk_call_sessions (
  id bigserial primary key,
  partner_id bigint not null,
  sdk_app_id bigint not null,
  sdk_user_id bigint not null,
  sdk_session_id bigint not null,
  runtime_snapshot_id bigint not null,
  sdk_call_id text not null,
  call_session_id bigint not null references call_sessions(id) on delete cascade,
  entitlement_decision_ref text not null,
  created_at timestamptz not null default now(),
  constraint fk_sdk_call_sessions_app foreign key (partner_id, sdk_app_id) references sdk_apps(partner_id, id),
  constraint fk_sdk_call_sessions_user foreign key (partner_id, sdk_user_id) references sdk_users(partner_id, id),
  constraint fk_sdk_call_sessions_session foreign key (partner_id, sdk_app_id, sdk_user_id, sdk_session_id) references sdk_sessions(partner_id, sdk_app_id, sdk_user_id, id),
  constraint fk_sdk_call_sessions_runtime_snapshot foreign key (partner_id, sdk_session_id, runtime_snapshot_id) references sdk_runtime_snapshots(partner_id, sdk_session_id, id),
  constraint uq_sdk_call_sessions_identity_call unique (partner_id, sdk_app_id, sdk_user_id, sdk_call_id),
  constraint uq_sdk_call_sessions_call_session unique (call_session_id),
  constraint chk_sdk_call_sessions_call_id_not_empty check (length(trim(sdk_call_id)) > 0),
  constraint chk_sdk_call_sessions_entitlement_ref_not_empty check (length(trim(entitlement_decision_ref)) > 0)
);

create index if not exists idx_sdk_call_sessions_runtime_snapshot
  on sdk_call_sessions (partner_id, sdk_app_id, sdk_user_id, sdk_session_id, runtime_snapshot_id);
