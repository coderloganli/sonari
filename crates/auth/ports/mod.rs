use async_trait::async_trait;
use shared_kernel::{AppResult, Claims};

#[async_trait]
pub trait AdminRepository: Send + Sync {
    async fn find_by_username(&self, username: &str) -> AppResult<Option<crate::domain::Admin>>;
    async fn reconcile_bootstrap_admin(
        &self,
        username: &str,
        password_hash: &str,
        role: crate::domain::AdminRole,
        permissions: &[String],
    ) -> AppResult<()>;
    async fn update_login_state(
        &self,
        admin_id: i64,
        login_fail_count: i32,
        locked_until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait AdminConsoleReadPort: Send + Sync {
    async fn list_admin_rows(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<crate::domain::Admin>, i64)>;
}

#[async_trait]
pub trait PermissionCatalogPort: Send + Sync {
    async fn list_permission_keys(&self) -> AppResult<Vec<String>>;
}

#[async_trait]
pub trait SmsCodeRepository: Send + Sync {
    async fn find_latest_by_phone(&self, phone: &str) -> AppResult<Option<crate::domain::SmsCode>>;
    async fn save(&self, sms_code: &crate::domain::SmsCode) -> AppResult<()>;
    async fn mark_used(
        &self,
        sms_code_id: i64,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait UserIdentityRepository: Send + Sync {
    async fn find_by_phone(&self, phone: &str) -> AppResult<Option<crate::domain::UserIdentity>>;
    async fn create_with_phone(&self, phone: &str) -> AppResult<crate::domain::UserIdentity>;
}

#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash_password(&self, plain_text: &str) -> AppResult<String>;
    async fn verify_password(&self, password_hash: &str, plain_text: &str) -> AppResult<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPairView {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_seconds: i64,
}

#[async_trait]
pub trait TokenService: Send + Sync {
    async fn issue_token_pair(
        &self,
        subject_id: i64,
        role: &str,
        permissions: &[String],
    ) -> AppResult<TokenPairView>;

    async fn refresh_token_pair(&self, refresh_token: &str) -> AppResult<TokenPairView>;

    async fn validate_access_token(&self, access_token: &str) -> AppResult<Claims>;
}

#[async_trait]
impl<T> TokenService for std::sync::Arc<T>
where
    T: TokenService + ?Sized,
{
    async fn issue_token_pair(
        &self,
        subject_id: i64,
        role: &str,
        permissions: &[String],
    ) -> AppResult<TokenPairView> {
        (**self)
            .issue_token_pair(subject_id, role, permissions)
            .await
    }

    async fn refresh_token_pair(&self, refresh_token: &str) -> AppResult<TokenPairView> {
        (**self).refresh_token_pair(refresh_token).await
    }

    async fn validate_access_token(&self, access_token: &str) -> AppResult<Claims> {
        (**self).validate_access_token(access_token).await
    }
}

#[async_trait]
pub trait SmsSender: Send + Sync {
    async fn send_code(&self, phone: &str, code: &str) -> AppResult<()>;
}

#[async_trait]
impl<T> SmsSender for Box<T>
where
    T: SmsSender + ?Sized,
{
    async fn send_code(&self, phone: &str, code: &str) -> AppResult<()> {
        (**self).send_code(phone, code).await
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

pub trait AuthSecrets: Send + Sync {
    fn generate_sms_code(&self) -> String;
    fn should_send_sms(&self) -> bool;
    fn hash_sms_code(&self, raw_code: &str) -> String;
}

pub trait AuthPolicy: Send + Sync {
    fn admin_lock_threshold(&self) -> i32;
    fn admin_lock_duration(&self) -> chrono::Duration;
    fn sms_code_ttl(&self) -> chrono::Duration;
    fn sms_send_cooldown(&self) -> chrono::Duration;
}
