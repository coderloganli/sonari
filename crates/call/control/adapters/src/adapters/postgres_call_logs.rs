use async_trait::async_trait;
use call::{
    CallCallerIdentity, CallConversationMessageFact, CallEvent, CallEventLogRepository,
    CallLogFact, CallLogFactsQuery, CallPromptLogFact, CallPromptLogMessageFact, CallTimelineEvent,
};
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
pub struct PostgresCallEventLogRepository {
    pool: PgPool,
}

impl PostgresCallEventLogRepository {
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

fn map_caller(row: &sqlx::postgres::PgRow) -> AppResult<CallCallerIdentity> {
    match row.get::<&str, _>("caller_kind") {
        "platform_user" => Ok(CallCallerIdentity::PlatformUser {
            user_id: row
                .get::<Option<i64>, _>("user_id")
                .ok_or_else(|| AppError::internal("platform call log is missing user_id"))?,
        }),
        _ => Err(AppError::internal("invalid call log caller kind")),
    }
}

#[async_trait]
impl CallEventLogRepository for PostgresCallEventLogRepository {
    async fn append_batch(&self, events: &[CallEvent]) -> AppResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut query = String::from(
            "insert into call_events (session_id, round_id, source, event, ts_ms, fields) values ",
        );
        for index in 0..events.len() {
            if index > 0 {
                query.push(',');
            }
            let base = index * 6;
            query.push_str(&format!(
                "(${}, ${}, ${}, ${}, ${}, ${})",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
                base + 6
            ));
        }
        let mut built = sqlx::query(&query);
        for event in events {
            built = built
                .bind(event.session_id)
                .bind(&event.round_id)
                .bind(&event.source)
                .bind(&event.event)
                .bind(event.ts_ms)
                .bind(&event.fields);
        }
        built.execute(&self.pool).await.map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn list_timeline(&self, session_id: i64) -> AppResult<Vec<CallTimelineEvent>> {
        let rows = sqlx::query(
            r#"
            select id, session_id, round_id, source, event, ts_ms, fields, created_at
            from call_events
            where session_id = $1
            order by ts_ms asc, id asc
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(rows
            .into_iter()
            .map(|row| CallTimelineEvent {
                id: row.get("id"),
                session_id: row.get("session_id"),
                round_id: row.get("round_id"),
                source: row.get("source"),
                event: row.get("event"),
                ts_ms: row.get("ts_ms"),
                fields: row.get("fields"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    async fn list_prompt_logs(&self, session_id: i64) -> AppResult<Vec<CallPromptLogFact>> {
        let rows = sqlx::query(
            r#"
            select id, session_id, round_id, source, ts_ms, fields, created_at
            from call_events
            where session_id = $1
              and event = 'agent_prompt_logged'
            order by ts_ms asc, id asc
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|row| {
                let fields: serde_json::Value = row.get("fields");
                let model_name = fields
                    .get("model_name")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        AppError::internal("call prompt log event is missing model_name")
                    })?
                    .to_owned();
                let response_text = fields
                    .get("response_text")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        AppError::internal("call prompt log event is missing response_text")
                    })?
                    .to_owned();
                let request_messages = serde_json::from_value::<Vec<CallPromptLogMessageFact>>(
                    fields.get("request_messages").cloned().ok_or_else(|| {
                        AppError::internal("call prompt log event is missing request_messages")
                    })?,
                )
                .map_err(|error| {
                    AppError::internal(format!(
                        "call prompt log event has invalid request_messages: {error}"
                    ))
                })?;

                Ok(CallPromptLogFact {
                    id: row.get("id"),
                    session_id: row.get("session_id"),
                    round_id: row.get("round_id"),
                    source: row.get("source"),
                    model_name,
                    request_messages,
                    response_text,
                    created_at: row.get("created_at"),
                })
            })
            .collect()
    }

    async fn list_call_conversation(
        &self,
        session_id: i64,
    ) -> AppResult<Vec<CallConversationMessageFact>> {
        // 原始对话来自 llm_messages,经 call_sessions.agent_session_id == llm_sessions.id 关联。
        // 按轮次有序(同轮多条按 id 兜底),只读,不依赖死掉的 agent_prompt_logged 管道。
        let rows = sqlx::query(
            r#"
            select m.id, m.turn_number, m.role, m.content, m.created_at
            from llm_messages m
            join call_sessions cs on cs.agent_session_id = m.session_id
            where cs.id = $1
            order by m.turn_number asc, m.id asc
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(rows
            .into_iter()
            .map(|row| CallConversationMessageFact {
                id: row.get("id"),
                turn_number: row.get("turn_number"),
                role: row.get("role"),
                content: row.get("content"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    async fn get_started_at_ms(&self, session_id: i64) -> AppResult<Option<i64>> {
        let row = sqlx::query("select started_at from call_sessions where id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(row.map(|row| {
            row.get::<chrono::DateTime<chrono::Utc>, _>("started_at")
                .timestamp_millis()
        }))
    }

    async fn list_call_log_facts(
        &self,
        query: &CallLogFactsQuery,
    ) -> AppResult<(Vec<CallLogFact>, i64)> {
        let rows = sqlx::query(
            r#"
            with filtered as (
              select
                cs.id,
                cs.caller_kind,
                cs.user_id,
                cs.sdk_partner_id,
                cs.sdk_app_id,
                cs.sdk_user_id,
                cs.sdk_session_id,
                cs.runtime_snapshot_id,
                cs.external_user_id_hash,
                cs.character_id,
                cs.character_name,
                cs.scene_id,
                cs.scene_name,
                cs.agent_session_id,
                cs.voice,
                cs.asr_language,
                cs.status,
                cs.runtime_status,
                cs.runtime_failure_reason,
                cs.owner_instance,
                cs.started_at,
                cs.ended_at,
                coalesce(nullif(cs.duration_seconds, 0), case when cs.ended_at is not null then greatest(0, extract(epoch from (cs.ended_at - cs.started_at))::int) else 0 end) as duration_seconds
              from call_sessions cs
              where ($1::bigint is null or cs.character_id = $1)
                and ($2::timestamptz is null or cs.started_at >= $2)
                and ($3::timestamptz is null or cs.started_at <= $3)
            ),
            totals as (
              select count(*)::bigint as total from filtered
            ),
            paged as (
              select * from filtered
              order by started_at desc
              limit $4 offset $5
            ),
            event_counts as (
              select
                ce.session_id,
                count(*)::bigint as event_count,
                count(distinct coalesce(ce.round_id, ce.fields->>'round_id'))
                  filter (
                    where coalesce(ce.round_id, ce.fields->>'round_id') is not null
                      and coalesce(ce.round_id, ce.fields->>'round_id') <> ''
                  )::bigint as round_count
              from call_events ce
              where ce.session_id in (select id from paged)
              group by ce.session_id
            )
            select
              p.id as session_id,
              p.caller_kind,
              p.user_id,
              p.sdk_partner_id,
              p.sdk_app_id,
              p.sdk_user_id,
              p.sdk_session_id,
              p.runtime_snapshot_id,
              p.external_user_id_hash,
              p.character_id,
              p.character_name,
              p.scene_id,
              p.scene_name,
              p.agent_session_id,
              p.voice,
              p.asr_language,
              p.status,
              p.runtime_status,
              p.runtime_failure_reason,
              p.owner_instance,
              p.started_at,
              p.ended_at,
              p.duration_seconds,
              coalesce(ec.event_count, 0) as event_count,
              coalesce(ec.round_count, 0) as round_count,
              t.total
            from paged p
            cross join totals t
            left join event_counts ec on ec.session_id = p.id
            order by p.started_at desc
            "#,
        )
        .bind(query.character_id)
        .bind(query.date_from)
        .bind(query.date_to)
        .bind(query.page_size)
        .bind((query.page - 1) * query.page_size)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let total = rows
            .first()
            .map(|row| row.get::<i64, _>("total"))
            .unwrap_or_default();
        let items = rows
            .into_iter()
            .map(|row| {
                Ok(CallLogFact {
                    session_id: row.get("session_id"),
                    caller: map_caller(&row)?,
                    character_id: row.get("character_id"),
                    character_name: row.get("character_name"),
                    scene_id: row.get("scene_id"),
                    scene_name: row.get("scene_name"),
                    agent_session_id: row.get("agent_session_id"),
                    voice: row.get("voice"),
                    asr_language: row.get("asr_language"),
                    status: row.get("status"),
                    runtime_status: row.get("runtime_status"),
                    runtime_failure_reason: row.get("runtime_failure_reason"),
                    owner_instance: row.get("owner_instance"),
                    started_at: row.get("started_at"),
                    ended_at: row.get("ended_at"),
                    duration_seconds: row.get("duration_seconds"),
                    event_count: row.get("event_count"),
                    round_count: row.get("round_count"),
                })
            })
            .collect::<AppResult<Vec<_>>>()?;

        Ok((items, total))
    }

    async fn get_call_log_fact(&self, session_id: i64) -> AppResult<Option<CallLogFact>> {
        let row = sqlx::query(
            r#"
            with event_counts as (
              select
                session_id,
                count(*)::bigint as event_count,
                count(distinct coalesce(round_id, fields->>'round_id'))
                  filter (
                    where coalesce(round_id, fields->>'round_id') is not null
                      and coalesce(round_id, fields->>'round_id') <> ''
                  )::bigint as round_count
              from call_events
              where session_id = $1
              group by session_id
            )
            select
              cs.id as session_id,
              cs.caller_kind,
              cs.user_id,
              cs.sdk_partner_id,
              cs.sdk_app_id,
              cs.sdk_user_id,
              cs.sdk_session_id,
              cs.runtime_snapshot_id,
              cs.external_user_id_hash,
              cs.character_id,
              cs.character_name,
              cs.scene_id,
              cs.scene_name,
              cs.agent_session_id,
              cs.voice,
              cs.asr_language,
              cs.status,
              cs.runtime_status,
              cs.runtime_failure_reason,
              cs.owner_instance,
              cs.started_at,
              cs.ended_at,
              coalesce(nullif(cs.duration_seconds, 0), case when cs.ended_at is not null then greatest(0, extract(epoch from (cs.ended_at - cs.started_at))::int) else 0 end) as duration_seconds,
              coalesce(ec.event_count, 0) as event_count,
              coalesce(ec.round_count, 0) as round_count
            from call_sessions cs
            left join event_counts ec on ec.session_id = cs.id
            where cs.id = $1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        row.map(|row| {
            Ok(CallLogFact {
                session_id: row.get("session_id"),
                caller: map_caller(&row)?,
                character_id: row.get("character_id"),
                character_name: row.get("character_name"),
                scene_id: row.get("scene_id"),
                scene_name: row.get("scene_name"),
                agent_session_id: row.get("agent_session_id"),
                voice: row.get("voice"),
                asr_language: row.get("asr_language"),
                status: row.get("status"),
                runtime_status: row.get("runtime_status"),
                runtime_failure_reason: row.get("runtime_failure_reason"),
                owner_instance: row.get("owner_instance"),
                started_at: row.get("started_at"),
                ended_at: row.get("ended_at"),
                duration_seconds: row.get("duration_seconds"),
                event_count: row.get("event_count"),
                round_count: row.get("round_count"),
            })
        })
        .transpose()
    }
}
