alter table call_events
  add column if not exists stream_message_id varchar(128) null;

create unique index if not exists idx_call_events_stream_message_id
  on call_events (stream_message_id)
  where stream_message_id is not null;
