#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerInitiatedTurnTrigger {
    CallStarted,
}

impl ServerInitiatedTurnTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CallStarted => "call_started",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInitiatedTurnRequest {
    pub session_id: i64,
    pub agent_session_id: String,
    pub trigger: ServerInitiatedTurnTrigger,
}
