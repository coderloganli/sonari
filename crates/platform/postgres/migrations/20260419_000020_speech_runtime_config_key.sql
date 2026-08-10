alter table speech_runtime_configs
  add column if not exists config_key varchar(64);

update speech_runtime_configs
set config_key = 'threshold'
where config_key is null;

alter table speech_runtime_configs
  alter column config_key set not null;

alter table speech_runtime_configs
  drop constraint if exists speech_runtime_configs_id_check;

alter table speech_runtime_configs
  drop constraint if exists speech_runtime_configs_check;

alter table speech_runtime_configs
  drop constraint if exists speech_runtime_configs_pkey;

alter table speech_runtime_configs
  add primary key (config_key);

alter table speech_runtime_configs
  drop column if exists id;
