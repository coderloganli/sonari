create table if not exists system_configs (
  key varchar(128) primary key,
  value text not null default '',
  updated_at timestamptz not null default now()
);

insert into system_configs (key, value, updated_at)
values ('voice_driver', 'websocket', now())
on conflict (key) do nothing;

insert into system_configs (key, value, updated_at)
values ('llm_recent_turns', '6', now())
on conflict (key) do nothing;
