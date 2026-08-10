alter table users
  add column if not exists timezone varchar(64);
