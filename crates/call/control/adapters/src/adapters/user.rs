use std::sync::Arc;

use async_trait::async_trait;
use call::ports::{CallUserContext, CallUserContextPort};
use shared_kernel::{AppError, AppResult};

#[derive(Clone)]
pub struct UserContextAdapter {
    service: Arc<dyn user_context::UserCallContextReadPort>,
}

impl UserContextAdapter {
    pub fn new(service: Arc<dyn user_context::UserCallContextReadPort>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl CallUserContextPort for UserContextAdapter {
    async fn get_user_context(&self, user_id: i64) -> AppResult<CallUserContext> {
        let context = self.service.get_call_context(user_id).await?;
        let timezone = context
            .timezone
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::invalid_input("user timezone is not configured"))?;
        if timezone.trim().is_empty() {
            return Err(AppError::invalid_input("user timezone is not configured"));
        }
        Ok(CallUserContext {
            user_id: context.user_id,
            timezone,
            needs_profile_completion: context.needs_profile_completion,
        })
    }
}
