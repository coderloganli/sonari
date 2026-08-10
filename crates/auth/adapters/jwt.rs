use async_trait::async_trait;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use shared_kernel::{AppError, AppResult, Claims, Role};

use crate::ports::{TokenPairView, TokenService};

#[derive(Debug, Clone)]
pub struct JwtTokenConfig {
    pub secret: String,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct JwtTokenService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    validation: Validation,
    config: JwtTokenConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JwtClaims {
    sub: String,
    user_id: i64,
    role: String,
    perms: Vec<String>,
    iat: i64,
    exp: i64,
}

impl JwtTokenService {
    pub fn new(config: JwtTokenConfig) -> Self {
        let secret = config.secret.clone();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;

        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            validation,
            config,
        }
    }

    fn issue_token(
        &self,
        subject_id: i64,
        role: &str,
        permissions: &[String],
        subject: &str,
        ttl: Duration,
    ) -> AppResult<String> {
        let now = Utc::now();
        let claims = JwtClaims {
            sub: subject.to_owned(),
            user_id: subject_id,
            role: role.to_owned(),
            perms: permissions.to_vec(),
            iat: now.timestamp(),
            exp: (now + ttl).timestamp(),
        };

        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(|_| AppError::internal("failed to sign token"))
    }

    fn parse_role(raw: &str) -> AppResult<Role> {
        match raw {
            "superadmin" | "admin" => Ok(Role::Admin),
            "user" => Ok(Role::User),
            _ => Err(AppError::unauthorized("invalid or expired token")),
        }
    }
}

#[async_trait]
impl TokenService for JwtTokenService {
    async fn issue_token_pair(
        &self,
        subject_id: i64,
        role: &str,
        permissions: &[String],
    ) -> AppResult<TokenPairView> {
        let access_token = self.issue_token(
            subject_id,
            role,
            permissions,
            "access",
            self.config.access_token_ttl,
        )?;
        let refresh_token = self.issue_token(
            subject_id,
            role,
            permissions,
            "refresh",
            self.config.refresh_token_ttl,
        )?;

        Ok(TokenPairView {
            access_token,
            refresh_token,
            expires_in_seconds: self.config.access_token_ttl.num_seconds(),
        })
    }

    async fn refresh_token_pair(&self, refresh_token: &str) -> AppResult<TokenPairView> {
        let token = decode::<JwtClaims>(refresh_token, &self.decoding_key, &self.validation)
            .map_err(|_| AppError::unauthorized("invalid or expired refresh token"))?;
        if token.claims.sub != "refresh" {
            return Err(AppError::unauthorized("invalid or expired refresh token"));
        }

        self.issue_token_pair(
            token.claims.user_id,
            &token.claims.role,
            &token.claims.perms,
        )
        .await
    }

    async fn validate_access_token(&self, access_token: &str) -> AppResult<Claims> {
        let token = decode::<JwtClaims>(access_token, &self.decoding_key, &self.validation)
            .map_err(|_| AppError::unauthorized("invalid or expired token"))?;
        if token.claims.sub != "access" {
            return Err(AppError::unauthorized("invalid or expired token"));
        }

        Ok(Claims {
            subject_id: token.claims.user_id,
            role: Self::parse_role(&token.claims.role)?,
        })
    }
}
