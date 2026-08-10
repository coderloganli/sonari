use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use shared_kernel::{AppError, AppResult};

use crate::{
    domain::{CallActivityEvent, CallActivityLog, CallActivityRound, CallTimelineEvent},
    ports::{
        CallConversationMessageFact, CallEventLogRepository, CallLogFact, CallLogFactsQuery,
        CallPromptLogFact,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct CallLogListQuery {
    pub character_id: Option<i64>,
    pub page: i64,
    pub page_size: i64,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PagedCallLogsView {
    pub items: Vec<CallLogListItemView>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallLogListItemView {
    pub session_id: i64,
    pub user_id: Option<i64>,
    pub caller: CallLogCallerView,
    pub character_id: i64,
    pub character_name: String,
    pub scene_id: Option<i64>,
    pub scene_name: Option<String>,
    pub status: String,
    pub runtime_status: String,
    pub runtime_failure_reason: Option<String>,
    pub owner_instance: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: i32,
    pub event_count: i64,
    pub round_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallDetailView {
    pub session: CallSessionSummaryView,
    /// 原始对话(用户↔AI 逐轮),来自 llm_messages。详情主视图展示这个。
    pub conversation: Vec<CallConversationMessageView>,
    pub timeline: Vec<CallTimelineEvent>,
    pub activity_log: CallActivityLog,
    /// 历史遗留的 prompt 日志(agent_prompt_logged 管道,目前无数据);保留字段兼容,前端改读 conversation。
    pub prompt_logs: Vec<CallPromptLogView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallSessionSummaryView {
    pub session_id: i64,
    pub user_id: Option<i64>,
    pub caller: CallLogCallerView,
    pub character_id: i64,
    pub character_name: String,
    pub scene_id: Option<i64>,
    pub scene_name: Option<String>,
    pub voice: String,
    pub agent_session_id: String,
    pub asr_language: String,
    pub status: String,
    pub runtime_status: String,
    pub runtime_failure_reason: Option<String>,
    pub owner_instance: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: i32,
    pub event_count: i64,
    pub round_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallLogCallerView {
    pub caller_kind: String,
    pub platform_user_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallPromptLogView {
    pub id: i64,
    pub session_id: i64,
    pub round_id: Option<String>,
    pub source: String,
    pub model_name: String,
    pub request_messages: Vec<CallPromptLogMessageView>,
    pub response_text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallPromptLogMessageView {
    pub role: String,
    pub content: String,
}

/// 原始对话单条消息(展示给管理后台)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallConversationMessageView {
    pub id: i64,
    pub turn_number: i32,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

pub struct CallLogEngine<R> {
    repo: R,
}

impl<R> CallLogEngine<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }
}

#[async_trait]
pub trait CallLogUseCases: Send + Sync {
    async fn list_call_logs(&self, query: CallLogListQuery) -> AppResult<PagedCallLogsView>;
    async fn get_call_detail(&self, session_id: i64) -> AppResult<CallDetailView>;
    async fn get_timeline(&self, session_id: i64) -> AppResult<Vec<CallTimelineEvent>>;
    async fn get_activity_log(&self, session_id: i64) -> AppResult<CallActivityLog>;
}

#[async_trait]
impl<R> CallLogUseCases for CallLogEngine<R>
where
    R: CallEventLogRepository + Send + Sync,
{
    async fn list_call_logs(&self, query: CallLogListQuery) -> AppResult<PagedCallLogsView> {
        let page = if query.page < 1 { 1 } else { query.page };
        let page_size = query.page_size.clamp(1, 100);
        let facts_query = CallLogFactsQuery {
            character_id: query.character_id,
            page,
            page_size,
            date_from: query.date_from,
            date_to: query.date_to,
        };
        let (facts, total) = self.repo.list_call_log_facts(&facts_query).await?;

        Ok(PagedCallLogsView {
            items: facts
                .into_iter()
                .map(map_list_item)
                .collect::<AppResult<Vec<_>>>()?,
            total,
            page,
            page_size,
        })
    }

    async fn get_call_detail(&self, session_id: i64) -> AppResult<CallDetailView> {
        if session_id <= 0 {
            return Err(AppError::invalid_input("session_id is required"));
        }

        let fact = self
            .repo
            .get_call_log_fact(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        let timeline = self.repo.list_timeline(session_id).await?;
        let activity_log =
            build_activity_log(session_id, fact.started_at.timestamp_millis(), &timeline);
        let prompt_logs = self
            .repo
            .list_prompt_logs(session_id)
            .await?
            .into_iter()
            .map(map_prompt_log)
            .collect();
        let conversation = self
            .repo
            .list_call_conversation(session_id)
            .await?
            .into_iter()
            .map(map_conversation_message)
            .collect();

        Ok(CallDetailView {
            session: map_detail_item(fact)?,
            conversation,
            timeline,
            activity_log,
            prompt_logs,
        })
    }

    async fn get_timeline(&self, session_id: i64) -> AppResult<Vec<CallTimelineEvent>> {
        if session_id <= 0 {
            return Err(AppError::invalid_input("session_id is required"));
        }
        self.repo.list_timeline(session_id).await
    }

    async fn get_activity_log(&self, session_id: i64) -> AppResult<CallActivityLog> {
        if session_id <= 0 {
            return Err(AppError::invalid_input("session_id is required"));
        }

        let started_at_ms = self
            .repo
            .get_started_at_ms(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("call session not found"))?;
        let timeline = self.repo.list_timeline(session_id).await?;
        Ok(build_activity_log(session_id, started_at_ms, &timeline))
    }
}

fn map_list_item(fact: CallLogFact) -> AppResult<CallLogListItemView> {
    if fact.character_name.trim().is_empty() {
        return Err(AppError::internal(format!(
            "call session {} is missing durable character log snapshot",
            fact.session_id
        )));
    }
    validate_scene_snapshot(fact.session_id, fact.scene_id, fact.scene_name.as_deref())?;

    Ok(CallLogListItemView {
        session_id: fact.session_id,
        user_id: fact.caller.platform_user_id(),
        caller: call_log_caller_view(&fact.caller),
        character_id: fact.character_id,
        character_name: fact.character_name,
        scene_id: fact.scene_id,
        scene_name: fact.scene_name,
        status: fact.status,
        runtime_status: fact.runtime_status,
        runtime_failure_reason: fact.runtime_failure_reason,
        owner_instance: fact.owner_instance,
        started_at: fact.started_at,
        ended_at: fact.ended_at,
        duration_seconds: fact.duration_seconds,
        event_count: fact.event_count,
        round_count: fact.round_count,
    })
}

fn map_detail_item(fact: CallLogFact) -> AppResult<CallSessionSummaryView> {
    if fact.character_name.trim().is_empty() {
        return Err(AppError::internal(format!(
            "call session {} is missing durable character log snapshot",
            fact.session_id
        )));
    }
    validate_scene_snapshot(fact.session_id, fact.scene_id, fact.scene_name.as_deref())?;

    Ok(CallSessionSummaryView {
        session_id: fact.session_id,
        user_id: fact.caller.platform_user_id(),
        caller: call_log_caller_view(&fact.caller),
        character_id: fact.character_id,
        character_name: fact.character_name,
        scene_id: fact.scene_id,
        scene_name: fact.scene_name,
        voice: fact.voice,
        agent_session_id: fact.agent_session_id,
        asr_language: fact.asr_language,
        status: fact.status,
        runtime_status: fact.runtime_status,
        runtime_failure_reason: fact.runtime_failure_reason,
        owner_instance: fact.owner_instance,
        started_at: fact.started_at,
        ended_at: fact.ended_at,
        duration_seconds: fact.duration_seconds,
        event_count: fact.event_count,
        round_count: fact.round_count,
    })
}

fn validate_scene_snapshot(
    session_id: i64,
    scene_id: Option<i64>,
    scene_name: Option<&str>,
) -> AppResult<()> {
    match (scene_id, scene_name) {
        (None, None) => Ok(()),
        (Some(_), Some(scene_name)) if !scene_name.trim().is_empty() => Ok(()),
        _ => Err(AppError::internal(format!(
            "call session {session_id} has inconsistent durable scene log snapshot"
        ))),
    }
}

fn call_log_caller_view(caller: &crate::domain::CallCallerIdentity) -> CallLogCallerView {
    match caller {
        crate::domain::CallCallerIdentity::PlatformUser { user_id } => CallLogCallerView {
            caller_kind: "platform_user".to_owned(),
            platform_user_id: Some(*user_id),
        },
    }
}

fn map_prompt_log(log: CallPromptLogFact) -> CallPromptLogView {
    CallPromptLogView {
        id: log.id,
        session_id: log.session_id,
        round_id: log.round_id,
        source: log.source,
        model_name: log.model_name,
        request_messages: log
            .request_messages
            .into_iter()
            .map(|message| CallPromptLogMessageView {
                role: message.role,
                content: message.content,
            })
            .collect(),
        response_text: log.response_text,
        created_at: log.created_at,
    }
}

fn map_conversation_message(message: CallConversationMessageFact) -> CallConversationMessageView {
    CallConversationMessageView {
        id: message.id,
        turn_number: message.turn_number,
        role: message.role,
        content: message.content,
        created_at: message.created_at,
    }
}

fn build_activity_log(
    session_id: i64,
    started_at_ms: i64,
    timeline: &[CallTimelineEvent],
) -> CallActivityLog {
    let mut rounds: BTreeMap<String, Vec<CallActivityEvent>> = BTreeMap::new();

    for event in timeline {
        let Some(name) = map_activity_name(event) else {
            continue;
        };
        let round_id = event
            .round_id
            .clone()
            .or_else(|| {
                event
                    .fields
                    .get("round_id")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "session".to_owned());

        rounds.entry(round_id).or_default().push(CallActivityEvent {
            name,
            timestamp: event.ts_ms,
            offset_ms: event.ts_ms.saturating_sub(started_at_ms),
        });
    }

    let rounds = rounds
        .into_iter()
        .map(|(round_id, mut events)| {
            events.sort_by_key(|event| event.timestamp);
            CallActivityRound { round_id, events }
        })
        .collect();

    CallActivityLog {
        session_id,
        started_at_ms,
        rounds,
    }
}

fn map_activity_name(event: &CallTimelineEvent) -> Option<String> {
    let name = match event.event.as_str() {
        "call_start_requested" => "call_start_requested",
        "sdk_call_start_requested" => "sdk_call_start_requested",
        "call_started" => "call_started",
        "call_start_failed" => "call_start_failed",
        "sdk_call_start_failed" => "sdk_call_start_failed",
        "runtime_start_claimed" => "runtime_start_claimed",
        "runtime_launch_prepared" => "runtime_launch_prepared",
        "runtime_failed" => "runtime_failed",
        "runtime_stop_requested" => "runtime_stop_requested",
        "call_end_requested" => "call_end_requested",
        "sdk_call_end_requested" => "sdk_call_end_requested",
        "call_ended" => "call_ended",
        "worker_runtime_started" => "worker_runtime_started",
        "worker_runtime_stopped" => "worker_runtime_stopped",
        "worker_runtime_failed" => "worker_runtime_failed",
        "speech_listening_started" => "speech_listening_started",
        "speech_detected" => "speech_detected",
        "speech_utterance_flushing" => "speech_utterance_flushing",
        "speech_asr_started" => "speech_asr_started",
        "speech_asr_completed" => "speech_asr_completed",
        "speech_agent_started" => "speech_agent_started",
        "speech_agent_completed" => "speech_agent_completed",
        "speech_tts_started" => "speech_tts_started",
        "speech_tts_completed" => "speech_tts_completed",
        "speech_reply_started" => "speech_reply_started",
        "speech_reply_finished" => "speech_reply_finished",
        "speech_session_failed" => "speech_session_failed",
        "bot_speech_enqueued" => "bot_speech_enqueued",
        "bot_speech_started" => "bot_speech_started",
        "bot_speech_completed" => "bot_speech_completed",
        "bot_speech_interrupted" => "bot_speech_interrupted",
        "bot_speech_cancelled" => "bot_speech_cancelled",
        "queue_flushed" => "queue_flushed",
        "external_audio_started" => "external_audio_started",
        "external_audio_finished" => "external_audio_finished",
        _ => return None,
    };
    Some(name.to_owned())
}
