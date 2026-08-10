update call_bot_speech_state
set completed_item_ids = '[]'::jsonb
where completed_item_ids is null;
