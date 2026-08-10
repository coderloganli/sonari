use crate::{
    AgentArchiveMessage, AgentCallerIdentity, AgentMessage, AgentMessageArchive, AgentSession,
    LlmUsageLog, LlmUsageStats, MessageRole, PartnerConversationPromptOverride, PromptTemplate,
    PromptTemplateKey, ProviderKey, SessionUsageSummary,
    ports::{
        AgentArchiveRepository, AgentMessageRepository, AgentSessionRepository,
        AgentSettingsRepository, PartnerConversationPromptOverrideRepository,
        PromptTemplateRepository, UsageLogRepository,
    },
};
use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
// prompt 模板仓储负责落库 agent 自有模板文本。
pub struct PostgresPromptTemplateRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct PostgresPartnerConversationPromptOverrideRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
// 会话仓储负责落库对话生命周期状态。
pub struct PostgresAgentSessionRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
// 消息仓储负责读写按轮次组织的会话消息。
pub struct PostgresAgentMessageRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
// 归档仓储负责固化已结束会话的快照。
pub struct PostgresAgentArchiveRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
// 用量仓储负责落库 token 和错误统计。
pub struct PostgresAgentUsageLogRepository {
    pool: PgPool,
}

#[derive(Debug, Clone)]
// 设置仓储负责读写 agent 模块自有的轻量运行参数。
pub struct PostgresAgentSettingsRepository {
    pool: PgPool,
}

macro_rules! repo_new {
    ($name:ident) => {
        impl $name {
            pub fn new(pool: PgPool) -> Self {
                Self { pool }
            }
        }
    };
}

repo_new!(PostgresPromptTemplateRepository);
repo_new!(PostgresPartnerConversationPromptOverrideRepository);
repo_new!(PostgresAgentSessionRepository);
repo_new!(PostgresAgentMessageRepository);
repo_new!(PostgresAgentArchiveRepository);
repo_new!(PostgresAgentUsageLogRepository);
repo_new!(PostgresAgentSettingsRepository);

fn map_sqlx_error(err: sqlx::Error) -> AppError {
    // 这里把 sqlx 错误收敛为 shared-kernel 统一错误模型。
    match err {
        sqlx::Error::RowNotFound => AppError::not_found("record not found"),
        other => AppError::internal(format!("postgres error: {other}")),
    }
}

fn parse_provider_key(raw: &str) -> AppResult<ProviderKey> {
    ProviderKey::parse(raw).ok_or_else(|| AppError::internal("invalid provider key"))
}

fn parse_template_key(raw: &str) -> AppResult<PromptTemplateKey> {
    PromptTemplateKey::parse(raw).ok_or_else(|| AppError::internal("invalid template key"))
}

fn parse_message_role(raw: &str) -> AppResult<MessageRole> {
    MessageRole::parse(raw).ok_or_else(|| AppError::internal("invalid message role"))
}

fn map_template(row: sqlx::postgres::PgRow) -> AppResult<PromptTemplate> {
    // 行映射保证模板键在出库时恢复为枚举类型。
    Ok(PromptTemplate {
        id: row.get("id"),
        template_key: parse_template_key(row.get::<&str, _>("template_key"))?,
        template_text: row.get("template_text"),
        updated_at: row.get("updated_at"),
    })
}

fn map_partner_prompt_override(row: sqlx::postgres::PgRow) -> PartnerConversationPromptOverride {
    PartnerConversationPromptOverride {
        partner_id: row.get("partner_id"),
        system_prompt_1: row.get("system_prompt_1"),
        system_prompt_2: row.get("system_prompt_2"),
        system_prompt_3: row.get("system_prompt_3"),
        welcome_user_prompt: row.get("welcome_user_prompt"),
        updated_by: row.get("updated_by"),
        updated_at: row.get("updated_at"),
        created_at: row.get("created_at"),
    }
}

fn required_i64(row: &sqlx::postgres::PgRow, column: &str, identity_kind: &str) -> AppResult<i64> {
    row.get::<Option<i64>, _>(column).ok_or_else(|| {
        AppError::internal(format!("{identity_kind} agent session is missing {column}"))
    })
}

fn map_session(row: sqlx::postgres::PgRow) -> AppResult<AgentSession> {
    // 行映射直接还原 agent 会话实体。
    let caller = match row.get::<&str, _>("caller_kind") {
        "platform_user" => AgentCallerIdentity::PlatformUser {
            user_id: required_i64(&row, "user_id", "platform")?,
        },
        _ => return Err(AppError::internal("invalid agent session caller_kind")),
    };
    Ok(AgentSession {
        id: row.get("id"),
        caller,
        character_id: row.get("character_id"),
        timezone: row.get("timezone"),
        scene_id: row.get("scene_id"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
    })
}

fn map_message(row: sqlx::postgres::PgRow) -> AppResult<AgentMessage> {
    // 行映射保证消息角色在出库时恢复为枚举类型。
    Ok(AgentMessage {
        id: row.get("id"),
        session_id: row.get("session_id"),
        role: parse_message_role(row.get::<&str, _>("role"))?,
        content: row.get("content"),
        turn_number: row.get("turn_number"),
        created_at: row.get("created_at"),
    })
}

fn map_usage_log(row: sqlx::postgres::PgRow) -> AppResult<LlmUsageLog> {
    // 行映射把 usage 记录恢复为 domain 日志实体。
    Ok(LlmUsageLog {
        id: row.get("id"),
        session_id: row.get("session_id"),
        provider_key: parse_provider_key(row.get::<&str, _>("provider_key"))?,
        model_name: row.get("model_name"),
        prompt_tokens: row.get("prompt_tokens"),
        completion_tokens: row.get("completion_tokens"),
        is_error: row.get("is_error"),
        error_message: row.get("error_message"),
        created_at: row.get("created_at"),
    })
}

#[async_trait]
impl PromptTemplateRepository for PostgresPromptTemplateRepository {
    async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>> {
        let row = sqlx::query("select * from llm_prompt_templates where template_key = $1 limit 1")
            .bind(key.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        row.map(map_template).transpose()
    }

    async fn list_all(&self) -> AppResult<Vec<PromptTemplate>> {
        let rows = sqlx::query("select * from llm_prompt_templates order by template_key")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(map_template).collect()
    }

    async fn upsert(&self, template: &PromptTemplate) -> AppResult<PromptTemplate> {
        // 模板写入使用 template_key 做唯一归属。
        let row = sqlx::query(
            r#"
            insert into llm_prompt_templates (template_key, template_text, updated_at)
            values ($1,$2,now())
            on conflict (template_key) do update set
              template_text = excluded.template_text,
              updated_at = now()
            returning *
            "#,
        )
        .bind(template.template_key.as_str())
        .bind(&template.template_text)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        map_template(row)
    }
}

#[async_trait]
impl PartnerConversationPromptOverrideRepository
    for PostgresPartnerConversationPromptOverrideRepository
{
    async fn get_by_partner_id(
        &self,
        partner_id: i64,
    ) -> AppResult<Option<PartnerConversationPromptOverride>> {
        let row = sqlx::query(
            "select * from llm_partner_conversation_prompt_overrides where partner_id = $1 limit 1",
        )
        .bind(partner_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.map(map_partner_prompt_override))
    }

    async fn upsert(
        &self,
        config: &PartnerConversationPromptOverride,
    ) -> AppResult<PartnerConversationPromptOverride> {
        let row = sqlx::query(
            r#"
            insert into llm_partner_conversation_prompt_overrides (
              partner_id,
              system_prompt_1,
              system_prompt_2,
              system_prompt_3,
              welcome_user_prompt,
              updated_by,
              updated_at,
              created_at
            )
            values ($1,$2,$3,$4,$5,$6,now(),coalesce($7, now()))
            on conflict (partner_id) do update set
              system_prompt_1 = excluded.system_prompt_1,
              system_prompt_2 = excluded.system_prompt_2,
              system_prompt_3 = excluded.system_prompt_3,
              welcome_user_prompt = excluded.welcome_user_prompt,
              updated_by = excluded.updated_by,
              updated_at = now()
            returning *
            "#,
        )
        .bind(config.partner_id)
        .bind(&config.system_prompt_1)
        .bind(&config.system_prompt_2)
        .bind(&config.system_prompt_3)
        .bind(&config.welcome_user_prompt)
        .bind(&config.updated_by)
        .bind(config.created_at)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(map_partner_prompt_override(row))
    }

    async fn delete_by_partner_id(&self, partner_id: i64) -> AppResult<()> {
        sqlx::query("delete from llm_partner_conversation_prompt_overrides where partner_id = $1")
            .bind(partner_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}

#[async_trait]
impl AgentSessionRepository for PostgresAgentSessionRepository {
    async fn create(&self, session: &AgentSession) -> AppResult<AgentSession> {
        // 会话创建时由数据库补齐 started_at 等默认列。
        let row = sqlx::query(
            r#"
            insert into llm_sessions (
              id,
              caller_kind,
              user_id,
              character_id,
              timezone,
              scene_id
            )
            values ($1,$2,$3,$4,$5,$6)
            returning *
            "#,
        )
        .bind(&session.id)
        .bind(session.caller.kind())
        .bind(session.caller.platform_user_id())
        .bind(session.character_id)
        .bind(&session.timezone)
        .bind(session.scene_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        map_session(row)
    }

    async fn get_by_id(&self, session_id: &str) -> AppResult<Option<AgentSession>> {
        let row = sqlx::query("select * from llm_sessions where id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        row.map(map_session).transpose()
    }

    async fn end(&self, session_id: &str) -> AppResult<()> {
        // 结束会话只更新 ended_at，不改写其他业务字段。
        sqlx::query("update llm_sessions set ended_at = now() where id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}

#[async_trait]
impl AgentMessageRepository for PostgresAgentMessageRepository {
    async fn append(&self, message: &AgentMessage) -> AppResult<AgentMessage> {
        // 消息追加保持顺序写入，不在仓储层做额外编排。
        let row = sqlx::query(
            r#"
            insert into llm_messages (session_id, role, content, turn_number)
            values ($1,$2,$3,$4)
            returning *
            "#,
        )
        .bind(&message.session_id)
        .bind(message.role.as_str())
        .bind(&message.content)
        .bind(message.turn_number)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        map_message(row)
    }

    async fn list_recent(
        &self,
        session_id: &str,
        recent_turns: i32,
    ) -> AppResult<Vec<AgentMessage>> {
        // recent_turns 乘二是为了同时覆盖用户和助手两个方向的消息。
        let rows = sqlx::query(
            "select * from llm_messages where session_id = $1 order by created_at desc, id desc limit $2",
        )
        .bind(session_id)
        .bind(recent_turns * 2)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        let mut messages = rows
            .into_iter()
            .map(map_message)
            .collect::<AppResult<Vec<_>>>()?;
        messages.reverse();
        Ok(messages)
    }

    async fn list_all(&self, session_id: &str) -> AppResult<Vec<AgentMessage>> {
        let rows = sqlx::query(
            "select * from llm_messages where session_id = $1 order by created_at asc, id asc",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        rows.into_iter().map(map_message).collect()
    }

    async fn next_turn_number(&self, session_id: &str) -> AppResult<i32> {
        // 下一轮编号由当前会话最大 turn_number 递增得到。
        let max_turn: Option<i32> =
            sqlx::query_scalar("select max(turn_number) from llm_messages where session_id = $1")
                .bind(session_id)
                .fetch_one(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
        Ok(max_turn.unwrap_or(0) + 1)
    }
}

#[async_trait]
impl AgentArchiveRepository for PostgresAgentArchiveRepository {
    async fn create(&self, archive: &AgentMessageArchive) -> AppResult<AgentMessageArchive> {
        // 归档消息以 JSON 形式持久化，避免再建一套归档明细表。
        let messages_json = serde_json::to_value(&archive.messages)
            .map_err(|_| AppError::internal("invalid archive messages"))?;
        let row = sqlx::query(
            r#"
            insert into llm_message_archives (
              character_id, messages, total_turns, prompt_tokens, completion_tokens
            )
            values ($1,$2,$3,$4,$5)
            returning *
            "#,
        )
        .bind(archive.character_id)
        .bind(messages_json)
        .bind(archive.total_turns)
        .bind(archive.prompt_tokens)
        .bind(archive.completion_tokens)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        let messages: Vec<AgentArchiveMessage> = serde_json::from_value(row.get("messages"))
            .map_err(|_| AppError::internal("invalid archive messages"))?;
        Ok(AgentMessageArchive {
            id: row.get("id"),
            character_id: row.get("character_id"),
            messages,
            total_turns: row.get("total_turns"),
            prompt_tokens: row.get("prompt_tokens"),
            completion_tokens: row.get("completion_tokens"),
            archived_at: row.get("archived_at"),
        })
    }
}

#[async_trait]
impl UsageLogRepository for PostgresAgentUsageLogRepository {
    async fn create(&self, log: &LlmUsageLog) -> AppResult<LlmUsageLog> {
        // 每次模型调用结果都会被固化为一条 usage 日志。
        let row = sqlx::query(
            r#"
            insert into llm_usage_logs (
              session_id, provider_key, model_name, prompt_tokens, completion_tokens, is_error, error_message
            )
            values ($1,$2,$3,$4,$5,$6,$7)
            returning *
            "#,
        )
        .bind(&log.session_id)
        .bind(log.provider_key.as_str())
        .bind(&log.model_name)
        .bind(log.prompt_tokens)
        .bind(log.completion_tokens)
        .bind(log.is_error)
        .bind(&log.error_message)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        map_usage_log(row)
    }

    async fn get_usage_stats(&self) -> AppResult<LlmUsageStats> {
        // 管理端统计直接由数据库聚合生成。
        let row = sqlx::query(
            r#"
            select
              count(*) as total_calls,
              coalesce(sum(prompt_tokens), 0) as total_prompt_tokens,
              coalesce(sum(completion_tokens), 0) as total_completion_tokens,
              coalesce(sum(case when is_error then 1 else 0 end), 0) as total_errors
            from llm_usage_logs
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        let total_calls: i64 = row.get("total_calls");
        let total_errors: i64 = row.get("total_errors");
        Ok(LlmUsageStats {
            total_calls,
            total_prompt_tokens: row.get("total_prompt_tokens"),
            total_completion_tokens: row.get("total_completion_tokens"),
            total_errors,
            error_rate: if total_calls == 0 {
                0.0
            } else {
                total_errors as f64 / total_calls as f64
            },
        })
    }

    async fn summarize_session(&self, session_id: &str) -> AppResult<SessionUsageSummary> {
        let row = sqlx::query(
            r#"
            select
              coalesce(sum(prompt_tokens), 0) as prompt_tokens,
              coalesce(sum(completion_tokens), 0) as completion_tokens
            from llm_usage_logs
            where session_id = $1
            "#,
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(SessionUsageSummary {
            prompt_tokens: row.get("prompt_tokens"),
            completion_tokens: row.get("completion_tokens"),
        })
    }
}

#[async_trait]
impl AgentSettingsRepository for PostgresAgentSettingsRepository {
    async fn get_recent_turns(&self) -> AppResult<i32> {
        let row =
            sqlx::query("select recent_turns from agent_settings where config_key = 'default'")
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
        Ok(match row {
            Some(row) => row.get::<i32, _>("recent_turns").clamp(1, 20),
            None => 6,
        })
    }

    async fn set_recent_turns(&self, recent_turns: i32) -> AppResult<()> {
        sqlx::query(
            "insert into agent_settings (config_key, recent_turns, updated_at) values ('default', $1, now()) on conflict (config_key) do update set recent_turns = excluded.recent_turns, updated_at = excluded.updated_at",
        )
        .bind(recent_turns.clamp(1, 20))
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }
}
