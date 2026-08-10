use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use rtc::livekit::{output::BotAudioOutputSink, pcm::PcmFrame};
use tokio::sync::{mpsc, oneshot};

/// 真实播出结束后再多等的余量:NativeAudioSource 以实时速率拉帧,起播+累计时长是估算的
/// 播出结束时刻,余量覆盖 10ms 拉帧粒度与时钟抖动,确保门控切回 listening 前尾音已发完。
const PLAYOUT_TAIL_MARGIN: Duration = Duration::from_millis(100);
/// FlushBarrier 等待播出期间的轮询粒度:每隔此时长检查 epoch,被打断(barge-in)即刻停等。
const PLAYOUT_POLL_STEP: Duration = Duration::from_millis(20);

enum MixerMessage {
    Frame {
        epoch: u64,
        frame: PcmFrame,
    },
    FlushBarrier {
        epoch: u64,
        reply: oneshot::Sender<Result<()>>,
    },
    Close {
        reply: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone)]
pub struct BotAudioMixer {
    sink: BotAudioOutputSink,
    tx: mpsc::UnboundedSender<MixerMessage>,
    playback_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl BotAudioMixer {
    pub fn start(sink: BotAudioOutputSink) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let playback_epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        tokio::spawn(run_mixer(sink.clone(), rx, playback_epoch.clone()));
        Self {
            sink,
            tx,
            playback_epoch,
        }
    }

    pub async fn enqueue_speech_frame(&self, frame: PcmFrame) -> Result<()> {
        self.enqueue_frame(frame)
    }

    pub async fn enqueue_pre_recorded_frame(&self, frame: PcmFrame) -> Result<()> {
        self.enqueue_frame(frame)
    }

    fn enqueue_frame(&self, frame: PcmFrame) -> Result<()> {
        let epoch = self.current_epoch();
        self.tx
            .send(MixerMessage::Frame { epoch, frame })
            .map_err(|_| anyhow!("bot audio mixer closed"))
    }

    pub fn interrupt_speech(&self) {
        self.playback_epoch
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.sink.clear_buffer();
    }

    pub fn clear_output_buffer(&self) {
        self.sink.clear_buffer();
    }

    pub fn current_epoch(&self) -> u64 {
        self.playback_epoch
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn is_current_epoch(&self, epoch: u64) -> bool {
        self.current_epoch() == epoch
    }

    pub async fn close(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(MixerMessage::Close { reply: reply_tx })
            .map_err(|_| anyhow!("bot audio mixer closed"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("bot audio mixer close reply dropped"))?
    }

    pub async fn wait_until_drained(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let epoch = self.current_epoch();
        self.tx
            .send(MixerMessage::FlushBarrier {
                epoch,
                reply: reply_tx,
            })
            .map_err(|_| anyhow!("bot audio mixer closed"))?;
        reply_rx
            .await
            .map_err(|_| anyhow!("bot audio mixer flush reply dropped"))?
    }
}

async fn run_mixer(
    sink: BotAudioOutputSink,
    mut rx: mpsc::UnboundedReceiver<MixerMessage>,
    playback_epoch: Arc<std::sync::atomic::AtomicU64>,
) {
    // 当前播放段的预计播出结束时刻(随每帧推进)。capture_frame 返回只代表帧已入
    // NativeAudioSource 的 ~queue_size_ms 缓冲、未播出;据此估算源真正放完的时刻,
    // 供 FlushBarrier 等到尾音播完再放行,避免门控过早切回 listening 把尾音清掉。
    let mut segment_epoch: Option<u64> = None;
    let mut playout_end: Option<Instant> = None;
    while let Some(message) = rx.recv().await {
        match message {
            MixerMessage::Frame { epoch, frame } => {
                if playback_epoch.load(std::sync::atomic::Ordering::SeqCst) != epoch {
                    continue;
                }

                // 推进播出结束估算:源以实时拉帧,连续喂帧时 end 累加帧时长;若中间有
                // 空档(now 已超过上次 end,源放过静音),则从 now 重新起算。
                let now = Instant::now();
                if segment_epoch != Some(epoch) {
                    segment_epoch = Some(epoch);
                    playout_end = None;
                }
                let frame_duration = if frame.sample_rate > 0 {
                    Duration::from_secs_f64(
                        frame.samples_per_channel as f64 / frame.sample_rate as f64,
                    )
                } else {
                    Duration::ZERO
                };
                let base = playout_end.unwrap_or(now).max(now);
                playout_end = Some(base + frame_duration);

                // NativeAudioSource(buffered 模式)的 capture_frame 自带实时背压,
                // 会阻塞到缓冲被实时消费。mixer 不再自行做节奏控制,直接喂帧靠背压限速;
                // 叠加额外限速会使喂帧慢于实时,导致输出缓冲 underrun(电流音/爆音)。
                if let Err(error) = sink.write_pcm_frame(&frame).await {
                    tracing::warn!(error = %error, "worker mixer failed to write bot audio frame");
                }
            }
            MixerMessage::FlushBarrier { epoch, reply } => {
                // 仅当本段未被打断(epoch 匹配)才等真实播出结束;期间被 barge-in 打断
                // (epoch 变)立即停等,保证打断响应。打断后尾音被清属预期,不再等待。
                if playback_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch
                    && let Some(end) = playout_end
                {
                    let target = end + PLAYOUT_TAIL_MARGIN;
                    while playback_epoch.load(std::sync::atomic::Ordering::SeqCst) == epoch {
                        let now = Instant::now();
                        if now >= target {
                            break;
                        }
                        tokio::time::sleep((target - now).min(PLAYOUT_POLL_STEP)).await;
                    }
                }
                segment_epoch = None;
                playout_end = None;
                let _ = reply.send(Ok(()));
            }
            MixerMessage::Close { reply } => {
                let result = sink.close().await.context("failed to close bot audio sink");
                let _ = reply.send(result);
                return;
            }
        }
    }
}
