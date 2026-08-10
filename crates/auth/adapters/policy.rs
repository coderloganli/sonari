use chrono::Duration;
use rand::Rng as _;
use sha2::{Digest, Sha256};

use crate::ports::{AuthPolicy, AuthSecrets};

#[derive(Debug, Clone)]
pub struct StaticAuthPolicy {
    pub admin_lock_threshold: i32,
    pub admin_lock_duration: Duration,
    pub sms_code_ttl: Duration,
    pub sms_send_cooldown: Duration,
}

impl AuthPolicy for StaticAuthPolicy {
    fn admin_lock_threshold(&self) -> i32 {
        self.admin_lock_threshold
    }

    fn admin_lock_duration(&self) -> Duration {
        self.admin_lock_duration
    }

    fn sms_code_ttl(&self) -> Duration {
        self.sms_code_ttl
    }

    fn sms_send_cooldown(&self) -> Duration {
        self.sms_send_cooldown
    }
}

#[derive(Debug, Clone, Default)]
pub struct StaticAuthSecrets {
    pub fixed_sms_code: Option<String>,
}

impl StaticAuthSecrets {
    fn hash_hex(raw: &str) -> String {
        let digest = Sha256::digest(raw.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl AuthSecrets for StaticAuthSecrets {
    fn generate_sms_code(&self) -> String {
        if let Some(code) = &self.fixed_sms_code
            && !code.trim().is_empty()
        {
            return code.clone();
        }

        let mut rng = rand::rng();
        format!("{:06}", rng.random_range(0..1_000_000))
    }

    fn should_send_sms(&self) -> bool {
        self.fixed_sms_code
            .as_ref()
            .map(|code| code.trim().is_empty())
            .unwrap_or(true)
    }

    fn hash_sms_code(&self, raw_code: &str) -> String {
        Self::hash_hex(raw_code)
    }
}
