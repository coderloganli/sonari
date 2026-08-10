use async_trait::async_trait;
use call::{
    CallCallerIdentity, CallEvent, CallSession, CallStatus, CallType, RuntimeWorkStatus,
    SpeechInputLanguage,
    ports::{
        CallSessionRepository, CallTransactionContext, CallTransactionRunner, CallTransactionStore,
        RuntimeSessionState, RuntimeSessionStatePort,
    },
};
use call_log_contract::CallPromptLogContextPort;
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Row, Transaction};

#[derive(Debug, Clone)]
pub struct PostgresCallSessionRepository {
    pool: PgPool,
}

impl PostgresCallSessionRepository {
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

fn parse_speech_input_language(raw: &str) -> AppResult<SpeechInputLanguage> {
    SpeechInputLanguage::parse(raw)
        .ok_or_else(|| AppError::internal("invalid speech input language value"))
}

fn parse_call_type(raw: &str) -> AppResult<CallType> {
    CallType::parse(raw).ok_or_else(|| AppError::internal("invalid call type value"))
}

fn parse_call_status(raw: &str) -> AppResult<CallStatus> {
    CallStatus::parse(raw).ok_or_else(|| AppError::internal("invalid call status value"))
}

fn parse_runtime_work_status(raw: &str) -> AppResult<RuntimeWorkStatus> {
    RuntimeWorkStatus::parse(raw)
        .ok_or_else(|| AppError::internal("invalid runtime work status value"))
}

fn map_call_session(row: sqlx::postgres::PgRow) -> AppResult<CallSession> {
    let caller = match row.get::<&str, _>("caller_kind") {
        "platform_user" => CallCallerIdentity::PlatformUser {
            user_id: row
                .get::<Option<i64>, _>("user_id")
                .ok_or_else(|| AppError::internal("platform call session is missing user_id"))?,
        },
        _ => return Err(AppError::internal("invalid call caller kind")),
    };
    Ok(CallSession {
        id: row.get("id"),
        caller,
        realtime_participant_identity: row.get("realtime_participant_identity"),
        character_id: row.get("character_id"),
        character_name: row.get("character_name"),
        scene_id: row.get("scene_id"),
        scene_name: row.get("scene_name"),
        voice: row.get("voice"),
        agent_session_id: row.get("agent_session_id"),
        asr_language: parse_speech_input_language(row.get::<&str, _>("asr_language"))?,
        call_type: parse_call_type(row.get::<&str, _>("call_type"))?,
        status: parse_call_status(row.get::<&str, _>("status"))?,
        owner_instance: row.get("owner_instance"),
        runtime_owner_id: row.get("runtime_owner_id"),
        runtime_status: parse_runtime_work_status(row.get::<&str, _>("runtime_status"))?,
        runtime_failure_reason: row.get("runtime_failure_reason"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        duration_seconds: row.get("duration_seconds"),
    })
}

async fn insert_call_event(tx: &mut Transaction<'_, Postgres>, event: CallEvent) -> AppResult<()> {
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
    .execute(tx.as_mut())
    .await
    .map_err(map_sqlx_error)?;
    Ok(())
}

pub struct PostgresCallTransactionStore<'tx, 'db> {
    tx: &'tx mut Transaction<'db, Postgres>,
}

const CALL_SESSION_COLUMNS: &str = r#"
  id,
  caller_kind,
  user_id,
  realtime_participant_identity,
  character_id,
  character_name,
  scene_id,
  scene_name,
  voice,
  agent_session_id,
  asr_language,
  call_type,
  status,
  owner_instance,
  runtime_owner_id,
  runtime_status,
  runtime_failure_reason,
  started_at,
  ended_at,
  duration_seconds
"#;

#[async_trait]
impl CallSessionRepository for PostgresCallSessionRepository {
    async fn create(&self, session: &CallSession) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            insert into call_sessions (
              caller_kind,
              user_id,
              realtime_participant_identity,
              character_id,
              character_name,
              scene_id,
              scene_name,
              voice,
              agent_session_id,
              asr_language,
              call_type,
              status,
              owner_instance,
              runtime_owner_id,
              runtime_status,
              runtime_failure_reason,
              started_at,
              ended_at,
              duration_seconds
            )
            values (
              $1, $2, $3, $4, $5, $6, $7, $8,
              $9, $10, $11, $12, $13, $14, $15, $16,
              $17, $18, $19
            )
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session.caller.kind())
        .bind(session.caller.platform_user_id())
        .bind(&session.realtime_participant_identity)
        .bind(session.character_id)
        .bind(&session.character_name)
        .bind(session.scene_id)
        .bind(&session.scene_name)
        .bind(session.voice.clone())
        .bind(&session.agent_session_id)
        .bind(session.asr_language.as_str())
        .bind(session.call_type.as_str())
        .bind(session.status.as_str())
        .bind(&session.owner_instance)
        .bind(&session.runtime_owner_id)
        .bind(session.runtime_status.as_str())
        .bind(&session.runtime_failure_reason)
        .bind(session.started_at)
        .bind(session.ended_at)
        .bind(session.duration_seconds)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn get_by_id(&self, session_id: i64) -> AppResult<Option<CallSession>> {
        let row = sqlx::query(&format!(
            r#"
            select {CALL_SESSION_COLUMNS}
            from call_sessions
            where id = $1
            "#,
        ))
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(map_call_session).transpose()
    }

    async fn list_for_user(&self, user_id: i64) -> AppResult<Vec<CallSession>> {
        let rows = sqlx::query(&format!(
            r#"
            select {CALL_SESSION_COLUMNS}
            from call_sessions
            where caller_kind = 'platform_user'
              and user_id = $1
            order by started_at desc
            "#,
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter().map(map_call_session).collect()
    }

    async fn claim_runtime_start_work(
        &self,
        runtime_owner_id: &str,
    ) -> AppResult<Option<CallSession>> {
        let row = sqlx::query(&format!(
            r#"
            with candidate as (
              select id
              from call_sessions
              where runtime_owner_id = $1
                and status = $2
                and runtime_status = $3
              order by started_at asc
              limit 1
              for update skip locked
            )
            update call_sessions
            set runtime_status = $4,
                runtime_failure_reason = null
            where id in (select id from candidate)
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(runtime_owner_id)
        .bind(CallStatus::Starting.as_str())
        .bind(RuntimeWorkStatus::PendingStart.as_str())
        .bind(RuntimeWorkStatus::StartClaimed.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(map_call_session).transpose()
    }

    async fn claim_runtime_stop_work(
        &self,
        runtime_owner_id: &str,
    ) -> AppResult<Option<CallSession>> {
        let row = sqlx::query(&format!(
            r#"
            with candidate as (
              select id
              from call_sessions
              where runtime_owner_id = $1
                and status = $2
                and runtime_status = $3
              order by started_at asc
              limit 1
              for update skip locked
            )
            update call_sessions
            set runtime_status = $4,
                runtime_failure_reason = null
            where id in (select id from candidate)
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(runtime_owner_id)
        .bind(CallStatus::Ending.as_str())
        .bind(RuntimeWorkStatus::StopRequested.as_str())
        .bind(RuntimeWorkStatus::StopClaimed.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(map_call_session).transpose()
    }

    async fn mark_status(&self, session_id: i64, status: CallStatus) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_runtime_state(
        &self,
        session_id: i64,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set runtime_status = $2,
                runtime_failure_reason = $3
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_status_and_runtime(
        &self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2,
                runtime_status = $3,
                runtime_failure_reason = $4
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_ended(
        &self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
        ended_at: chrono::DateTime<chrono::Utc>,
        duration_seconds: i32,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2,
                runtime_status = $3,
                runtime_failure_reason = $4,
                ended_at = $5,
                duration_seconds = $6
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .bind(ended_at)
        .bind(duration_seconds)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }
}

#[async_trait]
impl CallTransactionStore for PostgresCallTransactionStore<'_, '_> {
    async fn create(&mut self, session: &CallSession) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            insert into call_sessions (
              caller_kind,
              user_id,
              realtime_participant_identity,
              character_id,
              character_name,
              scene_id,
              scene_name,
              voice,
              agent_session_id,
              asr_language,
              call_type,
              status,
              owner_instance,
              runtime_owner_id,
              runtime_status,
              runtime_failure_reason,
              started_at,
              ended_at,
              duration_seconds
            )
            values (
              $1, $2, $3, $4, $5, $6, $7, $8,
              $9, $10, $11, $12, $13, $14, $15, $16,
              $17, $18, $19
            )
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session.caller.kind())
        .bind(session.caller.platform_user_id())
        .bind(&session.realtime_participant_identity)
        .bind(session.character_id)
        .bind(&session.character_name)
        .bind(session.scene_id)
        .bind(&session.scene_name)
        .bind(session.voice.clone())
        .bind(&session.agent_session_id)
        .bind(session.asr_language.as_str())
        .bind(session.call_type.as_str())
        .bind(session.status.as_str())
        .bind(&session.owner_instance)
        .bind(&session.runtime_owner_id)
        .bind(session.runtime_status.as_str())
        .bind(&session.runtime_failure_reason)
        .bind(session.started_at)
        .bind(session.ended_at)
        .bind(session.duration_seconds)
        .fetch_one(self.tx.as_mut())
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_status(&mut self, session_id: i64, status: CallStatus) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .fetch_one(self.tx.as_mut())
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_runtime_state(
        &mut self,
        session_id: i64,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set runtime_status = $2,
                runtime_failure_reason = $3
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .fetch_one(self.tx.as_mut())
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_status_and_runtime(
        &mut self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2,
                runtime_status = $3,
                runtime_failure_reason = $4
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .fetch_one(self.tx.as_mut())
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn mark_ended(
        &mut self,
        session_id: i64,
        status: CallStatus,
        runtime_status: RuntimeWorkStatus,
        runtime_failure_reason: Option<String>,
        ended_at: chrono::DateTime<chrono::Utc>,
        duration_seconds: i32,
    ) -> AppResult<CallSession> {
        let row = sqlx::query(&format!(
            r#"
            update call_sessions
            set status = $2,
                runtime_status = $3,
                runtime_failure_reason = $4,
                ended_at = $5,
                duration_seconds = $6
            where id = $1
            returning {CALL_SESSION_COLUMNS}
            "#,
        ))
        .bind(session_id)
        .bind(status.as_str())
        .bind(runtime_status.as_str())
        .bind(runtime_failure_reason)
        .bind(ended_at)
        .bind(duration_seconds)
        .fetch_one(self.tx.as_mut())
        .await
        .map_err(map_sqlx_error)?;

        map_call_session(row)
    }

    async fn insert(&mut self, event: CallEvent) -> AppResult<()> {
        insert_call_event(self.tx, event).await
    }
}

impl CallTransactionRunner for PostgresCallSessionRepository {
    fn run<'a, T, F>(&'a self, op: F) -> call::ports::CallTransactionFuture<'a, T>
    where
        T: Send + 'a,
        F: for<'tx> FnOnce(
                CallTransactionContext<'tx>,
            ) -> call::ports::CallTransactionFuture<'tx, T>
            + Send
            + 'a,
    {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
            let result = {
                let mut store = PostgresCallTransactionStore { tx: &mut tx };
                op(CallTransactionContext { store: &mut store }).await?
            };
            tx.commit().await.map_err(map_sqlx_error)?;
            Ok(result)
        })
    }
}

#[async_trait]
impl RuntimeSessionStatePort for PostgresCallSessionRepository {
    async fn get_runtime_session_state(
        &self,
        session_id: i64,
    ) -> AppResult<Option<RuntimeSessionState>> {
        let row = sqlx::query(
            r#"
            select
              id,
              agent_session_id,
              voice,
              asr_language,
              runtime_owner_id,
              status,
              runtime_status
            from call_sessions
            where id = $1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(|row| {
            Ok(RuntimeSessionState {
                session_id: row.get("id"),
                agent_session_id: row.get("agent_session_id"),
                voice: row.get("voice"),
                asr_language: parse_speech_input_language(row.get::<&str, _>("asr_language"))?,
                runtime_owner_id: row.get("runtime_owner_id"),
                status: parse_call_status(row.get::<&str, _>("status"))?,
                runtime_status: parse_runtime_work_status(row.get::<&str, _>("runtime_status"))?,
            })
        })
        .transpose()
    }
}

#[async_trait]
impl CallPromptLogContextPort for PostgresCallSessionRepository {
    async fn resolve_call_session_id(&self, agent_session_id: &str) -> AppResult<i64> {
        let row = sqlx::query(
            r#"
            select id
            from call_sessions
            where agent_session_id = $1
            "#,
        )
        .bind(agent_session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(|row| row.get("id"))
            .ok_or_else(|| AppError::not_found("call session not found for agent session"))
    }
}
