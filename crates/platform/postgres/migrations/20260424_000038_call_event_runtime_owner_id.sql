alter table call_event_outbox
  add column if not exists runtime_owner_id text;

alter table call_events
  add column if not exists runtime_owner_id text;

create index if not exists idx_call_event_outbox_runtime_owner_id
  on call_event_outbox (runtime_owner_id)
  where runtime_owner_id is not null;

create index if not exists idx_call_events_runtime_owner_id
  on call_events (runtime_owner_id)
  where runtime_owner_id is not null;
