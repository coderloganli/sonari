alter table admins add column if not exists password_hash text;
alter table admins add column if not exists status varchar(32) not null default 'active';
alter table admins add column if not exists login_fail_count int not null default 0;
alter table admins add column if not exists locked_until timestamptz null;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_name = 'admins' and column_name = 'password'
  ) then
    execute 'update admins set password_hash = password where password_hash is null';
  end if;
end $$;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_name = 'admins' and column_name = 'login_fails'
  ) then
    execute 'update admins set login_fail_count = login_fails where login_fail_count = 0';
  end if;
end $$;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_name = 'admins' and column_name = 'lock_until'
  ) then
    execute 'update admins set locked_until = lock_until where locked_until is null';
  end if;
end $$;

alter table sms_codes add column if not exists code_hash text;
alter table sms_codes add column if not exists used_at timestamptz null;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_name = 'sms_codes' and column_name = 'code'
  ) then
    execute 'update sms_codes set code_hash = code where code_hash is null';
  end if;
end $$;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_name = 'sms_codes' and column_name = 'used'
  ) then
    execute 'update sms_codes set used_at = created_at where used = true and used_at is null';
  end if;
end $$;
