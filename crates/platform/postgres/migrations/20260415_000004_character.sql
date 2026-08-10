create table if not exists scenes (
  id bigserial primary key,
  name varchar(128) not null,
  location text not null default '',
  user_role text not null default '',
  relationship text not null default '',
  environment text not null default '',
  goal text not null default '',
  opening_event text not null default '',
  time_period_mode varchar(16) not null default 'auto',
  time_period varchar(64),
  status varchar(32) not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists voiceprints (
  id bigserial primary key,
  name varchar(128) not null,
  voice_id varchar(128) not null default '',
  remark text not null default '',
  status varchar(32) not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists characters (
  id bigserial primary key,
  name_zh varchar(128) not null default '',
  name_en varchar(128) not null default '',
  age int not null default 0,
  category varchar(32) not null default 'anime',
  marital_status_zh varchar(128) not null default '',
  marital_status_en varchar(128) not null default '',
  occupation_zh varchar(128) not null default '',
  occupation_en varchar(128) not null default '',
  personality_zh text not null default '',
  personality_en text not null default '',
  description_zh text not null default '',
  description_en text not null default '',
  interests_zh jsonb not null default '[]'::jsonb,
  interests_en jsonb not null default '[]'::jsonb,
  traits_zh text not null default '',
  traits_en text not null default '',
  avatar_url text not null default '',
  video_url text not null default '',
  video_placeholder_url text not null default '',
  voiceprint_id bigint not null references voiceprints(id),
  status varchar(32) not null default 'disabled',
  sort_order int not null default 0,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists character_scenes (
  character_id bigint not null references characters(id) on delete cascade,
  scene_id bigint not null references scenes(id) on delete cascade,
  sort_order int not null default 0,
  created_at timestamptz not null default now(),
  primary key (character_id, scene_id)
);

create table if not exists voiceprint_audio_assets (
  id bigserial primary key,
  voiceprint_id bigint not null references voiceprints(id) on delete cascade,
  band varchar(16) not null,
  audio_url text not null,
  sort_order int not null default 0,
  weight int not null default 1,
  status varchar(32) not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

alter table if exists scenes add column if not exists opening_event text not null default '';
alter table if exists voiceprints add column if not exists voice_id varchar(128) not null default '';
alter table if exists characters add column if not exists video_placeholder_url text not null default '';

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'characters' and column_name = 'video_placeholder'
  ) then
    execute 'update characters set video_placeholder_url = coalesce(video_placeholder_url, '''')';
    execute 'update characters set video_placeholder_url = video_placeholder where video_placeholder <> '''' and video_placeholder_url = ''''';
  end if;
end $$;

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'voiceprints' and column_name = 'tts_voice_id_minimax'
  ) then
    execute 'update voiceprints set voice_id = tts_voice_id_minimax where coalesce(voice_id, '''') = ''''';
  end if;
end $$;

do $$
begin
  if to_regclass('public.voiceprint_audio_clips') is not null then
    execute $sql$
      insert into voiceprint_audio_assets (voiceprint_id, band, audio_url, sort_order, weight, status, created_at, updated_at)
      select c.voiceprint_id,
             case c.category when 'medium' then 'mid' else c.category end,
             c.audio_url,
             c.sort_order,
             greatest(c.weight, 1),
             c.status,
             c.created_at,
             c.updated_at
      from voiceprint_audio_clips c
      where not exists (
        select 1
        from voiceprint_audio_assets a
        where a.voiceprint_id = c.voiceprint_id
          and a.band = case c.category when 'medium' then 'mid' else c.category end
          and a.audio_url = c.audio_url
      )
    $sql$;
  end if;
end $$;

create index if not exists idx_characters_status_sort on characters (status, sort_order, created_at desc);
create index if not exists idx_character_scenes_scene on character_scenes (scene_id, character_id);
create index if not exists idx_voiceprint_audio_assets_voiceprint_band on voiceprint_audio_assets (voiceprint_id, band, sort_order);
