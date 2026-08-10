alter table device_sdk_events
  drop constraint if exists device_sdk_events_owner_device_event_id_key;

create unique index if not exists uq_device_sdk_events_partner_owner_event
  on device_sdk_events (partner_id, owner_device_event_id);

drop index if exists uq_device_sdk_output_attempts_idempotency;

alter table device_sdk_output_delivery_attempts
  add column if not exists lease_dedupe_active boolean not null default true;

with ranked_leases as (
  select
    partner_id,
    sdk_app_id,
    sdk_user_id,
    sdk_session_id,
    lease_idempotency_key,
    delivery_attempt_id,
    dense_rank() over (
      partition by partner_id, sdk_app_id, sdk_user_id, sdk_session_id, lease_idempotency_key
      order by max(id) desc
    ) as lease_rank
  from device_sdk_output_delivery_attempts
  group by
    partner_id,
    sdk_app_id,
    sdk_user_id,
    sdk_session_id,
    lease_idempotency_key,
    delivery_attempt_id
)
update device_sdk_output_delivery_attempts stale
set lease_dedupe_active = false
from ranked_leases ranked
where stale.partner_id = ranked.partner_id
  and stale.sdk_app_id = ranked.sdk_app_id
  and stale.sdk_user_id = ranked.sdk_user_id
  and stale.sdk_session_id = ranked.sdk_session_id
  and stale.lease_idempotency_key = ranked.lease_idempotency_key
  and stale.delivery_attempt_id = ranked.delivery_attempt_id
  and ranked.lease_rank > 1;

create unique index if not exists uq_device_sdk_output_attempts_idempotency
  on device_sdk_output_delivery_attempts (
    partner_id,
    sdk_app_id,
    sdk_user_id,
    sdk_session_id,
    lease_idempotency_key,
    command_id
  )
  where lease_dedupe_active;
