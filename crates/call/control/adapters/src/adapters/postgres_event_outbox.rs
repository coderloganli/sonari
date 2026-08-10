use async_trait::async_trait;
use call::CallEvent;
use call::ports::EventSinkPort;
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Row};
use tokio::{sync::watch, task::JoinHandle};

const CLAIM_BATCH_SIZE: i64 = 64;
const CLAIM_TTL_SECONDS: i64 = 30;
const POLL_INTERVAL_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct OutboxEventRecord {
    pub id: i64,
    pub event: CallEvent,
}

#[async_trait]
pub trait CallEventOutboxRepository: Send + Sync {
    async fn claim_batch(&self, claim_owner: &str, limit: i64)
    -> AppResult<Vec<OutboxEventRecord>>;
    async fn mark_dispatched(&self, claim_owner: &str, ids: &[i64]) -> AppResult<()>;
    async fn release_claims(&self, claim_owner: &str, ids: &[i64]) -> AppResult<()>;
}

#[derive(Clone)]
pub struct PostgresCallEventOutboxRepository {
    pool: PgPool,
}

impl PostgresCallEventOutboxRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CallEventOutboxRepository for PostgresCallEventOutboxRepository {
    async fn claim_batch(
        &self,
        claim_owner: &str,
        limit: i64,
    ) -> AppResult<Vec<OutboxEventRecord>> {
        let rows = sqlx::query(
            r#"
            with candidate as (
              select id
              from call_event_outbox
              where dispatched_at is null
                and (
                  claimed_at is null
                  or claimed_at < now() - make_interval(secs => $2)
                )
              order by id asc
              limit $3
              for update skip locked
            )
            update call_event_outbox as outbox
            set claimed_at = now(),
                claim_owner = $1,
                dispatch_attempts = dispatch_attempts + 1
            from candidate
            where outbox.id = candidate.id
            returning
              outbox.id,
              outbox.session_id,
              outbox.round_id,
              outbox.source,
              outbox.event,
              outbox.ts_ms,
              outbox.fields
            "#,
        )
        .bind(claim_owner)
        .bind(CLAIM_TTL_SECONDS)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| {
            AppError::internal(format!("failed to claim call event outbox batch: {error}"))
        })?;

        rows.into_iter()
            .map(|row| {
                Ok(OutboxEventRecord {
                    id: row.get("id"),
                    event: CallEvent {
                        session_id: row.get("session_id"),
                        round_id: row.get("round_id"),
                        source: row.get("source"),
                        event: row.get("event"),
                        ts_ms: row.get("ts_ms"),
                        fields: row.get("fields"),
                    },
                })
            })
            .collect()
    }

    async fn mark_dispatched(&self, claim_owner: &str, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        sqlx::query(
            r#"
            update call_event_outbox
            set dispatched_at = now(),
                claimed_at = null,
                claim_owner = null
            where id = any($1)
              and claim_owner = $2
            "#,
        )
        .bind(ids)
        .bind(claim_owner)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            AppError::internal(format!("failed to mark call events dispatched: {error}"))
        })?;
        Ok(())
    }

    async fn release_claims(&self, claim_owner: &str, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        sqlx::query(
            r#"
            update call_event_outbox
            set claimed_at = null,
                claim_owner = null
            where id = any($1)
              and claim_owner = $2
            "#,
        )
        .bind(ids)
        .bind(claim_owner)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            AppError::internal(format!("failed to release claimed call events: {error}"))
        })?;
        Ok(())
    }
}

pub struct CallEventOutboxPublisher<R, S> {
    repo: R,
    sink: S,
}

impl<R, S> CallEventOutboxPublisher<R, S> {
    pub fn new(repo: R, sink: S) -> Self {
        Self { repo, sink }
    }
}

impl<R, S> CallEventOutboxPublisher<R, S>
where
    R: CallEventOutboxRepository + Send + Sync + 'static,
    S: EventSinkPort + Send + Sync + 'static,
{
    pub async fn start(self) -> AppResult<CallEventOutboxPublisherHandle> {
        let claim_owner = format!("publisher-{:016x}", rand::random::<u64>());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_outbox_publisher(
            self.repo,
            self.sink,
            claim_owner,
            shutdown_rx,
        ));
        Ok(CallEventOutboxPublisherHandle { shutdown_tx, task })
    }
}

pub struct CallEventOutboxPublisherHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<AppResult<()>>,
}

impl CallEventOutboxPublisherHandle {
    pub async fn stop(self) -> AppResult<()> {
        let _ = self.shutdown_tx.send(true);
        self.task.await.map_err(|error| {
            AppError::internal(format!("call event publisher task join failed: {error}"))
        })?
    }
}

async fn run_outbox_publisher<R>(
    repo: R,
    sink: impl EventSinkPort,
    claim_owner: String,
    mut shutdown_rx: watch::Receiver<bool>,
) -> AppResult<()>
where
    R: CallEventOutboxRepository + Send + Sync,
{
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                match changed {
                    Ok(()) if *shutdown_rx.borrow() => return Ok(()),
                    Ok(()) => {}
                    Err(_) => return Ok(()),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)) => {
                let batch = repo.claim_batch(&claim_owner, CLAIM_BATCH_SIZE).await?;
                if batch.is_empty() {
                    continue;
                }

                let mut published_ids = Vec::new();
                let mut unpublished_ids = Vec::new();
                for record in batch {
                    match sink.publish(record.event).await {
                        Ok(()) => published_ids.push(record.id),
                        Err(error) => {
                            tracing::warn!(
                                outbox_id = record.id,
                                error = %error,
                                "failed to publish call event outbox record to redis stream"
                            );
                            unpublished_ids.push(record.id);
                        }
                    }
                }

                if !published_ids.is_empty() {
                    repo.mark_dispatched(&claim_owner, &published_ids).await?;
                }
                if !unpublished_ids.is_empty() {
                    repo.release_claims(&claim_owner, &unpublished_ids).await?;
                }
            }
        }
    }
}
