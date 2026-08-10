create table if not exists voice_suppliers (
  id bigserial primary key,
  type varchar(16) not null,
  name varchar(128) not null,
  provider varchar(32) not null,
  endpoint_url text not null default '',
  api_key_encrypted text not null default '',
  api_key_prefix varchar(32) not null default '',
  model_name varchar(128) not null default '',
  is_active boolean not null default false,
  app_id varchar(128),
  access_key varchar(128),
  resource_id varchar(128),
  vocabulary_id varchar(128),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create index if not exists idx_voice_suppliers_type_created on voice_suppliers (type, created_at desc);
create index if not exists idx_voice_suppliers_type_active on voice_suppliers (type, is_active);
create index if not exists idx_voice_suppliers_type_provider on voice_suppliers (type, provider);
