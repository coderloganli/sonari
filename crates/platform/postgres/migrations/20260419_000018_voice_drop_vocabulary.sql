alter table voice_suppliers
  drop column if exists vocabulary_id;

delete from system_configs
where key = 'alibaba.asr.vocabulary';
