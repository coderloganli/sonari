alter table users add column if not exists system_language varchar(8);
alter table users add column if not exists character_language varchar(8);
alter table users add column if not exists user_sexual_preferences text[];

update users
set
  system_language = coalesce(nullif(system_language, ''), nullif(language, ''), 'zh'),
  character_language = coalesce(nullif(character_language, ''), nullif(language, '')),
  user_sexual_preferences = coalesce(user_sexual_preferences, array[]::text[]);

alter table users alter column system_language set default 'zh';
alter table users alter column character_language drop default;
alter table users alter column user_sexual_preferences set default array[]::text[];

alter table users alter column system_language set not null;
alter table users alter column character_language drop not null;
alter table users alter column user_sexual_preferences set not null;

alter table users drop constraint if exists users_system_language_check;
alter table users drop constraint if exists users_character_language_check;
alter table users drop constraint if exists users_user_sexual_preferences_check;
alter table users drop constraint if exists chk_users_system_language;
alter table users drop constraint if exists chk_users_character_language;
alter table users drop constraint if exists chk_users_user_sexual_preferences;

alter table users
  add constraint chk_users_system_language check (system_language in ('zh', 'en')),
  add constraint chk_users_character_language check (
    character_language is null or character_language in ('zh', 'en', 'ja')
  ),
  add constraint chk_users_user_sexual_preferences check (
    user_sexual_preferences <@ array[
      'male_female',
      'female_male',
      'male_male_1',
      'male_male_0',
      'male_male_0_5',
      'female_female_t',
      'female_female_p',
      'female_female_h'
    ]::text[]
  );

create index if not exists idx_users_character_profile
  on users (character_language);
