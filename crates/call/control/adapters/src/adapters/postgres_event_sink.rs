use async_trait::async_trait;
use call::{CallEvent, ports::EventSinkPort};
use shared_kernel::{AppError, AppResult};
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct PostgresCallEventOutboxSink {
    pool: PgPool,
}

impl PostgresCallEventOutboxSink {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventSinkPort for PostgresCallEventOutboxSink {
    async fn publish(&self, event: CallEvent) -> AppResult<()> {
        sqlx::query(
            r#"
            insert into call_event_outbox (
              session_id,
              round_id,
              source,
              event,
              ts_ms,
              fields
            )
            values ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(event.session_id)
        .bind(event.round_id)
        .bind(event.source)
        .bind(event.event)
        .bind(event.ts_ms)
        .bind(event.fields)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            AppError::internal(format!("failed to write call event into outbox: {error}"))
        })?;
        Ok(())
    }
}
