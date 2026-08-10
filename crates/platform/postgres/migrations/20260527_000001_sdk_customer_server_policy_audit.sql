alter table sdk_policy_audit_logs
  drop constraint if exists chk_sdk_policy_audit_logs_runtime_snapshot_required_when_session_bound,
  drop constraint if exists chk_sdk_policy_audit_logs_config_version_required_when_session_bound,
  drop constraint if exists chk_sdk_policy_audit_logs_permission_snapshot_required_when_session_bound,
  drop constraint if exists chk_sdk_policy_audit_logs_runtime_snapshot_complete;

alter table sdk_policy_audit_logs
  add column if not exists metadata jsonb not null default '{}'::jsonb;

alter table sdk_policy_audit_logs
  add constraint chk_sdk_policy_audit_logs_runtime_snapshot_complete
  check (
    (
      runtime_snapshot_id is null
      and permission_snapshot_id is null
      and (
        (
          capability = 'sdk.identity.exchange_token'
          and sdk_session_id is null
        )
        or (
          -- Planned customer-server purchase grants must not be added here
          -- until their route, writer contract, and conformance tests are
          -- promoted together.
          capability in (
            'sdk.identity.renew_token',
            'sdk.identity.revoke_session'
          )
          and sdk_session_id is not null
        )
      )
    )
    or (
      runtime_snapshot_id is not null
      and sdk_session_id is not null
      and config_version_id is not null
      and permission_snapshot_id is not null
    )
  );
