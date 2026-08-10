alter table call_event_outbox
  add column if not exists dispatch_attempts integer not null default 0,
  add column if not exists claimed_at timestamptz null,
  add column if not exists claim_owner varchar(128) null,
  add column if not exists dispatched_at timestamptz null;

create index if not exists idx_call_event_outbox_dispatch_pending
  on call_event_outbox (dispatched_at, id);

create index if not exists idx_call_event_outbox_claimed_at
  on call_event_outbox (claimed_at);
