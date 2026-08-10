insert into system_configs (key, value, updated_at)
values ('voice_asr_input_language', 'zh', now())
on conflict (key) do nothing;
