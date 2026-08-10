use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserCharacterLanguage {
    Zh,
    En,
    Ja,
}

impl UserCharacterLanguage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
            Self::Ja => "ja",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            "ja" => Some(Self::Ja),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserCallContext {
    pub user_id: i64,
    pub timezone: Option<String>,
    pub needs_profile_completion: bool,
}

#[async_trait]
pub trait UserCallContextReadPort: Send + Sync {
    async fn get_call_context(&self, user_id: i64) -> AppResult<UserCallContext>;
}

#[async_trait]
impl<T> UserCallContextReadPort for Arc<T>
where
    T: UserCallContextReadPort + Send + Sync + ?Sized,
{
    async fn get_call_context(&self, user_id: i64) -> AppResult<UserCallContext> {
        (**self).get_call_context(user_id).await
    }
}
