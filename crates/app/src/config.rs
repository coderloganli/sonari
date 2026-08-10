use anyhow::{Context, Result};
use chrono::Duration;
use serde::{Deserialize, Serialize};

use platform_postgres::PostgresConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub instance_id: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 8080,
            instance_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub postgres: PostgresConfig,
    pub secrets: SecretsConfig,
    pub sdk: SdkConfig,
    pub auth: AuthConfig,
    pub livekit: LiveKitConfig,
    pub internal_runtime_secret: String,
    pub internal_runtime_advertise_url: String,
    pub runtime_owner_id: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let mut cfg = Self::default();

        if let Ok(host) = std::env::var("SERVER_HOST")
            && !host.trim().is_empty()
        {
            cfg.server.host = host;
        }

        if let Ok(port) = std::env::var("SERVER_PORT")
            && let Ok(parsed) = port.parse::<u16>()
        {
            cfg.server.port = parsed;
        }

        if let Ok(instance_id) = std::env::var("SERVER_INSTANCE_ID")
            && !instance_id.trim().is_empty()
        {
            cfg.server.instance_id = Some(instance_id);
        }

        if let Ok(dsn) = std::env::var("DATABASE_DSN")
            && !dsn.trim().is_empty()
        {
            cfg.postgres.dsn = dsn;
        }

        if let Ok(secret) = std::env::var("VOICE_SECRETS_KEY")
            && !secret.trim().is_empty()
        {
            cfg.secrets.voice = secret;
        }

        if let Ok(secret) = std::env::var("AGENT_SECRETS_KEY")
            && !secret.trim().is_empty()
        {
            cfg.secrets.agent = secret;
        }

        if let Ok(key_id) = std::env::var("SDK_CREDENTIAL_MATERIAL_ACTIVE_KEY_ID")
            && !key_id.trim().is_empty()
        {
            cfg.sdk.credential_material_active_key_id = key_id;
        }

        if let Ok(key_ring) = std::env::var("SDK_CREDENTIAL_MATERIAL_KEY_RING_JSON")
            && !key_ring.trim().is_empty()
        {
            cfg.sdk.credential_material_key_ring_json = key_ring;
        }

        if let Ok(key_id) = std::env::var("SDK_EXTERNAL_USER_MATERIAL_ACTIVE_KEY_ID")
            && !key_id.trim().is_empty()
        {
            cfg.sdk.external_user_material_active_key_id = key_id;
        }

        if let Ok(key_ring) = std::env::var("SDK_EXTERNAL_USER_MATERIAL_KEY_RING_JSON")
            && !key_ring.trim().is_empty()
        {
            cfg.sdk.external_user_material_key_ring_json = key_ring;
        }

        if let Ok(key_id) = std::env::var("SDK_SIGNING_MATERIAL_ACTIVE_KEY_ID")
            && !key_id.trim().is_empty()
        {
            cfg.sdk.signing_material_active_key_id = key_id;
        }

        if let Ok(key_ring) = std::env::var("SDK_SIGNING_MATERIAL_KEY_RING_JSON")
            && !key_ring.trim().is_empty()
        {
            cfg.sdk.signing_material_key_ring_json = key_ring;
        }

        if let Ok(issuer) = std::env::var("SDK_JWT_ISSUER")
            && !issuer.trim().is_empty()
        {
            cfg.sdk.jwt_issuer = issuer;
        }

        if let Ok(audience) = std::env::var("SDK_JWT_AUDIENCE")
            && !audience.trim().is_empty()
        {
            cfg.sdk.jwt_audience = audience;
        }

        if let Ok(enabled) = std::env::var("SDK_ENABLED") {
            cfg.sdk.enabled = parse_bool_env(&enabled, "SDK_ENABLED")?;
        }

        if let Ok(enabled) = std::env::var("SDK_TOKEN_EXCHANGE_ENABLED") {
            cfg.sdk.token_exchange_enabled =
                parse_bool_env(&enabled, "SDK_TOKEN_EXCHANGE_ENABLED")?;
        }

        if let Ok(enabled) = std::env::var("SDK_RUNTIME_INITIALIZE_ENABLED") {
            cfg.sdk.runtime_initialize_enabled =
                parse_bool_env(&enabled, "SDK_RUNTIME_INITIALIZE_ENABLED")?;
        }

        if let Ok(seconds) = std::env::var("SDK_CREDENTIAL_ABUSE_BUCKET_SECONDS") {
            cfg.sdk.credential_abuse_bucket_seconds = seconds
                .trim()
                .parse::<i64>()
                .with_context(|| "SDK_CREDENTIAL_ABUSE_BUCKET_SECONDS must be a signed integer")?;
        }

        if let Ok(seconds) = std::env::var("SDK_CREDENTIAL_TIMESTAMP_WINDOW_SECONDS") {
            cfg.sdk.credential_timestamp_window_seconds =
                seconds.trim().parse::<i64>().with_context(
                    || "SDK_CREDENTIAL_TIMESTAMP_WINDOW_SECONDS must be a signed integer",
                )?;
        }

        if let Ok(secret) = std::env::var("JWT_SECRET")
            && !secret.trim().is_empty()
        {
            cfg.auth.jwt.secret = secret;
        }

        if let Ok(ttl) = std::env::var("JWT_ACCESS_TTL_MINUTES")
            && let Ok(parsed) = ttl.parse::<i64>()
        {
            cfg.auth.jwt.access_token_ttl_minutes = parsed;
        }

        if let Ok(ttl) = std::env::var("JWT_REFRESH_TTL_DAYS")
            && let Ok(parsed) = ttl.parse::<i64>()
        {
            cfg.auth.jwt.refresh_token_ttl_days = parsed;
        }

        if let Ok(provider) = std::env::var("SMS_PROVIDER")
            && !provider.trim().is_empty()
        {
            cfg.auth.sms.provider = provider;
        }

        if let Ok(sign_name) = std::env::var("SMS_SIGN_NAME")
            && !sign_name.trim().is_empty()
        {
            cfg.auth.sms.sign_name = sign_name;
        }

        if let Ok(template_code) = std::env::var("SMS_TEMPLATE_CODE")
            && !template_code.trim().is_empty()
        {
            cfg.auth.sms.template_code = template_code;
        }

        if let Ok(role_name) = std::env::var("SMS_ROLE_NAME")
            && !role_name.trim().is_empty()
        {
            cfg.auth.sms.role_name = role_name;
        }

        if let Ok(access_key_id) = std::env::var("SMS_ACCESS_KEY_ID")
            && !access_key_id.trim().is_empty()
        {
            cfg.auth.sms.access_key_id = access_key_id;
        }

        if let Ok(access_key_secret) = std::env::var("SMS_ACCESS_KEY_SECRET")
            && !access_key_secret.trim().is_empty()
        {
            cfg.auth.sms.access_key_secret = access_key_secret;
        }

        if let Ok(url) = std::env::var("LIVEKIT_URL")
            && !url.trim().is_empty()
        {
            cfg.livekit.url = url;
        }

        if let Ok(url) = std::env::var("LIVEKIT_PUBLIC_URL")
            && !url.trim().is_empty()
        {
            cfg.livekit.public_url = url;
        }

        if let Ok(api_key) = std::env::var("LIVEKIT_API_KEY")
            && !api_key.trim().is_empty()
        {
            cfg.livekit.api_key = api_key;
        }

        if let Ok(api_secret) = std::env::var("LIVEKIT_API_SECRET")
            && !api_secret.trim().is_empty()
        {
            cfg.livekit.api_secret = api_secret;
        }
        if let Ok(secret) = std::env::var("INTERNAL_RUNTIME_SECRET")
            && !secret.trim().is_empty()
        {
            cfg.internal_runtime_secret = secret;
        }

        if let Ok(url) = std::env::var("INTERNAL_RUNTIME_ADVERTISE_URL")
            && !url.trim().is_empty()
        {
            cfg.internal_runtime_advertise_url = url;
        }

        if let Ok(runtime_owner_id) = std::env::var("RUNTIME_OWNER_ID")
            && !runtime_owner_id.trim().is_empty()
        {
            cfg.runtime_owner_id = Some(runtime_owner_id);
        }

        if let Ok(fixed_code) = std::env::var("SMS_FIXED_CODE")
            && !fixed_code.trim().is_empty()
        {
            cfg.auth.sms.fixed_code = Some(fixed_code);
        }

        Ok(cfg)
    }
}

fn parse_bool_env(value: &str, env_name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{env_name} must be a boolean: true/false, yes/no, on/off, or 1/0"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub jwt: JwtConfig,
    pub sms: SmsConfig,
    pub policy: AuthPolicyConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecretsConfig {
    pub voice: String,
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkConfig {
    pub enabled: bool,
    pub credential_material_active_key_id: String,
    pub credential_material_key_ring_json: String,
    pub external_user_material_active_key_id: String,
    pub external_user_material_key_ring_json: String,
    pub signing_material_active_key_id: String,
    pub signing_material_key_ring_json: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub token_exchange_enabled: bool,
    pub runtime_initialize_enabled: bool,
    pub credential_abuse_bucket_seconds: i64,
    pub credential_timestamp_window_seconds: i64,
}

impl Default for SdkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            credential_material_active_key_id: String::new(),
            credential_material_key_ring_json: String::new(),
            external_user_material_active_key_id: String::new(),
            external_user_material_key_ring_json: String::new(),
            signing_material_active_key_id: String::new(),
            signing_material_key_ring_json: String::new(),
            jwt_issuer: "sonari".to_owned(),
            jwt_audience: "sonari-client".to_owned(),
            token_exchange_enabled: false,
            runtime_initialize_enabled: false,
            credential_abuse_bucket_seconds: 60,
            credential_timestamp_window_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JwtConfig {
    pub secret: String,
    pub access_token_ttl_minutes: i64,
    pub refresh_token_ttl_days: i64,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: "dev-secret".to_owned(),
            access_token_ttl_minutes: 30,
            refresh_token_ttl_days: 7,
        }
    }
}

impl JwtConfig {
    pub fn access_token_ttl(&self) -> Duration {
        Duration::minutes(self.access_token_ttl_minutes)
    }

    pub fn refresh_token_ttl(&self) -> Duration {
        Duration::days(self.refresh_token_ttl_days)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SmsConfig {
    pub provider: String,
    pub sign_name: String,
    pub template_code: String,
    pub role_name: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub fixed_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LiveKitConfig {
    pub url: String,
    pub public_url: String,
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPolicyConfig {
    pub admin_lock_threshold: i32,
    pub admin_lock_duration_minutes: i64,
    pub sms_code_ttl_minutes: i64,
    pub sms_send_cooldown_seconds: i64,
}

impl Default for AuthPolicyConfig {
    fn default() -> Self {
        Self {
            admin_lock_threshold: 5,
            admin_lock_duration_minutes: 15,
            sms_code_ttl_minutes: 5,
            sms_send_cooldown_seconds: 60,
        }
    }
}

impl AuthPolicyConfig {
    pub fn admin_lock_duration(&self) -> Duration {
        Duration::minutes(self.admin_lock_duration_minutes)
    }

    pub fn sms_code_ttl(&self) -> Duration {
        Duration::minutes(self.sms_code_ttl_minutes)
    }

    pub fn sms_send_cooldown(&self) -> Duration {
        Duration::seconds(self.sms_send_cooldown_seconds)
    }
}
