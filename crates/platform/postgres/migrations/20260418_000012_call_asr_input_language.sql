alter table call_sessions
  add column if not exists asr_language varchar(16);

update call_sessions
set asr_language = 'zh'
where asr_language is null;

alter table call_sessions
  alter column asr_language set not null;
