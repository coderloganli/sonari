create table if not exists users (
  id bigserial primary key,
  phone varchar(32) not null unique,
  nickname varchar(64) not null default '',
  language varchar(8) not null default 'zh',
  status varchar(32) not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists admins (
  id bigserial primary key,
  username varchar(128) not null unique,
  password_hash text not null,
  role varchar(32) not null,
  permissions jsonb not null default '[]',
  status varchar(32) not null default 'active',
  login_fail_count int not null default 0,
  locked_until timestamptz null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists api_keys (
  id bigserial primary key,
  key_hash text not null unique,
  key_prefix varchar(32) not null,
  status varchar(32) not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists sms_codes (
  id bigserial primary key,
  phone varchar(32) not null,
  code_hash text not null,
  expires_at timestamptz not null,
  used_at timestamptz null,
  created_at timestamptz not null default now()
);

create index if not exists idx_sms_codes_phone_created
  on sms_codes (phone, created_at desc);
