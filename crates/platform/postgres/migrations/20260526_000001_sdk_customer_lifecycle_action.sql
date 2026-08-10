alter table sdk_customer_session_bindings
  drop constraint if exists chk_sdk_customer_session_bindings_lifecycle_action;

alter table sdk_customer_session_bindings
  add constraint chk_sdk_customer_session_bindings_lifecycle_action
  check (
    data_lifecycle_action is null
    or data_lifecycle_action in ('clear_runtime_only', 'clear_local_cache', 'clear_all_sdk_local_state')
  );
