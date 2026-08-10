use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::{
    AppErrorGroupListItem, AppErrorListQuery, AppErrorOccurrence, AppErrorOccurrenceListItem,
    AppErrorOccurrenceListQuery, AppErrorQueryPort, AppErrorSinkPort, AppErrorStatBucket,
    AppErrorStatsQuery, AppErrorStatsView, HttpRequestContext, ObservabilityError,
    ObservabilityResult,
};

#[derive(Debug, Clone)]
pub struct PostgresAppErrorRepository {
    pool: PgPool,
}

impl PostgresAppErrorRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_sqlx_error(err: sqlx::Error) -> ObservabilityError {
    ObservabilityError::internal(format!("postgres error: {err}"))
}

#[async_trait]
impl AppErrorSinkPort for PostgresAppErrorRepository {
    async fn append_app_errors(
        &self,
        _request_context: &HttpRequestContext,
        occurrences: &[AppErrorOccurrence],
    ) -> ObservabilityResult<()> {
        for occurrence in occurrences {
            let group_id: i64 = sqlx::query_scalar(
                r#"
                insert into app_error_groups (
                  fingerprint,
                  category,
                  latest_level,
                  sample_message,
                  sample_stack_trace,
                  latest_request_id,
                  latest_trace_id,
                  latest_session_id,
                  latest_device_name,
                  latest_app_version,
                  latest_os_version,
                  first_seen_at,
                  last_seen_at,
                  occurrence_count
                )
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now(), now(), 1)
                on conflict (fingerprint)
                do update set
                  category = excluded.category,
                  latest_level = excluded.latest_level,
                  sample_message = excluded.sample_message,
                  sample_stack_trace = excluded.sample_stack_trace,
                  latest_request_id = excluded.latest_request_id,
                  latest_trace_id = excluded.latest_trace_id,
                  latest_session_id = excluded.latest_session_id,
                  latest_device_name = excluded.latest_device_name,
                  latest_app_version = excluded.latest_app_version,
                  latest_os_version = excluded.latest_os_version,
                  last_seen_at = now(),
                  occurrence_count = app_error_groups.occurrence_count + 1
                returning id
                "#,
            )
            .bind(&occurrence.fingerprint)
            .bind(&occurrence.report.category)
            .bind(&occurrence.report.level)
            .bind(&occurrence.report.message)
            .bind(&occurrence.report.stack_trace)
            .bind(&occurrence.request_id)
            .bind(&occurrence.trace_id)
            .bind(&occurrence.report.session_id)
            .bind(&occurrence.report.device_name)
            .bind(&occurrence.report.app_version)
            .bind(&occurrence.report.os_version)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

            sqlx::query(
                r#"
                insert into app_error_occurrences (
                  group_id,
                  request_id,
                  trace_id,
                  fingerprint,
                  level,
                  category,
                  message,
                  stack_trace,
                  session_id,
                  device_name,
                  app_version,
                  os_version,
                  fields,
                  client_ts_ms
                )
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                "#,
            )
            .bind(group_id)
            .bind(&occurrence.request_id)
            .bind(&occurrence.trace_id)
            .bind(&occurrence.fingerprint)
            .bind(&occurrence.report.level)
            .bind(&occurrence.report.category)
            .bind(&occurrence.report.message)
            .bind(&occurrence.report.stack_trace)
            .bind(&occurrence.report.session_id)
            .bind(&occurrence.report.device_name)
            .bind(&occurrence.report.app_version)
            .bind(&occurrence.report.os_version)
            .bind(&occurrence.report.fields)
            .bind(occurrence.report.ts_ms)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        }
        Ok(())
    }
}

#[async_trait]
impl AppErrorQueryPort for PostgresAppErrorRepository {
    async fn list_app_errors(
        &self,
        query: &AppErrorListQuery,
    ) -> ObservabilityResult<(Vec<AppErrorGroupListItem>, i64)> {
        let offset = (query.page - 1) * query.page_size;
        let rows = sqlx::query(
            r#"
            select
              id,
              fingerprint,
              latest_level,
              category,
              sample_message,
              sample_stack_trace,
              latest_request_id,
              latest_trace_id,
              latest_session_id,
              latest_device_name,
              latest_app_version,
              latest_os_version,
              first_seen_at,
              last_seen_at,
              occurrence_count
            from app_error_groups
            where ($1::text is null or latest_level = $1)
              and ($2::text is null or category = $2)
              and ($3::timestamptz is null or last_seen_at >= $3)
              and ($4::timestamptz is null or last_seen_at <= $4)
            order by last_seen_at desc, id desc
            limit $5 offset $6
            "#,
        )
        .bind(&query.level)
        .bind(&query.category)
        .bind(query.date_from)
        .bind(query.date_to)
        .bind(query.page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let total_row = sqlx::query(
            r#"
            select count(*) as count
            from app_error_groups
            where ($1::text is null or latest_level = $1)
              and ($2::text is null or category = $2)
              and ($3::timestamptz is null or last_seen_at >= $3)
              and ($4::timestamptz is null or last_seen_at <= $4)
            "#,
        )
        .bind(&query.level)
        .bind(&query.category)
        .bind(query.date_from)
        .bind(query.date_to)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let items = rows
            .into_iter()
            .map(|row| AppErrorGroupListItem {
                id: row.get("id"),
                fingerprint: row.get("fingerprint"),
                latest_level: row.get("latest_level"),
                category: row.get("category"),
                sample_message: row.get("sample_message"),
                sample_stack_trace: row.get("sample_stack_trace"),
                latest_request_id: row.get("latest_request_id"),
                latest_trace_id: row.get("latest_trace_id"),
                latest_session_id: row.get("latest_session_id"),
                latest_device_name: row.get("latest_device_name"),
                latest_app_version: row.get("latest_app_version"),
                latest_os_version: row.get("latest_os_version"),
                first_seen_at: row.get("first_seen_at"),
                last_seen_at: row.get("last_seen_at"),
                occurrence_count: row.get("occurrence_count"),
            })
            .collect();

        Ok((items, total_row.get("count")))
    }

    async fn list_app_error_occurrences(
        &self,
        query: &AppErrorOccurrenceListQuery,
    ) -> ObservabilityResult<(Vec<AppErrorOccurrenceListItem>, i64)> {
        let offset = (query.page - 1) * query.page_size;
        let rows = sqlx::query(
            r#"
            select
              id,
              group_id,
              request_id,
              trace_id,
              fingerprint,
              level,
              category,
              message,
              stack_trace,
              session_id,
              device_name,
              app_version,
              os_version,
              fields,
              client_ts_ms,
              created_at
            from app_error_occurrences
            where group_id = $1
            order by created_at desc, id desc
            limit $2 offset $3
            "#,
        )
        .bind(query.group_id)
        .bind(query.page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let total_row = sqlx::query(
            r#"
            select count(*) as count
            from app_error_occurrences
            where group_id = $1
            "#,
        )
        .bind(query.group_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let items = rows
            .into_iter()
            .map(|row| AppErrorOccurrenceListItem {
                id: row.get("id"),
                group_id: row.get("group_id"),
                request_id: row.get("request_id"),
                trace_id: row.get("trace_id"),
                fingerprint: row.get("fingerprint"),
                level: row.get("level"),
                category: row.get("category"),
                message: row.get("message"),
                stack_trace: row.get("stack_trace"),
                session_id: row.get("session_id"),
                device_name: row.get("device_name"),
                app_version: row.get("app_version"),
                os_version: row.get("os_version"),
                fields: row.get("fields"),
                client_ts_ms: row.get("client_ts_ms"),
                created_at: row.get("created_at"),
            })
            .collect();

        Ok((items, total_row.get("count")))
    }

    async fn get_app_error_stats(
        &self,
        query: &AppErrorStatsQuery,
    ) -> ObservabilityResult<AppErrorStatsView> {
        let totals = sqlx::query(
            r#"
            select
              count(*) as total_occurrences,
              count(distinct group_id) as total_groups
            from app_error_occurrences
            where ($1::timestamptz is null or created_at >= $1)
              and ($2::timestamptz is null or created_at <= $2)
            "#,
        )
        .bind(query.date_from)
        .bind(query.date_to)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let by_category = query_stat_buckets(
            &self.pool,
            r#"
            select category as key, count(*) as count
            from app_error_occurrences
            where ($1::timestamptz is null or created_at >= $1)
              and ($2::timestamptz is null or created_at <= $2)
            group by category
            order by count desc, key asc
            limit 10
            "#,
            query,
        )
        .await?;

        let by_app_version = query_stat_buckets(
            &self.pool,
            r#"
            select app_version as key, count(*) as count
            from app_error_occurrences
            where ($1::timestamptz is null or created_at >= $1)
              and ($2::timestamptz is null or created_at <= $2)
            group by app_version
            order by count desc, key asc
            limit 10
            "#,
            query,
        )
        .await?;

        let by_device_name = query_stat_buckets(
            &self.pool,
            r#"
            select coalesce(device_name, 'unknown') as key, count(*) as count
            from app_error_occurrences
            where ($1::timestamptz is null or created_at >= $1)
              and ($2::timestamptz is null or created_at <= $2)
            group by coalesce(device_name, 'unknown')
            order by count desc, key asc
            limit 10
            "#,
            query,
        )
        .await?;

        Ok(AppErrorStatsView {
            total_occurrences: totals.get("total_occurrences"),
            total_groups: totals.get("total_groups"),
            by_category,
            by_app_version,
            by_device_name,
        })
    }
}

async fn query_stat_buckets(
    pool: &PgPool,
    sql: &str,
    query: &AppErrorStatsQuery,
) -> ObservabilityResult<Vec<AppErrorStatBucket>> {
    let rows = sqlx::query(sql)
        .bind(query.date_from)
        .bind(query.date_to)
        .fetch_all(pool)
        .await
        .map_err(map_sqlx_error)?;

    Ok(rows
        .into_iter()
        .map(|row| AppErrorStatBucket {
            key: row.get("key"),
            count: row.get("count"),
        })
        .collect())
}
