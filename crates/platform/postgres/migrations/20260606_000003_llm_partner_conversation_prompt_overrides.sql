create table if not exists llm_partner_conversation_prompt_overrides (
  partner_id bigint primary key references sdk_partners(id) on delete cascade,
  system_prompt_1 text not null default '',
  system_prompt_2 text not null default '',
  system_prompt_3 text not null default '',
  welcome_user_prompt text not null default '',
  updated_by varchar(128) not null default '',
  updated_at timestamptz not null default now(),
  created_at timestamptz not null default now()
);

