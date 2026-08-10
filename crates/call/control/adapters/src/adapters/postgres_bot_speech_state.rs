use async_trait::async_trait;
use call::{BotSpeechState, BotSpeechStateStorePort};
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct PostgresBotSpeechStateStoreAdapter {
    pool: PgPool,
}

impl PostgresBotSpeechStateStoreAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BotSpeechStateStorePort for PostgresBotSpeechStateStoreAdapter {
    async fn load_state(&self, session_id: i64) -> AppResult<Option<BotSpeechState>> {
        let row = sqlx::query(
            r#"
            select initialized, active_item, pending_items, handled_item_ids, started_item_ids, completed_item_ids
            from call_bot_speech_state
            where session_id = $1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::internal(format!("failed to load bot speech state: {error}")))?;

        row.map(|row| {
            let active_item = row
                .try_get::<Option<serde_json::Value>, _>("active_item")
                .map_err(|error| {
                    AppError::internal(format!("failed to read active_item: {error}"))
                })?
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| {
                    AppError::internal(format!("failed to decode active_item: {error}"))
                })?;
            let initialized = row.try_get::<bool, _>("initialized").map_err(|error| {
                AppError::internal(format!("failed to read initialized: {error}"))
            })?;
            let pending_items =
                serde_json::from_value::<Vec<_>>(row.try_get("pending_items").map_err(
                    |error| AppError::internal(format!("failed to read pending_items: {error}")),
                )?)
                .map_err(|error| {
                    AppError::internal(format!("failed to decode pending_items: {error}"))
                })?;
            let started_item_ids =
                serde_json::from_value::<Vec<String>>(row.try_get("started_item_ids").map_err(
                    |error| AppError::internal(format!("failed to read started_item_ids: {error}")),
                )?)
                .map_err(|error| {
                    AppError::internal(format!("failed to decode started_item_ids: {error}"))
                })?;
            let handled_item_ids =
                serde_json::from_value::<Vec<String>>(row.try_get("handled_item_ids").map_err(
                    |error| AppError::internal(format!("failed to read handled_item_ids: {error}")),
                )?)
                .map_err(|error| {
                    AppError::internal(format!("failed to decode handled_item_ids: {error}"))
                })?;
            let completed_item_ids = serde_json::from_value::<Vec<String>>(
                row.try_get("completed_item_ids").map_err(|error| {
                    AppError::internal(format!("failed to read completed_item_ids: {error}"))
                })?,
            )
            .map_err(|error| {
                AppError::internal(format!("failed to decode completed_item_ids: {error}"))
            })?;
            Ok(BotSpeechState {
                initialized,
                active_item,
                pending_items,
                handled_item_ids,
                started_item_ids,
                completed_item_ids,
            })
        })
        .transpose()
    }

    async fn save_state(&self, session_id: i64, state: &BotSpeechState) -> AppResult<()> {
        let active_item = state
            .active_item
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| {
                AppError::internal(format!("failed to encode active_item: {error}"))
            })?;
        let pending_items = serde_json::to_value(&state.pending_items).map_err(|error| {
            AppError::internal(format!("failed to encode pending_items: {error}"))
        })?;
        let started_item_ids = serde_json::to_value(&state.started_item_ids).map_err(|error| {
            AppError::internal(format!("failed to encode started_item_ids: {error}"))
        })?;
        let handled_item_ids = serde_json::to_value(&state.handled_item_ids).map_err(|error| {
            AppError::internal(format!("failed to encode handled_item_ids: {error}"))
        })?;
        let completed_item_ids =
            serde_json::to_value(&state.completed_item_ids).map_err(|error| {
                AppError::internal(format!("failed to encode completed_item_ids: {error}"))
            })?;

        sqlx::query(
            r#"
            insert into call_bot_speech_state (
              session_id,
              initialized,
              active_item,
              pending_items,
              handled_item_ids,
              started_item_ids,
              completed_item_ids,
              updated_at
            )
            values ($1, $2, $3, $4, $5, $6, $7, now())
            on conflict (session_id) do update set
              initialized = excluded.initialized,
              active_item = excluded.active_item,
              pending_items = excluded.pending_items,
              handled_item_ids = excluded.handled_item_ids,
              started_item_ids = excluded.started_item_ids,
              completed_item_ids = excluded.completed_item_ids,
              updated_at = now()
            "#,
        )
        .bind(session_id)
        .bind(state.initialized)
        .bind(active_item)
        .bind(pending_items)
        .bind(handled_item_ids)
        .bind(started_item_ids)
        .bind(completed_item_ids)
        .execute(&self.pool)
        .await
        .map_err(|error| AppError::internal(format!("failed to save bot speech state: {error}")))?;
        Ok(())
    }
}
