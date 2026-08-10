create table if not exists character_profile_migration_archive (
  character_id bigint primary key,
  source_profile jsonb not null,
  archived_at timestamptz not null default now()
);

insert into character_profile_migration_archive (character_id, source_profile)
select c.id, to_jsonb(c)
from characters c
on conflict (character_id) do nothing;

alter table characters add column if not exists language varchar(8);
alter table characters add column if not exists role_orientation varchar(32);
alter table characters add column if not exists name varchar(128);
alter table characters add column if not exists marital_status varchar(128);
alter table characters add column if not exists occupation varchar(128);
alter table characters add column if not exists persona text;
alter table characters add column if not exists private_interests jsonb;
alter table characters add column if not exists personality_traits text;
alter table characters add column if not exists speaking_style text;
alter table characters add column if not exists photo_url text;

do $$
begin
  if exists (
    select 1
    from information_schema.columns
    where table_schema = 'public'
      and table_name = 'characters'
      and column_name = 'user_sexual_preference'
  ) then
    execute $sql$
      update characters
      set role_orientation = case user_sexual_preference
        when 'male_female' then 'straight_female'
        when 'female_male' then 'straight_male'
        when 'male_male_1' then 'male_top'
        when 'male_male_0' then 'male_bottom'
        when 'female_female_t' then 'female_t'
        when 'female_female_p' then 'female_p'
        else role_orientation
      end
      where user_sexual_preference is not null
        and user_sexual_preference <> ''
    $sql$;
  end if;
end $$;

with normalized as (
  select
    id,
    coalesce(nullif(language, ''), case when nullif(name_zh, '') is not null then 'zh' else 'en' end) as target_language
  from characters
)
update characters c
set
  language = n.target_language,
  -- The repo-owned bilingual schema did not carry this dimension explicitly.
  -- When the old preference column is absent, use the product default for legacy rows.
  role_orientation = coalesce(nullif(c.role_orientation, ''), 'straight_female'),
  name = coalesce(
    nullif(c.name, ''),
    case when n.target_language = 'en' then nullif(c.name_en, '') else nullif(c.name_zh, '') end,
    nullif(c.name_zh, ''),
    nullif(c.name_en, ''),
    null
  ),
  marital_status = coalesce(
    nullif(c.marital_status, ''),
    case when n.target_language = 'en' then nullif(c.marital_status_en, '') else nullif(c.marital_status_zh, '') end,
    nullif(c.marital_status_zh, ''),
    nullif(c.marital_status_en, ''),
    null
  ),
  occupation = coalesce(
    nullif(c.occupation, ''),
    case when n.target_language = 'en' then nullif(c.occupation_en, '') else nullif(c.occupation_zh, '') end,
    nullif(c.occupation_zh, ''),
    nullif(c.occupation_en, ''),
    null
  ),
  persona = coalesce(
    nullif(c.persona, ''),
    case when n.target_language = 'en' then nullif(c.description_en, '') else nullif(c.description_zh, '') end,
    nullif(c.description_zh, ''),
    nullif(c.description_en, ''),
    null
  ),
  private_interests = case
    when jsonb_typeof(c.private_interests) = 'array' and jsonb_array_length(c.private_interests) > 0 then c.private_interests
    when n.target_language = 'en' and jsonb_typeof(c.interests_en) = 'array' and jsonb_array_length(c.interests_en) > 0 then c.interests_en
    when jsonb_typeof(c.interests_zh) = 'array' and jsonb_array_length(c.interests_zh) > 0 then c.interests_zh
    when jsonb_typeof(c.interests_en) = 'array' and jsonb_array_length(c.interests_en) > 0 then c.interests_en
    else null
  end,
  personality_traits = coalesce(
    nullif(c.personality_traits, ''),
    case when n.target_language = 'en' then nullif(c.personality_en, '') else nullif(c.personality_zh, '') end,
    nullif(c.personality_zh, ''),
    nullif(c.personality_en, ''),
    null
  ),
  speaking_style = coalesce(
    nullif(c.speaking_style, ''),
    case when n.target_language = 'en' then nullif(c.traits_en, '') else nullif(c.traits_zh, '') end,
    case when n.target_language = 'en' then nullif(c.personality_en, '') else nullif(c.personality_zh, '') end,
    case when n.target_language = 'en' then nullif(c.description_en, '') else nullif(c.description_zh, '') end,
    nullif(c.traits_zh, ''),
    nullif(c.traits_en, ''),
    nullif(c.personality_zh, ''),
    nullif(c.personality_en, ''),
    nullif(c.description_zh, ''),
    nullif(c.description_en, ''),
    '自然、口语化地回应'
  ),
  photo_url = coalesce(nullif(c.photo_url, ''), nullif(c.video_placeholder_url, ''), '')
from normalized n
where c.id = n.id;

do $$
begin
  if exists (
    select 1
    from characters
    where language is null
       or role_orientation is null
       or name is null
       or name = ''
       or private_interests is null
       or private_interests = '[]'::jsonb
       or speaking_style is null
       or speaking_style = ''
  ) then
    raise exception 'canonical character migration requires source data for name and private_interests; legacy role_orientation and speaking_style are canonicalized by this migration';
  end if;
end $$;

alter table characters drop constraint if exists characters_language_check;
alter table characters drop constraint if exists characters_role_orientation_check;
alter table characters drop constraint if exists characters_category_check;
alter table characters drop constraint if exists characters_status_check;

alter table characters
  add constraint characters_language_check check (language in ('zh', 'en', 'ja')),
  add constraint characters_role_orientation_check check (role_orientation in ('straight_male', 'male_top', 'male_bottom', 'straight_female', 'female_p', 'female_t')),
  add constraint characters_category_check check (category in ('anime', 'real')),
  add constraint characters_status_check check (status in ('active', 'disabled'));

alter table characters alter column language set default 'zh';
alter table characters alter column language set not null;
alter table characters alter column role_orientation set default 'straight_female';
alter table characters alter column role_orientation set not null;
alter table characters alter column name set default '';
alter table characters alter column name set not null;
alter table characters alter column marital_status set default '';
alter table characters alter column marital_status set not null;
alter table characters alter column occupation set default '';
alter table characters alter column occupation set not null;
alter table characters alter column persona set default '';
alter table characters alter column persona set not null;
alter table characters alter column private_interests set default '[]'::jsonb;
alter table characters alter column private_interests set not null;
alter table characters alter column personality_traits set default '';
alter table characters alter column personality_traits set not null;
alter table characters alter column speaking_style set default '';
alter table characters alter column speaking_style set not null;
alter table characters alter column photo_url set default '';
alter table characters alter column photo_url set not null;

alter table characters
  drop column if exists name_zh,
  drop column if exists name_en,
  drop column if exists marital_status_zh,
  drop column if exists marital_status_en,
  drop column if exists occupation_zh,
  drop column if exists occupation_en,
  drop column if exists personality_zh,
  drop column if exists personality_en,
  drop column if exists description_zh,
  drop column if exists description_en,
  drop column if exists interests_zh,
  drop column if exists interests_en,
  drop column if exists traits_zh,
  drop column if exists traits_en,
  drop column if exists video_placeholder_url,
  drop column if exists user_sexual_preference;

create index if not exists idx_characters_admin_filters
  on characters (category, language, role_orientation, status, sort_order, created_at desc);
