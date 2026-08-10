//! Call events are written straight to `call_events`.
//!
//! Producer and reader share a process, so there is no stream to fan out
//! through and no consumer group to read it back.

use async_trait::async_trait;
use call::{CallEvent, ports::EventSinkPort};
use shared_kernel::{AppError, AppResult};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PostgresCallEventLogSink {
    pool: PgPool,
}

impl PostgresCallEventLogSink {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventSinkPort for PostgresCallEventLogSink {
    async fn publish(&self, event: CallEvent) -> AppResult<()> {
        // `stream_message_id` existed to deduplicate stream redelivery. Nothing
        // redelivers now, so the database supplies a unique value per row.
        sqlx::query(
            "insert into call_events \
             (stream_message_id, session_id, round_id, source, event, ts_ms, fields) \
             values (gen_random_uuid()::text, $1, $2, $3, $4, $5, $6)",
        )
        .bind(event.session_id)
        .bind(event.round_id)
        .bind(event.source)
        .bind(event.event)
        .bind(event.ts_ms)
        .bind(event.fields)
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::unavailable(format!("failed to write call event: {error}")))?;
        Ok(())
    }
}
