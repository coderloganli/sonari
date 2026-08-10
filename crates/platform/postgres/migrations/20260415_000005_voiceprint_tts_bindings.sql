create table if not exists voiceprint_tts_bindings (
  id bigserial primary key,
  voiceprint_id bigint not null references voiceprints(id) on delete cascade,
  provider varchar(32) not null,
  voice_id varchar(128) not null,
  voice_name varchar(128),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (voiceprint_id, provider)
);

insert into voiceprint_tts_bindings (voiceprint_id, provider, voice_id, voice_name)
select id, 'minimax', voice_id, null
from voiceprints
where coalesce(voice_id, '') <> ''
  and not exists (
    select 1
    from voiceprint_tts_bindings b
    where b.voiceprint_id = voiceprints.id
      and b.provider = 'minimax'
  );

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'voiceprints' and column_name = 'tts_voice_id_minimax'
  ) then
    execute $sql$
      insert into voiceprint_tts_bindings (voiceprint_id, provider, voice_id, voice_name)
      select id, 'minimax', tts_voice_id_minimax, tts_voice_name_minimax
      from voiceprints
      where coalesce(tts_voice_id_minimax, '') <> ''
        and not exists (
          select 1
          from voiceprint_tts_bindings b
          where b.voiceprint_id = voiceprints.id
            and b.provider = 'minimax'
        )
    $sql$;
  end if;
end $$;

do $$
begin
  if exists (
    select 1 from information_schema.columns
    where table_name = 'voiceprints' and column_name = 'tts_voice_id_elevenlabs'
  ) then
    execute $sql$
      insert into voiceprint_tts_bindings (voiceprint_id, provider, voice_id, voice_name)
      select id, 'elevenlabs', tts_voice_id_elevenlabs, null
      from voiceprints
      where coalesce(tts_voice_id_elevenlabs, '') <> ''
        and not exists (
          select 1
          from voiceprint_tts_bindings b
          where b.voiceprint_id = voiceprints.id
            and b.provider = 'elevenlabs'
        )
    $sql$;
  end if;
end $$;

create index if not exists idx_voiceprint_tts_bindings_voiceprint
  on voiceprint_tts_bindings (voiceprint_id, provider);
