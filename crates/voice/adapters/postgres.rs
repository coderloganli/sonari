use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Row};

use crate::domain::AsrLanguage;
use crate::ports::VoiceConfigRepository;

#[derive(Debug, Clone)]
pub struct PostgresVoiceConfigRepository {
    pool: PgPool,
}

impl PostgresVoiceConfigRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_sqlx_error(err: sqlx::Error) -> AppError {
    match err {
        sqlx::Error::RowNotFound => AppError::not_found("record not found"),
        other => AppError::internal(format!("postgres error: {other}")),
    }
}

fn parse_asr_language(raw: &str) -> AppResult<AsrLanguage> {
    AsrLanguage::parse(raw).ok_or_else(|| AppError::internal("invalid ASR language"))
}

#[async_trait]
impl VoiceConfigRepository for PostgresVoiceConfigRepository {
    async fn get_asr_input_language(&self) -> AppResult<AsrLanguage> {
        let row =
            sqlx::query("select value from system_configs where key = 'voice_asr_input_language'")
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
        let row =
            row.ok_or_else(|| AppError::not_found("voice ASR input language config not found"))?;
        parse_asr_language(row.get::<&str, _>("value"))
    }

    async fn set_asr_input_language(&self, language: AsrLanguage) -> AppResult<()> {
        sqlx::query(
            "insert into system_configs (key, value, updated_at) values ('voice_asr_input_language', $1, now()) on conflict (key) do update set value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(language.as_str())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}
