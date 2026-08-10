create table if not exists auth_permission_catalog (
  permission_key text primary key,
  sort_order int not null
);

insert into auth_permission_catalog (permission_key, sort_order)
values
  ('dashboard:view', 10),
  ('admin:manage', 20),
  ('user:manage', 30),
  ('content:manage', 40),
  ('llm:manage', 50),
  ('voice:manage', 60)
on conflict (permission_key) do update
set sort_order = excluded.sort_order;
