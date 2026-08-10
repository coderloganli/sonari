use super::types::{LiveKitParticipantIdentity, LiveKitRoomName};

pub fn room_name_for_session(session_id: i64) -> LiveKitRoomName {
    LiveKitRoomName::new(format!("call-{session_id}"))
}

pub fn user_identity_for_user(user_id: i64) -> LiveKitParticipantIdentity {
    LiveKitParticipantIdentity::new(format!(
        "user-{}",
        sanitize_identity_component(&user_id.to_string())
    ))
}

pub fn bot_identity_for_session(session_id: i64) -> LiveKitParticipantIdentity {
    LiveKitParticipantIdentity::new(format!(
        "bot-{}",
        sanitize_identity_component(&session_id.to_string())
    ))
}

pub fn sanitize_identity_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitize_identity_component_replaces_unsupported_chars() {
        assert_eq!(
            super::sanitize_identity_component("user:@example.com"),
            "user--example.com"
        );
    }

    #[test]
    fn room_name_for_session_uses_stable_prefix() {
        assert_eq!(super::room_name_for_session(42).as_str(), "call-42");
    }

    #[test]
    fn identity_helpers_return_typed_values() {
        assert_eq!(super::user_identity_for_user(21).as_str(), "user-21");
        assert_eq!(super::bot_identity_for_session(42).as_str(), "bot-42");
    }
}
