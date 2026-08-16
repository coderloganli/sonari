//! 输入/ASR 进度写入的「比较-决策」纯逻辑。
//!
//! 这段逻辑与具体存储无关(无 Redis、无网络),决定在某个 phase 下收到一份新的会话快照时
//! 应该「写入(并保留哪些字段)」还是「仅 touch(返回 Terminal/Dropped)」。Redis 适配器与
//! 进程内内存适配器共用同一份决策,保证两种存储下的会话状态机语义完全一致。

use crate::application::{SpeechSessionPhase, StoredSpeechSession};

/// 进度写入模式:普通输入帧 / ASR 进度 / 关闭排空。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputProgressWriteMode {
    Input,
    Asr,
    ClosingDrain,
}

/// 比较后的写入结果分类(供适配器映射为对外的 SpeechSessionInputProgressSaveResult)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Retained so the store contract stays exhaustive.
pub(crate) enum InputProgressWriteResult {
    Updated,
    Missing,
    Terminal,
    Dropped,
}

/// 决策:写入一份(已根据 phase 调整过的)会话,或仅 touch 并返回某结果。
pub(crate) enum InputProgressWriteDecision {
    Write(Box<StoredSpeechSession>),
    Touch(InputProgressWriteResult),
}

/// 给定当前已存储会话、待写入的提议会话与写入模式,决定如何写入。
///
/// 关键不变量:
/// - Failed / (非 ClosingDrain 模式下的)Closing → 终态,拒绝(Terminal)。
/// - Responding 且为普通 Input 模式 → 丢弃(Dropped),不覆盖正在进行的输出轮。
/// - Closing / Responding 写入时,保留当前的 phase / active_turn / dispatched_round_ids,
///   只更新输入侧累积(避免输入进度覆盖输出轮状态)。
pub(crate) fn build_input_progress_write_decision(
    current_session: StoredSpeechSession,
    proposed_session: &StoredSpeechSession,
    mode: InputProgressWriteMode,
) -> InputProgressWriteDecision {
    match current_session.phase {
        SpeechSessionPhase::Failed => {
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Terminal)
        }
        SpeechSessionPhase::Closing if mode != InputProgressWriteMode::ClosingDrain => {
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Terminal)
        }
        SpeechSessionPhase::Responding if mode == InputProgressWriteMode::Input => {
            InputProgressWriteDecision::Touch(InputProgressWriteResult::Dropped)
        }
        SpeechSessionPhase::Closing => {
            let mut session = proposed_session.clone();
            session.phase = SpeechSessionPhase::Closing;
            session.active_turn = current_session.active_turn;
            session.dispatched_round_ids = current_session.dispatched_round_ids;
            InputProgressWriteDecision::Write(Box::new(session))
        }
        SpeechSessionPhase::Responding => {
            let mut session = proposed_session.clone();
            session.phase = SpeechSessionPhase::Responding;
            session.active_turn = current_session.active_turn;
            session.dispatched_round_ids = current_session.dispatched_round_ids;
            InputProgressWriteDecision::Write(Box::new(session))
        }
        _ => {
            let mut session = proposed_session.clone();
            session.active_turn = current_session.active_turn;
            session.dispatched_round_ids = current_session.dispatched_round_ids;
            InputProgressWriteDecision::Write(Box::new(session))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{ActiveSpeechTurn, PendingAsrRound, SpeechSegmentationConfig};
    use serde_json::json;
    use std::collections::VecDeque;

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
            pending_rounds: VecDeque::new(),
            dispatched_round_ids: Vec::new(),
            active_turn: None,
            candidate_speech_ms: 0,
            candidate_pcm: Vec::new(),
            turn_timings: Default::default(),
        }
    }

    #[test]
    fn input_progress_write_preserves_empty_array_json_shape() -> Result<(), String> {
        let current = test_session();
        let proposed = test_session();

        let session = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::Input,
        ) {
            InputProgressWriteDecision::Write(session) => session,
            InputProgressWriteDecision::Touch(_) => return Err("expected session write".to_owned()),
        };
        let value = serde_json::to_value(session).map_err(|error| error.to_string())?;

        assert_eq!(value["utterance_pcm"], json!([]));
        assert_eq!(value["pending_rounds"], json!([]));
        assert_eq!(value["dispatched_round_ids"], json!([]));
        Ok(())
    }

    #[test]
    fn responding_session_drops_input_progress_without_overwriting_active_turn()
    -> Result<(), String> {
        let mut current = test_session();
        current.phase = SpeechSessionPhase::Responding;
        current.active_turn = Some(ActiveSpeechTurn {
            round_id: "speech-1".to_owned(),
        });
        current.dispatched_round_ids = vec!["speech-1".to_owned()];
        let proposed = test_session();

        let result = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::Input,
        ) {
            InputProgressWriteDecision::Touch(result) => result,
            InputProgressWriteDecision::Write(_) => return Err("expected touch result".to_owned()),
        };

        assert_eq!(result, InputProgressWriteResult::Dropped);
        Ok(())
    }

    #[test]
    fn responding_session_accepts_asr_progress_without_overwriting_active_turn()
    -> Result<(), String> {
        let mut current = test_session();
        current.phase = SpeechSessionPhase::Responding;
        current.active_turn = Some(ActiveSpeechTurn {
            round_id: "bot-1".to_owned(),
        });
        current.dispatched_round_ids = vec!["bot-1".to_owned()];
        current.pending_rounds = VecDeque::from([PendingAsrRound {
            round_id: "speech-1".to_owned(),
            latest_partial_transcript: String::new(),
            commit_started_at_ms: None,
            force_agent_deadline_at_ms: None,
            turn_timings: Default::default(),
        }]);
        let mut proposed = test_session();
        proposed.pending_rounds = VecDeque::new();

        let session = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::Asr,
        ) {
            InputProgressWriteDecision::Write(session) => session,
            InputProgressWriteDecision::Touch(_) => {
                return Err("expected session write".to_owned());
            }
        };

        assert_eq!(session.phase, SpeechSessionPhase::Responding);
        assert_eq!(
            session
                .active_turn
                .as_ref()
                .map(|turn| turn.round_id.as_str()),
            Some("bot-1")
        );
        assert!(session.pending_rounds.is_empty());
        Ok(())
    }

    #[test]
    fn closing_session_rejects_normal_input_progress() -> Result<(), String> {
        let mut current = test_session();
        current.phase = SpeechSessionPhase::Closing;
        let proposed = test_session();

        let result = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::Input,
        ) {
            InputProgressWriteDecision::Touch(result) => result,
            InputProgressWriteDecision::Write(_) => return Err("expected touch result".to_owned()),
        };

        assert_eq!(result, InputProgressWriteResult::Terminal);
        Ok(())
    }

    #[test]
    fn closing_drain_preserves_closing_phase() -> Result<(), String> {
        let mut current = test_session();
        current.phase = SpeechSessionPhase::Closing;
        current.active_turn = Some(ActiveSpeechTurn {
            round_id: "speech-1".to_owned(),
        });
        current.dispatched_round_ids = vec!["speech-1".to_owned()];
        let mut proposed = test_session();
        proposed.phase = SpeechSessionPhase::Listening;
        proposed.utterance_pcm = vec![1, 2, 3];

        let session = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::ClosingDrain,
        ) {
            InputProgressWriteDecision::Write(session) => session,
            InputProgressWriteDecision::Touch(_) => return Err("expected session write".to_owned()),
        };

        assert_eq!(session.phase, SpeechSessionPhase::Closing);
        assert_eq!(
            session
                .active_turn
                .as_ref()
                .map(|turn| turn.round_id.as_str()),
            Some("speech-1")
        );
        assert_eq!(session.dispatched_round_ids, vec!["speech-1".to_owned()]);
        assert_eq!(session.utterance_pcm, vec![1, 2, 3]);
        Ok(())
    }

    #[test]
    fn failed_session_rejects_input_progress() -> Result<(), String> {
        let mut current = test_session();
        current.phase = SpeechSessionPhase::Failed;
        let proposed = test_session();

        let result = match build_input_progress_write_decision(
            current,
            &proposed,
            InputProgressWriteMode::Input,
        ) {
            InputProgressWriteDecision::Touch(result) => result,
            InputProgressWriteDecision::Write(_) => return Err("expected touch result".to_owned()),
        };

        assert_eq!(result, InputProgressWriteResult::Terminal);
        Ok(())
    }
}
