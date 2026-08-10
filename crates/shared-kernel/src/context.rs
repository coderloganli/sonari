use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub subject_id: i64,
    pub role: Role,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestContext {
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub user_id: Option<i64>,
}
