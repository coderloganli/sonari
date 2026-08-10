-- Bring already-migrated databases to the final signing-key schema used by the base SDK schema:
-- pending keys are first-class, and only one key may be active for signing.
alter table sdk_signing_keys
  drop constraint if exists chk_sdk_signing_keys_status;

alter table sdk_signing_keys
  add constraint chk_sdk_signing_keys_status
  check (status in ('pending', 'active', 'verifying_only', 'retired', 'revoked'));

with ranked_active as (
  select
    id,
    row_number() over (order by active_from desc, id desc) as active_rank
  from sdk_signing_keys
  where status = 'active'
)
update sdk_signing_keys
set status = 'verifying_only',
    active_until = coalesce(active_until, now()),
    verify_until = coalesce(verify_until, now() + interval '30 days'),
    updated_at = now()
where id in (
  select id from ranked_active where active_rank > 1
);

create unique index if not exists idx_sdk_signing_keys_single_active
  on sdk_signing_keys ((true))
  where status = 'active';
