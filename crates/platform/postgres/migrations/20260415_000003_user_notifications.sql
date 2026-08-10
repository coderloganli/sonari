create table if not exists notifications (
  id bigserial primary key,
  user_id bigint null references users(id),
  type varchar(32) not null default 'system',
  title_zh text not null default '',
  title_en text not null default '',
  content_zh text not null default '',
  content_en text not null default '',
  created_at timestamptz not null default now()
);

create index if not exists idx_notifications_created on notifications (created_at desc);
create index if not exists idx_notifications_user_id on notifications (user_id);

create table if not exists notification_reads (
  user_id bigint not null references users(id),
  notification_id bigint not null references notifications(id),
  read_at timestamptz not null default now(),
  primary key (user_id, notification_id)
);

alter table notifications add column if not exists user_id bigint null references users(id);
alter table notifications add column if not exists title_zh text not null default '';
alter table notifications add column if not exists title_en text not null default '';
alter table notifications add column if not exists content_zh text not null default '';
alter table notifications add column if not exists content_en text not null default '';

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'notifications' and column_name = 'title'
  ) then
    execute 'update notifications set title_zh = title where title_zh = ''''';
    execute 'update notifications set title_en = title where title_en = ''''';
  end if;
end $$;

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'notifications' and column_name = 'content'
  ) then
    execute 'update notifications set content_zh = content where content_zh = ''''';
    execute 'update notifications set content_en = content where content_en = ''''';
  end if;
end $$;
