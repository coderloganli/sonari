alter table call_bot_speech_state
  add column if not exists handled_item_ids jsonb not null default '[]'::jsonb;
