use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use call_runtime_control::{RuntimeEventFact, RuntimeFactKind, RuntimeWorkItem};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

use crate::{
    client::{RuntimeControlClient, RuntimeStatusRequest, is_retryable_control_plane_error},
    config::WorkerConfig,
    pipeline::{RemoteSpeechHandler, SpeechHandler},
    runtime::ActiveRuntime,
};

/// Drives the dispatch loop until shutdown. The control plane is reached
/// through the handles passed in, never over the network.
pub async fn run(
    config: WorkerConfig,
    execution: Arc<dyn call_execution::CallExecutionUseCases>,
    speech: Arc<dyn speech_runtime::SpeechRuntimeUseCases>,
    agent: Arc<dyn agent::AgentRuntimeUseCases>,
    call_events: Arc<dyn call_log_contract::CallEventSinkPort>,
    voice_runtime: Arc<dyn voice::ports::VoiceRuntimeExecutionPort>,
) -> Result<()> {
    let client = RuntimeControlClient::new(execution, speech);
    let mut service = WorkerService::new(config, client, agent, call_events, voice_runtime);
    service.run().await
}

struct WorkerService {
    config: WorkerConfig,
    client: RuntimeControlClient,
    agent: Arc<dyn agent::AgentRuntimeUseCases>,
    call_events: Arc<dyn call_log_contract::CallEventSinkPort>,
    voice_runtime: Arc<dyn voice::ports::VoiceRuntimeExecutionPort>,
    active: HashMap<i64, ActiveRuntime>,
    pending_failures: HashMap<i64, PendingRuntimeFailure>,
    pending_actions: VecDeque<PendingControlPlaneAction>,
}

#[derive(Debug, Clone)]
struct PendingRuntimeFailure {
    reason: String,
    cleanup_done: bool,
}

#[derive(Debug, Clone)]
enum PendingControlPlaneAction {
    Status(RuntimeStatusRequest),
    Event(RuntimeEventFact),
}

impl WorkerService {
    fn new(
        config: WorkerConfig,
        client: RuntimeControlClient,
        agent: Arc<dyn agent::AgentRuntimeUseCases>,
        call_events: Arc<dyn call_log_contract::CallEventSinkPort>,
        voice_runtime: Arc<dyn voice::ports::VoiceRuntimeExecutionPort>,
    ) -> Self {
        Self {
            config,
            client,
            agent,
            call_events,
            voice_runtime,
            active: HashMap::new(),
            pending_failures: HashMap::new(),
            pending_actions: VecDeque::new(),
        }
    }

    async fn run(&mut self) -> Result<()> {
        let mut retry_backoff = ControlPlaneRetryBackoff::new(
            self.config.control_plane_retry_initial_ms,
            self.config.control_plane_retry_max_ms,
        );
        loop {
            match self.tick_once().await {
                Ok(()) => retry_backoff.reset(),
                Err(error) if is_retryable_control_plane_error(&error) => {
                    let retry_delay_ms = retry_backoff.next_delay_ms();
                    tracing::warn!(
                        error = %error,
                        retry_delay_ms,
                        consecutive_failures = retry_backoff.consecutive_failures(),
                        "worker control plane temporarily unavailable; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn tick_once(&mut self) -> Result<()> {
        self.reconcile_active_runtimes().await?;
        self.flush_pending_actions().await?;
        self.flush_pending_failures().await?;
        match self.client.poll_work(&self.config.runtime_owner_id).await? {
            Some(work) => self.handle_work(work).await,
            None => {
                tokio::time::sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                Ok(())
            }
        }
    }

    async fn reconcile_active_runtimes(&mut self) -> Result<()> {
        let session_ids: Vec<i64> = self.active.keys().copied().collect();
        let mut failed = Vec::new();

        for session_id in session_ids {
            let Some(runtime) = self.active.get_mut(&session_id) else {
                continue;
            };

            match runtime.poll_health().await? {
                Some(Ok(())) => {
                    tracing::debug!(
                        session_id,
                        "worker runtime completed unexpectedly without an explicit stop request"
                    );
                    failed.push((
                        session_id,
                        "runtime pipeline exited unexpectedly".to_owned(),
                    ));
                }
                Some(Err(error)) => {
                    tracing::debug!(
                        session_id,
                        error = %error,
                        "worker detected runtime pipeline failure"
                    );
                    failed.push((session_id, error.to_string()));
                }
                None => {}
            }
        }

        for (session_id, failure_reason) in failed {
            self.pending_failures
                .entry(session_id)
                .or_insert(PendingRuntimeFailure {
                    reason: failure_reason,
                    cleanup_done: false,
                });
        }

        Ok(())
    }

    async fn flush_pending_failures(&mut self) -> Result<()> {
        let session_ids: Vec<i64> = self.pending_failures.keys().copied().collect();
        for session_id in session_ids {
            let (failure_reason, cleanup_done) = match self.pending_failures.get(&session_id) {
                Some(pending) => (pending.reason.clone(), pending.cleanup_done),
                None => continue,
            };

            if !cleanup_done {
                if let Some(runtime) = self.active.remove(&session_id)
                    && let Err(error) = runtime.stop().await
                {
                    tracing::debug!(
                        session_id,
                        error = %error,
                        "worker failed to clean up runtime after pipeline failure"
                    );
                }
                if let Some(pending) = self.pending_failures.get_mut(&session_id) {
                    pending.cleanup_done = true;
                }
            }

            self.dispatch_actions_or_queue(vec![
                PendingControlPlaneAction::Status(RuntimeStatusRequest {
                    session_id,
                    runtime_owner_id: self.config.runtime_owner_id.clone(),
                    status: "failed".to_owned(),
                    failure_reason: Some(failure_reason.clone()),
                }),
                PendingControlPlaneAction::Event(RuntimeEventFact {
                    session_id,
                    runtime_owner_id: self.config.runtime_owner_id.clone(),
                    round_id: None,
                    source: "worker".to_owned(),
                    ts_ms: now_ms(),
                    kind: RuntimeFactKind::WorkerRuntimeFailed {
                        reason: failure_reason,
                    },
                }),
            ])
            .await?;
            self.pending_failures.remove(&session_id);
        }

        Ok(())
    }

    async fn handle_work(&mut self, work: RuntimeWorkItem) -> Result<()> {
        self.ensure_owned_work(&work)?;
        match work.kind.as_str() {
            "start" => self.handle_start_work(work).await,
            "stop" => self.handle_stop_work(work).await,
            other => Err(anyhow!("unsupported runtime work kind: {other}")),
        }
    }

    fn ensure_owned_work(&self, work: &RuntimeWorkItem) -> Result<()> {
        if work.runtime_owner_id != self.config.runtime_owner_id {
            return Err(anyhow!(
                "worker owner mismatch for session {}: expected {}, got {}",
                work.session_id,
                self.config.runtime_owner_id,
                work.runtime_owner_id
            ));
        }

        Ok(())
    }

    async fn handle_start_work(&mut self, work: RuntimeWorkItem) -> Result<()> {
        if self.active.contains_key(&work.session_id) {
            return Err(anyhow!(
                "duplicate start work for active runtime session {}",
                work.session_id
            ));
        }

        let launch = work
            .launch
            .ok_or_else(|| anyhow!("start work missing launch spec"))?;
        // 进程内编排:launch 携带 speech bootstrap(后端 COMBRABO_INPROCESS_SPEECH 启用)时,
        // worker 进程内运行 speech-runtime(消除每帧音频 HTTP);否则回退旧的远端编排。
        let handler: Arc<dyn SpeechHandler> = if let Some(bootstrap) = &launch.speech {
            tracing::info!(
                session_id = work.session_id,
                runtime_owner_id = %work.runtime_owner_id,
                "worker 启用进程内编排(co-located media pipeline)"
            );
            crate::local_orchestration::build_local_speech_handler(
                bootstrap,
                work.session_id,
                work.runtime_owner_id.clone(),
                &self.config,
                self.client.clone(),
                self.agent.clone(),
                self.call_events.clone(),
                self.voice_runtime.clone(),
            )?
        } else {
            Arc::new(RemoteSpeechHandler::new(
                self.client.clone(),
                work.session_id,
                work.runtime_owner_id.clone(),
                self.config.control_plane_retry_initial_ms,
                self.config.control_plane_retry_max_ms,
            ))
        };
        tracing::debug!(
            session_id = work.session_id,
            runtime_owner_id = %work.runtime_owner_id,
            "worker accepted runtime start work"
        );
        self.dispatch_actions_or_queue(vec![PendingControlPlaneAction::Event(RuntimeEventFact {
            session_id: work.session_id,
            runtime_owner_id: work.runtime_owner_id.clone(),
            round_id: None,
            source: "worker".to_owned(),
            ts_ms: now_ms(),
            kind: RuntimeFactKind::WorkerRuntimeStarted,
        })])
        .await?;

        match ActiveRuntime::start(
            &self.config,
            work.session_id,
            launch,
            self.client.clone(),
            handler,
        )
        .await
        {
            Ok(runtime) => {
                let runtime_owner_id = work.runtime_owner_id.clone();
                self.active.insert(work.session_id, runtime);
                self.dispatch_actions_or_queue(vec![
                    PendingControlPlaneAction::Status(RuntimeStatusRequest {
                        session_id: work.session_id,
                        runtime_owner_id: runtime_owner_id.clone(),
                        status: "ready".to_owned(),
                        failure_reason: None,
                    }),
                    PendingControlPlaneAction::Event(RuntimeEventFact {
                        session_id: work.session_id,
                        runtime_owner_id,
                        round_id: None,
                        source: "worker".to_owned(),
                        ts_ms: now_ms(),
                        kind: RuntimeFactKind::WorkerRuntimeReady,
                    }),
                ])
                .await
            }
            Err(error) => {
                let runtime_owner_id = work.runtime_owner_id.clone();
                let error_chain = format!("{error:#}");
                tracing::debug!(
                    session_id = work.session_id,
                    runtime_owner_id = %work.runtime_owner_id,
                    error = %error,
                    error_chain = %error_chain,
                    "worker failed to start runtime"
                );
                self.dispatch_actions_or_queue(vec![
                    PendingControlPlaneAction::Status(RuntimeStatusRequest {
                        session_id: work.session_id,
                        runtime_owner_id: runtime_owner_id.clone(),
                        status: "failed".to_owned(),
                        failure_reason: Some(error.to_string()),
                    }),
                    PendingControlPlaneAction::Event(RuntimeEventFact {
                        session_id: work.session_id,
                        runtime_owner_id,
                        round_id: None,
                        source: "worker".to_owned(),
                        ts_ms: now_ms(),
                        kind: RuntimeFactKind::WorkerRuntimeFailed {
                            reason: error.to_string(),
                        },
                    }),
                ])
                .await
            }
        }
    }

    async fn handle_stop_work(&mut self, work: RuntimeWorkItem) -> Result<()> {
        tracing::debug!(
            session_id = work.session_id,
            runtime_owner_id = %work.runtime_owner_id,
            "worker accepted runtime stop work"
        );

        if let Some(runtime) = self.active.remove(&work.session_id) {
            match runtime.stop().await {
                Ok(()) => {
                    let runtime_owner_id = work.runtime_owner_id.clone();
                    self.dispatch_actions_or_queue(vec![
                        PendingControlPlaneAction::Status(RuntimeStatusRequest {
                            session_id: work.session_id,
                            runtime_owner_id: runtime_owner_id.clone(),
                            status: "stopped".to_owned(),
                            failure_reason: None,
                        }),
                        PendingControlPlaneAction::Event(RuntimeEventFact {
                            session_id: work.session_id,
                            runtime_owner_id,
                            round_id: None,
                            source: "worker".to_owned(),
                            ts_ms: now_ms(),
                            kind: RuntimeFactKind::WorkerRuntimeStopped,
                        }),
                    ])
                    .await
                }
                Err(error) => {
                    let runtime_owner_id = work.runtime_owner_id.clone();
                    tracing::debug!(
                        session_id = work.session_id,
                        runtime_owner_id = %work.runtime_owner_id,
                        error = %error,
                        "worker failed to stop runtime"
                    );
                    self.dispatch_actions_or_queue(vec![
                        PendingControlPlaneAction::Status(RuntimeStatusRequest {
                            session_id: work.session_id,
                            runtime_owner_id: runtime_owner_id.clone(),
                            status: "failed".to_owned(),
                            failure_reason: Some(error.to_string()),
                        }),
                        PendingControlPlaneAction::Event(RuntimeEventFact {
                            session_id: work.session_id,
                            runtime_owner_id,
                            round_id: None,
                            source: "worker".to_owned(),
                            ts_ms: now_ms(),
                            kind: RuntimeFactKind::WorkerRuntimeFailed {
                                reason: error.to_string(),
                            },
                        }),
                    ])
                    .await
                }
            }
        } else {
            let runtime_owner_id = work.runtime_owner_id.clone();
            tracing::debug!(
                session_id = work.session_id,
                runtime_owner_id = %work.runtime_owner_id,
                "worker stop work could not find a local runtime to stop"
            );
            self.dispatch_actions_or_queue(vec![
                PendingControlPlaneAction::Status(RuntimeStatusRequest {
                    session_id: work.session_id,
                    runtime_owner_id: runtime_owner_id.clone(),
                    status: "missing".to_owned(),
                    failure_reason: None,
                }),
                PendingControlPlaneAction::Event(RuntimeEventFact {
                    session_id: work.session_id,
                    runtime_owner_id,
                    round_id: None,
                    source: "worker".to_owned(),
                    ts_ms: now_ms(),
                    kind: RuntimeFactKind::WorkerRuntimeMissing,
                }),
            ])
            .await
        }
    }

    async fn flush_pending_actions(&mut self) -> Result<()> {
        loop {
            let Some(action) = self.pending_actions.front().cloned() else {
                return Ok(());
            };
            self.send_action(action).await?;
            self.pending_actions.pop_front();
        }
    }

    async fn dispatch_actions_or_queue(
        &mut self,
        actions: Vec<PendingControlPlaneAction>,
    ) -> Result<()> {
        let mut pending = actions.into_iter();
        while let Some(action) = pending.next() {
            match self.send_action(action.clone()).await {
                Ok(()) => {}
                Err(error) if is_retryable_control_plane_error(&error) => {
                    self.pending_actions.push_back(action);
                    self.pending_actions.extend(pending);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    async fn send_action(&self, action: PendingControlPlaneAction) -> Result<()> {
        match action {
            PendingControlPlaneAction::Status(request) => self.client.report_status(request).await,
            PendingControlPlaneAction::Event(fact) => self.client.publish_event(fact).await,
        }
    }
}

struct ControlPlaneRetryBackoff {
    initial_ms: u64,
    max_ms: u64,
    current_ms: u64,
    consecutive_failures: u32,
}

impl ControlPlaneRetryBackoff {
    fn new(initial_ms: u64, max_ms: u64) -> Self {
        Self {
            initial_ms,
            max_ms,
            current_ms: initial_ms,
            consecutive_failures: 0,
        }
    }

    fn reset(&mut self) {
        self.current_ms = self.initial_ms;
        self.consecutive_failures = 0;
    }

    fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    fn next_delay_ms(&mut self) -> u64 {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let jitter_cap = self.current_ms / 4;
        let jitter = if jitter_cap == 0 {
            0
        } else {
            now_ms().unsigned_abs() % (jitter_cap + 1)
        };
        let delay_ms = self.current_ms.saturating_add(jitter);
        self.current_ms = self.current_ms.saturating_mul(2).min(self.max_ms);
        delay_ms
    }
}

fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(error) => -(error.duration().as_millis() as i64),
    }
}
