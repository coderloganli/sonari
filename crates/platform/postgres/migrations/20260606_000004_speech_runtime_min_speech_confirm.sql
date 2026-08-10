alter table speech_runtime_configs
  add column if not exists min_speech_confirm_ms integer default 150;

update speech_runtime_configs
set min_speech_confirm_ms = 150
where min_speech_confirm_ms is null;

alter table speech_runtime_configs
  alter column min_speech_confirm_ms set not null;
