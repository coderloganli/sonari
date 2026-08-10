create table if not exists agent_settings (
  config_key varchar(64) primary key,
  recent_turns int not null,
  updated_at timestamptz not null default now(),
  constraint agent_settings_default_key check (config_key = 'default'),
  constraint agent_settings_recent_turns_range check (recent_turns between 1 and 20)
);

insert into agent_settings (config_key, recent_turns, updated_at)
select
  'default',
  greatest(
    1,
    least(
      20,
      case
        when value ~ '^-?[0-9]+$' then value::int
        else 6
      end
    )
  ),
  now()
from system_configs
where key = 'llm_recent_turns'
on conflict (config_key) do update set
  recent_turns = excluded.recent_turns,
  updated_at = excluded.updated_at;

insert into agent_settings (config_key, recent_turns, updated_at)
values ('default', 6, now())
on conflict (config_key) do nothing;

delete from system_configs
where key = 'llm_recent_turns';
