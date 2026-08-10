alter table call_bot_speech_state
  add column if not exists started_item_ids jsonb not null default '[]'::jsonb;
