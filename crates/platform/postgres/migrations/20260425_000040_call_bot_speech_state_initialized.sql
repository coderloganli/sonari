alter table call_bot_speech_state
  add column if not exists initialized boolean not null default false;

update call_bot_speech_state
set initialized = true
where initialized = false;
