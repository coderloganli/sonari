create table if not exists call_bot_speech_state (
  session_id bigint primary key references call_sessions(id) on delete cascade,
  active_item jsonb null,
  pending_items jsonb not null default '[]'::jsonb,
  completed_item_ids jsonb not null default '[]'::jsonb,
  updated_at timestamptz not null default now()
);

create index if not exists idx_call_bot_speech_state_updated_at
  on call_bot_speech_state(updated_at desc);
