create table if not exists llm_provider_configs (
  provider_key varchar(64) primary key,
  endpoint_url text not null default '',
  api_key_encrypted text not null default '',
  api_key_prefix varchar(32) not null default '',
  model_name varchar(128) not null default '',
  temperature double precision not null default 0.7,
  frequency_penalty double precision not null default 0,
  updated_at timestamptz not null default now()
);

create table if not exists llm_prompt_templates (
  id bigserial primary key,
  template_key varchar(128) not null unique,
  template_text text not null default '',
  updated_at timestamptz not null default now()
);

create table if not exists llm_sessions (
  id varchar(128) primary key,
  user_id bigint not null,
  character_id bigint not null references characters(id),
  timezone varchar(64) not null default '',
  scene_id bigint not null default 0,
  started_at timestamptz not null default now(),
  ended_at timestamptz
);

create table if not exists llm_messages (
  id bigserial primary key,
  session_id varchar(128) not null references llm_sessions(id) on delete cascade,
  role varchar(32) not null,
  content text not null default '',
  turn_number int not null default 1,
  created_at timestamptz not null default now()
);

create table if not exists llm_message_archives (
  id bigserial primary key,
  character_id bigint not null references characters(id),
  messages jsonb not null default '[]'::jsonb,
  total_turns int not null default 0,
  prompt_tokens int not null default 0,
  completion_tokens int not null default 0,
  archived_at timestamptz not null default now()
);

create table if not exists llm_usage_logs (
  id bigserial primary key,
  session_id varchar(128) not null,
  provider_key varchar(64) not null,
  model_name varchar(128) not null default '',
  prompt_tokens int not null default 0,
  completion_tokens int not null default 0,
  is_error boolean not null default false,
  error_message text not null default '',
  created_at timestamptz not null default now()
);

create index if not exists idx_llm_messages_session_created on llm_messages (session_id, created_at desc, id desc);
create index if not exists idx_llm_usage_logs_session on llm_usage_logs (session_id, created_at desc);
create index if not exists idx_llm_sessions_character on llm_sessions (character_id, started_at desc);
