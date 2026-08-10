use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdminRole {
    SuperAdmin,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    Active,
    Disabled,
}

pub type PermissionKey = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admin {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: AdminRole,
    pub permissions: Vec<PermissionKey>,
    pub status: AccountStatus,
    pub login_fail_count: i32,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsCode {
    pub id: i64,
    pub phone: String,
    pub code_hash: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserIdentity {
    pub user_id: i64,
    pub phone: String,
    pub status: AccountStatus,
}
