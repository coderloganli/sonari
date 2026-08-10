drop index if exists idx_call_events_stream_message_id;

create unique index if not exists idx_call_events_stream_message_id
  on call_events (stream_message_id);
