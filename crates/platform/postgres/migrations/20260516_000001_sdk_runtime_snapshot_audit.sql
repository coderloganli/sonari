alter table sdk_policy_audit_logs
  add column if not exists runtime_snapshot_id bigint;

alter table sdk_capability_audit_logs
  add column if not exists runtime_snapshot_id bigint;

delete from sdk_policy_audit_logs audit
where audit.sdk_session_id is not null
  and (
    audit.runtime_snapshot_id is null
    or audit.config_version_id is null
    or audit.permission_snapshot_id is null
    or not exists (
      select 1
      from sdk_runtime_snapshots snapshot
      where snapshot.partner_id = audit.partner_id
        and snapshot.sdk_session_id = audit.sdk_session_id
        and snapshot.id = audit.runtime_snapshot_id
        and snapshot.config_version_id = audit.config_version_id
        and snapshot.permission_snapshot_id = audit.permission_snapshot_id
    )
  );

delete from sdk_capability_audit_logs audit
where audit.runtime_snapshot_id is null
  or audit.config_version_id is null
  or audit.permission_snapshot_id is null
  or not exists (
    select 1
    from sdk_runtime_snapshots snapshot
    where snapshot.partner_id = audit.partner_id
      and snapshot.sdk_session_id = audit.sdk_session_id
      and snapshot.id = audit.runtime_snapshot_id
      and snapshot.config_version_id = audit.config_version_id
      and snapshot.permission_snapshot_id = audit.permission_snapshot_id
  );

alter table sdk_capability_audit_logs
  alter column runtime_snapshot_id set not null;

alter table sdk_capability_audit_logs
  alter column config_version_id set not null;

alter table sdk_capability_audit_logs
  alter column permission_snapshot_id set not null;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'uq_sdk_runtime_snapshots_audit_binding'
  ) then
    alter table sdk_runtime_snapshots
      add constraint uq_sdk_runtime_snapshots_audit_binding
      unique (partner_id, sdk_session_id, id, config_version_id, permission_snapshot_id);
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'fk_sdk_policy_audit_logs_runtime_snapshot'
  ) then
    alter table sdk_policy_audit_logs
      add constraint fk_sdk_policy_audit_logs_runtime_snapshot
      foreign key (
        partner_id,
        sdk_session_id,
        runtime_snapshot_id,
        config_version_id,
        permission_snapshot_id
      )
      references sdk_runtime_snapshots(
        partner_id,
        sdk_session_id,
        id,
        config_version_id,
        permission_snapshot_id
      );
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'chk_sdk_policy_audit_logs_runtime_snapshot_session'
  ) then
    alter table sdk_policy_audit_logs
      add constraint chk_sdk_policy_audit_logs_runtime_snapshot_session
      check (runtime_snapshot_id is null or sdk_session_id is not null);
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'chk_sdk_policy_audit_logs_runtime_snapshot_required_when_session_bound'
  ) then
    alter table sdk_policy_audit_logs
      add constraint chk_sdk_policy_audit_logs_runtime_snapshot_required_when_session_bound
      check (sdk_session_id is null or runtime_snapshot_id is not null);
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'chk_sdk_policy_audit_logs_config_version_required_when_session_bound'
  ) then
    alter table sdk_policy_audit_logs
      add constraint chk_sdk_policy_audit_logs_config_version_required_when_session_bound
      check (sdk_session_id is null or config_version_id is not null);
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'chk_sdk_policy_audit_logs_permission_snapshot_required_when_session_bound'
  ) then
    alter table sdk_policy_audit_logs
      add constraint chk_sdk_policy_audit_logs_permission_snapshot_required_when_session_bound
      check (sdk_session_id is null or permission_snapshot_id is not null);
  end if;
end $$;

do $$
begin
  if not exists (
    select 1 from pg_constraint
    where conname = 'fk_sdk_capability_audit_logs_runtime_snapshot'
  ) then
    alter table sdk_capability_audit_logs
      add constraint fk_sdk_capability_audit_logs_runtime_snapshot
      foreign key (
        partner_id,
        sdk_session_id,
        runtime_snapshot_id,
        config_version_id,
        permission_snapshot_id
      )
      references sdk_runtime_snapshots(
        partner_id,
        sdk_session_id,
        id,
        config_version_id,
        permission_snapshot_id
      );
  end if;
end $$;
