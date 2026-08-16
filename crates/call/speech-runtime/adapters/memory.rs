//! 进程内内存版 `SpeechSessionStorePort`。
//!
//! 用于 co-located 编排:speech-runtime 直接跑在 worker 进程内时,会话状态与输出事件队列
//! 单进程单属主,无需外部共享存储。语义与 Redis 适配器保持一致(复用同一份
//! `build_input_progress_write_decision` 比较-决策逻辑),但用进程内 `Mutex<HashMap>`
//! 替代 Redis 的 WATCH/MULTI 乐观并发与 List 队列;无 TTL(会话随 worker 持有,关闭时 take/移除)。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use crate::application::session_progress::{
    InputProgressWriteDecision, InputProgressWriteMode, InputProgressWriteResult,
    build_input_progress_write_decision,
};
use crate::{
    SpeechRuntimeEvent, SpeechSessionInputProgressSaveResult, SpeechSessionStorePort,
    StoredSpeechSession,
};

struct SessionEntry {
    session: StoredSpeechSession,
    events: VecDeque<SpeechRuntimeEvent>,
}

/// 进程内会话存储:`speech_session_id -> (会话状态, 输出事件队列)`。
#[derive(Clone, Default)]
pub struct InMemorySpeechSessionStore {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
}

impl InMemorySpeechSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionEntry>> {
        // 进程内锁不跨 await 持有;中毒(panic)极少见,直接恢复内部数据继续。
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn save_progress(
        &self,
        speech_session_id: &str,
        proposed: &StoredSpeechSession,
        mode: InputProgressWriteMode,
    ) -> AppResult<SpeechSessionInputProgressSaveResult> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            return Err(AppError::not_found("speech session not found"));
        };
        match build_input_progress_write_decision(entry.session.clone(), proposed, mode) {
            InputProgressWriteDecision::Write(session) => {
                entry.session = *session;
                Ok(SpeechSessionInputProgressSaveResult::Saved)
            }
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Terminal) => {
                Ok(SpeechSessionInputProgressSaveResult::Terminal)
            }
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Dropped) => {
                Ok(SpeechSessionInputProgressSaveResult::Dropped)
            }
            // Missing/Updated 不会由 build_* 产出(Missing 由上面的 entry 缺失分支处理)。
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Missing) => {
                Err(AppError::not_found("speech session not found"))
            }
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Updated) => {
                Ok(SpeechSessionInputProgressSaveResult::Saved)
            }
        }
    }
}

#[async_trait]
impl SpeechSessionStorePort for InMemorySpeechSessionStore {
    async fn create_session(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<()> {
        let mut sessions = self.lock();
        if sessions.contains_key(speech_session_id) {
            return Err(AppError::conflict("speech session already exists"));
        }
        sessions.insert(
            speech_session_id.to_owned(),
            SessionEntry {
                session: session.clone(),
                events: VecDeque::new(),
            },
        );
        Ok(())
    }

    async fn get_session(&self, speech_session_id: &str) -> AppResult<Option<StoredSpeechSession>> {
        Ok(self
            .lock()
            .get(speech_session_id)
            .map(|entry| entry.session.clone()))
    }

    async fn save_session(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<()> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            return Err(AppError::not_found("speech session not found"));
        };
        entry.session = session.clone();
        Ok(())
    }

    async fn save_input_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult> {
        self.save_progress(speech_session_id, session, InputProgressWriteMode::Input)
    }

    async fn save_asr_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult> {
        self.save_progress(speech_session_id, session, InputProgressWriteMode::Asr)
    }

    async fn save_closing_drain_progress(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
    ) -> AppResult<SpeechSessionInputProgressSaveResult> {
        self.save_progress(
            speech_session_id,
            session,
            InputProgressWriteMode::ClosingDrain,
        )
    }

    async fn save_session_and_push_events(
        &self,
        speech_session_id: &str,
        session: &StoredSpeechSession,
        events: &[SpeechRuntimeEvent],
    ) -> AppResult<()> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            return Err(AppError::not_found("speech session not found"));
        };
        entry.session = session.clone();
        entry.events.extend(events.iter().cloned());
        Ok(())
    }

    async fn take_session(
        &self,
        speech_session_id: &str,
    ) -> AppResult<Option<StoredSpeechSession>> {
        Ok(self
            .lock()
            .remove(speech_session_id)
            .map(|entry| entry.session))
    }

    async fn push_events(
        &self,
        speech_session_id: &str,
        events: &[SpeechRuntimeEvent],
    ) -> AppResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            return Err(AppError::not_found("speech session not found"));
        };
        entry.events.extend(events.iter().cloned());
        Ok(())
    }

    async fn pop_events(
        &self,
        speech_session_id: &str,
        max_events: usize,
    ) -> AppResult<Vec<SpeechRuntimeEvent>> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            // 与 Redis 一致:会话不存在时事件查询返回空(而非错误)。
            return Ok(Vec::new());
        };
        let take = max_events.min(entry.events.len());
        Ok(entry.events.drain(0..take).collect())
    }

    async fn drain_events(&self, speech_session_id: &str) -> AppResult<Vec<SpeechRuntimeEvent>> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(speech_session_id) else {
            return Ok(Vec::new());
        };
        Ok(entry.events.drain(..).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{SpeechSegmentationConfig, SpeechSessionPhase};

    fn test_session() -> StoredSpeechSession {
        StoredSpeechSession {
            session_id: 1,
            runtime_owner_id: "worker-test".to_owned(),
            agent_session_id: "agent-test".to_owned(),
            owner_backend_url: "http://backend:8080".to_owned(),
            voice: "voice-7".to_owned(),
            asr_session_id: "asr-test".to_owned(),
            asr_owner_instance_id: "backend-test".to_owned(),
            asr_owner_instance_epoch: "epoch-test".to_owned(),
            language: voice::AsrLanguage::Zh,
            sample_rate_hz: 16_000,
            num_channels: 1,
            segmentation_config: SpeechSegmentationConfig {
                min_utterance_ms: 1,
                silence_flush_ms: 1,
                silence_force_agent_ms: 1,
                voice_activity_threshold: 1,
                min_speech_confirm_ms: 0,
            },
            utterance_pcm: Vec::new(),
            phase: SpeechSessionPhase::Listening,
            trailing_silence_ms: 0,
            deferred_flush: false,
            next_round_seq: 1,
            pending_rounds: std::collections::VecDeque::new(),
            dispatched_round_ids: Vec::new(),
            active_turn: None,
            candidate_speech_ms: 0,
            candidate_pcm: Vec::new(),
            turn_timings: Default::default(),
        }
    }

    #[tokio::test]
    async fn create_get_and_conflict() {
        let store = InMemorySpeechSessionStore::new();
        store.create_session("s1", &test_session()).await.unwrap();
        assert!(store.get_session("s1").await.unwrap().is_some());
        assert!(store.get_session("missing").await.unwrap().is_none());
        // 重复 create → conflict
        assert!(store.create_session("s1", &test_session()).await.is_err());
    }

    #[tokio::test]
    async fn save_session_requires_existing() {
        let store = InMemorySpeechSessionStore::new();
        assert!(store.save_session("s1", &test_session()).await.is_err());
        store.create_session("s1", &test_session()).await.unwrap();
        let mut updated = test_session();
        updated.phase = SpeechSessionPhase::Flushing;
        store.save_session("s1", &updated).await.unwrap();
        assert_eq!(
            store.get_session("s1").await.unwrap().unwrap().phase,
            SpeechSessionPhase::Flushing
        );
    }

    #[tokio::test]
    async fn input_progress_drops_during_responding_and_rejects_terminal() {
        let store = InMemorySpeechSessionStore::new();
        let mut responding = test_session();
        responding.phase = SpeechSessionPhase::Responding;
        store.create_session("s1", &responding).await.unwrap();
        // Responding 下普通 Input 进度被丢弃,不覆盖输出轮
        let result = store
            .save_input_progress("s1", &test_session())
            .await
            .unwrap();
        assert_eq!(result, SpeechSessionInputProgressSaveResult::Dropped);
        assert_eq!(
            store.get_session("s1").await.unwrap().unwrap().phase,
            SpeechSessionPhase::Responding
        );
        // Failed 下任何输入进度被拒(Terminal)
        let mut failed = test_session();
        failed.phase = SpeechSessionPhase::Failed;
        store.save_session("s1", &failed).await.unwrap();
        let result = store
            .save_input_progress("s1", &test_session())
            .await
            .unwrap();
        assert_eq!(result, SpeechSessionInputProgressSaveResult::Terminal);
    }

    #[tokio::test]
    async fn events_push_pop_drain_fifo() {
        let store = InMemorySpeechSessionStore::new();
        store.create_session("s1", &test_session()).await.unwrap();
        let e = |id: &str| SpeechRuntimeEvent::ReplyFinished {
            round_id: id.to_owned(),
        };
        store
            .push_events("s1", &[e("r1"), e("r2"), e("r3")])
            .await
            .unwrap();
        let first = store.pop_events("s1", 2).await.unwrap();
        assert_eq!(first.len(), 2); // FIFO 取前两条
        let rest = store.drain_events("s1").await.unwrap();
        assert_eq!(rest.len(), 1);
        assert!(store.pop_events("s1", 10).await.unwrap().is_empty());
        // 不存在的会话事件查询返回空,不报错(与 Redis 一致)
        assert!(store.pop_events("missing", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn take_session_removes() {
        let store = InMemorySpeechSessionStore::new();
        store.create_session("s1", &test_session()).await.unwrap();
        assert!(store.take_session("s1").await.unwrap().is_some());
        assert!(store.get_session("s1").await.unwrap().is_none());
        assert!(store.take_session("s1").await.unwrap().is_none());
    }
}
