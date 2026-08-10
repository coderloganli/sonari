alter table if exists app_error_reports
  add column if not exists request_id text not null default '';

alter table if exists client_debug_events
  add column if not exists request_id text not null default '';

create index if not exists idx_app_error_reports_request_id
  on app_error_reports (request_id);

create index if not exists idx_client_debug_events_request_id
  on client_debug_events (request_id);
