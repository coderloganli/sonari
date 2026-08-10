//! Prepares the LiveKit room and tokens a call launches into.

use std::sync::Arc;

use call_execution::SpeechBootstrapComposerPort;
use call_runtime_control::RuntimeLaunchSpec;
use rtc_control_contract::RuntimeLaunchArtifactsPort;
use shared_kernel::AppResult;

#[derive(Clone)]
pub struct LiveKitRuntimeLaunchProvision<R> {
    launch_artifacts: Arc<R>,
    // 进程内编排配置组装器:dispatch 时解析每会话非密钥配置填入 launch-spec;
    // 返回 None 则走旧后端编排路径(speech 为 None)。
    bootstrap_composer: Arc<dyn SpeechBootstrapComposerPort>,
}

impl<R> LiveKitRuntimeLaunchProvision<R> {
    pub fn new(
        launch_artifacts: Arc<R>,
        bootstrap_composer: Arc<dyn SpeechBootstrapComposerPort>,
    ) -> Self {
        Self {
            launch_artifacts,
            bootstrap_composer,
        }
    }
}

#[async_trait::async_trait]
impl<R> call_execution::RuntimeLaunchProvisionPort for LiveKitRuntimeLaunchProvision<R>
where
    R: RuntimeLaunchArtifactsPort + Send + Sync,
{
    async fn prepare_runtime_launch(
        &self,
        session_id: i64,
        expected_remote_participant_identity: String,
        voice: String,
        agent_session_id: &str,
        asr_language: &str,
    ) -> AppResult<RuntimeLaunchSpec> {
        let artifacts = self
            .launch_artifacts
            .issue_runtime_launch_artifacts(session_id, &expected_remote_participant_identity)
            .await?;
        let speech = self
            .bootstrap_composer
            .compose(voice, agent_session_id, asr_language)
            .await?;
        Ok(RuntimeLaunchSpec {
            endpoint: artifacts.endpoint,
            room_name: artifacts.room_name,
            access_token: artifacts.access_token,
            local_participant_identity: artifacts.local_participant_identity,
            expected_remote_participant_identity: artifacts.expected_remote_participant_identity,
            speech,
        })
    }
}
