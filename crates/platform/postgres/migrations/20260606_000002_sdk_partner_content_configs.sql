create table if not exists sdk_partner_content_configs (
  partner_id bigint primary key references sdk_partners(id),
  content_scope_summary jsonb not null default '{"mode":"all"}'::jsonb,
  updated_by text not null,
  updated_at timestamptz not null default now(),
  created_at timestamptz not null default now()
);

