alter table speech_runtime_configs
  add column if not exists silence_force_agent_ms integer;

update speech_runtime_configs
set silence_force_agent_ms = 1000
where silence_force_agent_ms is null;

alter table speech_runtime_configs
  alter column silence_force_agent_ms set not null;
