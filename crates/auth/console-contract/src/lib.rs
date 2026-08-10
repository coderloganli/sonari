use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shared_kernel::AppResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminConsoleAdminItem {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub permissions: Vec<String>,
    pub status: String,
    pub is_locked: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait AdminConsoleQueryPort: Send + Sync {
    async fn list_admins(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AdminConsoleAdminItem>, i64)>;
    async fn list_permissions(&self) -> AppResult<Vec<String>>;
}

#[async_trait]
impl<T> AdminConsoleQueryPort for std::sync::Arc<T>
where
    T: AdminConsoleQueryPort + ?Sized,
{
    async fn list_admins(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AdminConsoleAdminItem>, i64)> {
        (**self).list_admins(page, page_size).await
    }

    async fn list_permissions(&self) -> AppResult<Vec<String>> {
        (**self).list_permissions().await
    }
}
