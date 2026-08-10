alter table device_sdk_events
  add column if not exists input_id text;

update device_sdk_events
set input_id = owner_device_event_id
where input_id is null;

alter table device_sdk_events
  alter column input_id set not null;

drop index if exists uq_device_sdk_events_partner_owner_event;

create unique index if not exists uq_device_sdk_events_identity_input
  on device_sdk_events (
    partner_id,
    sdk_app_id,
    sdk_user_id,
    sdk_session_id,
    input_id
  );

alter table device_sdk_output_delivery_attempts
  add column if not exists result_idempotency_key text;

alter table device_sdk_output_delivery_attempts
  add column if not exists result_device_context jsonb;
