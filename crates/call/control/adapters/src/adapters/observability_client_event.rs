use async_trait::async_trait;
use call::{CallEvent, ports::EventSinkPort};
use observability::{
    ClientDebugEvent, ClientDebugEventSinkPort, ObservabilityError, ObservabilityResult,
};
use serde_json::{Map, Value};

#[derive(Clone)]
pub struct ClientDebugEventCallSinkAdapter<S> {
    sink: S,
}

impl<S> ClientDebugEventCallSinkAdapter<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }
}

fn should_bridge_client_event(event: &ClientDebugEvent) -> bool {
    if matches!(event.level.as_str(), "error" | "warning" | "warn") {
        return true;
    }

    matches!(
        event.name.as_str(),
        "call_started"
            | "call_ended"
            | "call_entry_call_ended"
            | "call_start_api_succeeded"
            | "call_runtime_livekit_connect_requested"
            | "call_runtime_livekit_connected"
            | "call_runtime_bot_track_ready"
            | "call_runtime_bot_track_lost"
            | "call_runtime_bot_playback_started"
            | "call_runtime_bot_playback_stopped"
            | "call_runtime_reconnecting"
            | "call_runtime_reconnected"
            | "call_runtime_disconnected"
            | "call_runtime_provider_failed"
            | "call_microphone_toggle_requested"
            | "call_audio_route_toggle_requested"
            | "cue_play"
            | "cue_blocked"
            | "cue_stop"
            | "audio_interruption_began"
            | "audio_interruption_ended"
            | "audio_route_changed"
    )
}

#[async_trait]
impl<S> ClientDebugEventSinkPort for ClientDebugEventCallSinkAdapter<S>
where
    S: EventSinkPort + Send + Sync,
{
    async fn publish_client_event(&self, event: ClientDebugEvent) -> ObservabilityResult<()> {
        if !should_bridge_client_event(&event) {
            return Ok(());
        }

        let session_id = event
            .session_id
            .as_deref()
            .ok_or_else(|| {
                ObservabilityError::invalid_input("session_id is required for call client events")
            })?
            .parse::<i64>()
            .map_err(|_| ObservabilityError::invalid_input("session_id must be numeric"))?;

        let mut fields = match event.fields {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            _ => {
                return Err(ObservabilityError::invalid_input(
                    "client event fields must be a JSON object",
                ));
            }
        };
        if let Some(device_name) = event.device_name {
            fields.insert("device_name".to_owned(), Value::String(device_name));
        }
        if let Some(character_id) = event.character_id {
            fields.insert("character_id".to_owned(), Value::String(character_id));
        }
        if let Some(flow_id) = event.flow_id {
            fields.insert("flow_id".to_owned(), Value::String(flow_id));
        }
        if let Some(event_class) = event.event_class {
            fields.insert("event_class".to_owned(), Value::String(event_class));
        }
        if event.merged_count > 1 {
            fields.insert(
                "merged_count".to_owned(),
                Value::Number(serde_json::Number::from(event.merged_count)),
            );
        }

        self.sink
            .publish(CallEvent {
                session_id,
                round_id: None,
                source: "client".to_owned(),
                event: event.name,
                ts_ms: event.ts_ms,
                fields: Value::Object(fields),
            })
            .await
            .map_err(|error| ObservabilityError::internal(error.message.into_owned()))
    }
}
