alter table sdk_capability_audit_logs
  drop constraint if exists chk_sdk_capability_audit_logs_result;

alter table sdk_capability_audit_logs
  add constraint chk_sdk_capability_audit_logs_result
  check (result in ('success', 'denied', 'failure', 'warning'));
