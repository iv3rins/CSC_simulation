//! 决策桥接：Human 策略 ChannelDecisionSource + 模拟线程命令 + 推进模板。

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use csc_core::client::ClientState;
use csc_core::engine::Engine;
use csc_core::state::{ExecutionState, GameState, StepKind};
use csc_decision::point::{DecisionPoint, PlayerDecision};
use csc_decision::source::{AutoDecisionSource, DecisionSource};
use csc_events::event::WorldEvent;
use csc_events::projection::{ProjectionContext, projected_json};
use tokio::sync::broadcast;

use super::types::*;
/// 模拟线程命令（内部；大载荷装箱避免 enum 变体尺寸失衡）。
pub(super) enum Command {
    Career {
        command: super::career::CareerCommand,
        reply: mpsc::Sender<Result<serde_json::Value, String>>,
    },
    Advance {
        months: u32,
        reply: mpsc::Sender<Result<Vec<StepSummary>, String>>,
    },
    /// FIFA 式赛季推进：一直模拟到下一个赛季边界（赛季内用稳妥默认决策，
    /// 不向玩家弹事件决策；换队/BP/发言都在赛季间主动完成）。
    AdvanceSeason {
        reply: mpsc::Sender<Result<Vec<StepSummary>, String>>,
    },
    AdvanceDays {
        days: u32,
        reply: mpsc::Sender<Result<Vec<StepSummary>, String>>,
    },
    State {
        reply: mpsc::Sender<GameState>,
    },
    Journal {
        since: i32,
        reply: mpsc::Sender<Vec<WorldEvent>>,
    },
    Load {
        state: Box<GameState>,
        reply: mpsc::Sender<Result<(), String>>,
    },
    Train {
        focus: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
    SetSeasonGoal {
        goal: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// 资金运用（2026 复审新增）：BUYOUT / INVEST / SKIN —— 服务端外部操作，
    /// 引擎内实现全部确定性（投资/选品哈希派生；买断补位走确定性 RNG）。
    Finance {
        action: String,
        param: Option<String>,
        reply: mpsc::Sender<Result<serde_json::Value, String>>,
    },
    /// 主动转会：查询当前可签约候选。
    TransferOffers {
        reply: mpsc::Sender<Result<Vec<csc_decision::offer::TransferOffer>, String>>,
    },
    /// 主动转会：选择目标队立即签约（不再等转会窗决策事件）。
    SignTransfer {
        team_signature: String,
        reply: mpsc::Sender<Result<serde_json::Value, String>>,
    },
    /// 主动发言：CONFIDENT / HUMBLE / PROVOKE。
    Statement {
        tone: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    /// 主动赛前 BP / 战术预案：AGGRESSIVE / BALANCED / CONSERVATIVE。
    MatchPlan {
        style: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// 跳过今日比赛日 LIVE 提示：把主角今日 pending 对阵标记 skipped（持久语义，
    /// 不改结算），返回被标记数量——解除「跳过后浮层重现」闭环。
    SkipLiveToday {
        reply: mpsc::Sender<Result<usize, String>>,
    },
    /// R2：标记某赛事对局已看完（watched 持久语义，幂等），返回是否命中。
    MarkWatched {
        event_name: String,
        fixture_id: u32,
        reply: mpsc::Sender<Result<bool, String>>,
    },
    Shutdown,
}

/// 推进粒度（`run_advance_steps` 的日/月差异点）。
pub(super) enum AdvanceStep {
    Month,
    Day,
}

/// 模拟线程内连续推进 `count` 步（月步或日步共用同一模板：快照前后计数 →
/// catch_unwind → StepSummary 构造 → WS 广播）。**Advance/AdvanceDays 唯一实现**
/// ——历史上两 handler 各自复制约 40 行，修一处漏一处（结构拆分收敛）。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_advance_steps(
    engine: &mut Engine,
    source: &mut dyn DecisionSource,
    id: u64,
    policy: Policy,
    sim_events: &tokio::sync::broadcast::Sender<WsEvent>,
    sim_view: &std::sync::Mutex<ClientState>,
    journal_rx: &mpsc::Receiver<WorldEvent>,
    count: u32,
    step_kind: AdvanceStep,
) -> Result<Vec<StepSummary>, String> {
    let mut summaries: Vec<StepSummary> = Vec::new();
    for _ in 0..count {
        let journal_before = engine.journal().len();
        let decisions_before = engine.decision_log().count();
        let step = std::panic::catch_unwind(AssertUnwindSafe(|| match step_kind {
            AdvanceStep::Month => engine.advance_month(source),
            AdvanceStep::Day => engine.advance_day(source),
        }));
        match step {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("决策被拒绝（协议错误）：{e}")),
            Err(_) => {
                return Err("模拟推进 panic（内部错误）——本局继续存活，可读档恢复".into());
            }
        }
        // P2-2：drain 决策源发来的提示事件（如决策超时自动继续）入 journal。
        // 时机在 step 成功之后、WS 广播之前——seq 盖章与既有事件流同序，
        // 超时提示随本步的 Journal/Step 事件一并推送，前端即时可见。
        for notice in journal_rx.try_iter() {
            engine.journal_mut().record(notice);
        }
        let month = engine.month();
        let (year, mon, day) = engine.clock().now();
        let events: Vec<WorldEvent> = engine.journal().since(journal_before as i32 - 1);
        let protagonist_id = engine
            .world()
            .players
            .iter()
            .find(|p| p.is_player())
            .map(|p| p.id);
        let protagonist_team_id = protagonist_id
            .and_then(|pid| engine.world().player(pid))
            .and_then(|pc| pc.team);
        let projection_context = ProjectionContext {
            protagonist_id,
            protagonist_team_id,
        };
        // WS carries both channels with their authoritative projection. This keeps the
        // cursor continuous and lets a connected client render either tab immediately.
        let projected_events = events
            .iter()
            .filter(|event| csc_events::projection::project(event, projection_context).visible)
            .filter_map(|event| projected_json(event, projection_context))
            .collect();
        let summary = StepSummary {
            game_id: id,
            month,
            date: format!("{year}-{mon:02}-{day:02}"),
            day: match step_kind {
                AdvanceStep::Month => None,
                AdvanceStep::Day => Some(day),
            },
            journal_added: engine.journal().len() - journal_before,
            decisions_made: engine.decision_log().count() - decisions_before,
            season_end: month > 0 && month % 12 == 0,
            season_index: (month + 11) / 12,
        };
        let _ = sim_events.send(WsEvent::Journal {
            events: projected_events,
            // 下一条事件序号（= 当前游标 + 1；与 REST `/journal?since=` 的 next_seq 同口径）。
            next_seq: engine.journal().cursor() + 1,
        });
        if policy == Policy::Human {
            *sim_view.lock().expect("view 锁") = engine.client_state();
        }
        let _ = sim_events.send(WsEvent::Step(summary.clone()));
        summaries.push(summary);
    }
    Ok(summaries)
}

/// 推进批次收尾（快照/视图刷新 + advancing 复位 + 应答）——Advance/AdvanceDays 共用。
pub(super) fn finish_advance(
    engine: &mut Engine,
    sim_view: &std::sync::Mutex<ClientState>,
    sim_snapshot: &std::sync::Mutex<GameState>,
    sim_advancing: &AtomicBool,
    reply: mpsc::Sender<Result<Vec<StepSummary>, String>>,
    result: Result<Vec<StepSummary>, String>,
) {
    let state = engine.snapshot();
    *sim_view.lock().expect("view 锁") = engine.client_state();
    *sim_snapshot.lock().expect("snapshot 锁") = state;
    sim_advancing.store(false, Ordering::SeqCst);
    let _ = reply.send(result);
}

/// Human 策略的决策源：decide() 阻塞在 channel 上（D3 桥接）。
///
/// 22 R1 修复：通道载荷为 [`DecisionSubmission`] 信封（携带 batch_id/request_id）。
/// 接收端在**状态拥有者**（本线程）处原子校验批次身份与提交全集，通过后才消费
/// 并回传权威结果；批次不匹配/校验失败的回传 `Err`，且**不消费** pending，
/// 提交方不会把旧选择送进新批次。
pub(super) struct ChannelDecisionSource {
    pub(super) decisions_rx: mpsc::Receiver<DecisionSubmission>,
    pub(super) events_tx: broadcast::Sender<WsEvent>,
    /// P2-2：决策超时提示事件的通道——decide() 不持 Engine，超时提示经此
    /// 送入模拟线程，由 `run_advance_steps` 每步结束后 drain 并
    /// `engine.journal_mut().record(...)`（seq 盖章与既有事件流同序）。
    pub(super) journal_tx: mpsc::SyncSender<WorldEvent>,
    pub(super) pending: Arc<Mutex<Option<PendingBatch>>>,
    pub(super) next_batch_id: u64,
    /// 决策等待超时：超时后按 Auto 默认继续（防断连死局）。
    /// 故事模式（23 §4.2）传 `None`：**关键选择持续等待**，不因读得慢/断网代选。
    pub(super) decision_timeout: Option<Duration>,
    /// 共享快照：发布/消费待决策时把 `execution` 写入快照，使**待决策时保存**能
    /// 捕获悬停点（22 R5：摆脱「待决策时请求存档排队/超时」）。
    pub(super) snapshot: Arc<Mutex<GameState>>,
    /// 是否故事模式（写入 `execution.story`）。
    pub(super) story: bool,
}

impl ChannelDecisionSource {
    /// 把「等待决策」写入共享快照（保存时捕获悬停点）。
    fn stamp_awaiting(&self, batch: &PendingBatch, remaining_steps: u32, step_kind: StepKind) {
        let mut snap = self.snapshot.lock().expect("snapshot 锁");
        snap.execution = ExecutionState::AwaitingDecision {
            batch_id: batch.batch_id,
            next_batch_id: self.next_batch_id,
            points: batch.points.clone(),
            armed_decisions: Vec::new(),
            date: batch.date.clone(),
            remaining_steps,
            step_kind,
            story: self.story,
            replay_current: true,
        };
    }

    /// 清除「等待决策」标记（决策已消费）。
    fn clear_awaiting(&self) {
        let mut snap = self.snapshot.lock().expect("snapshot 锁");
        snap.execution = ExecutionState::Idle;
    }
}

impl ChannelDecisionSource {
    /// 校验一份提交信封（批次身份 + 提交全集），返回决策副本；失败返回错误串。
    /// **纯值校验**，不消费 pending、不回传——调用方持有 reply 通道负责回传。
    fn validate_submission(
        batch: &PendingBatch,
        batch_id: u64,
        decisions: &[PlayerDecision],
    ) -> Result<(), String> {
        if batch_id != batch.batch_id {
            return Err(format!(
                "批次身份不匹配：提交针对 {batch_id}，当前待决策为 {}",
                batch.batch_id
            ));
        }
        csc_decision::validation::validate_submission(&batch.points, decisions)
            .map_err(|e| format!("决策校验失败：{e}"))
    }
}

impl DecisionSource for ChannelDecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> {
        self.next_batch_id += 1;
        let batch = PendingBatch {
            batch_id: self.next_batch_id,
            date: points.first().map(|p| p.date().to_string()),
            points: points.to_vec(),
        };
        *self.pending.lock().expect("pending 锁") = Some(batch.clone());
        // 22 R5：把「等待决策」写进共享快照，使**待决策时保存**能捕获悬停点。
        self.stamp_awaiting(&batch, 1, StepKind::Month);
        let _ = self.events_tx.send(WsEvent::Decisions(batch.clone()));

        // 超时兜底：按 Auto 权威默认继续（单一事实源）。
        let timeout_defaults =
            |journal_tx: &mpsc::SyncSender<WorldEvent>, events_tx: &broadcast::Sender<WsEvent>| {
                let count = points.len();
                let date = points
                    .first()
                    .map(|p| p.date().to_string())
                    .unwrap_or_default();
                let _ = journal_tx.try_send(WorldEvent::LiveUpdate {
                    date,
                    seq: -1,
                    headline: "决策超时".into(),
                    detail: format!("本批决策等待超时，已按默认策略继续推进（共 {count} 项决策）"),
                });
                let _ = events_tx.send(WsEvent::Notice {
                    message: format!("决策等待超时（共 {count} 项），已按默认策略自动继续"),
                });
                points
                    .iter()
                    .map(AutoDecisionSource::default_decision_for)
                    .collect::<Vec<_>>()
            };

        let got = match self.decision_timeout {
            // 故事模式（None）：**持续等待**，不因读得慢/断网代选。
            None => loop {
                match self.decisions_rx.recv() {
                    Ok(submission) => {
                        let DecisionSubmission {
                            request_id: _,
                            batch_id,
                            decisions,
                            reply,
                        } = submission;
                        match Self::validate_submission(&batch, batch_id, &decisions) {
                            Ok(()) => {
                                let _ = reply.send(Ok(()));
                                break decisions;
                            }
                            Err(msg) => {
                                let _ = reply.send(Err(msg));
                                continue;
                            }
                        }
                    }
                    Err(_) => break Vec::new(),
                }
            },
            // Human（Some）：有界超时，超时按 Auto 默认继续（防断连死局）。
            Some(timeout) => {
                let deadline = std::time::Instant::now() + timeout;
                loop {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        break timeout_defaults(&self.journal_tx, &self.events_tx);
                    }
                    match self.decisions_rx.recv_timeout(deadline - now) {
                        Ok(submission) => {
                            let DecisionSubmission {
                                request_id: _,
                                batch_id,
                                decisions,
                                reply,
                            } = submission;
                            match Self::validate_submission(&batch, batch_id, &decisions) {
                                Ok(()) => {
                                    let _ = reply.send(Ok(()));
                                    break decisions;
                                }
                                Err(msg) => {
                                    let _ = reply.send(Err(msg));
                                    continue;
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            break timeout_defaults(&self.journal_tx, &self.events_tx);
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break Vec::new(),
                    }
                }
            }
        };
        // pending 只能由模拟线程清理：若由提交方清理，提交 send 与本线程
        // 下一个 decide() 写 pending 之间存在竞态——新批次被清掉 → 提交方
        // 永远看不到 → 本线程在 recv 永久阻塞（协议死锁）。
        *self.pending.lock().expect("pending 锁") = None;
        // 决策已消费：清除悬停点标记。
        self.clear_awaiting();
        got
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_decision::point::DecisionPoint;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::broadcast;

    /// 最小合法 GameState（测试用；与 csc-core::state tests 同形状）。
    fn empty_state() -> GameState {
        GameState {
            version: GameState::CURRENT_VERSION,
            month: 0,
            last_contract_year: 2026,
            locks: vec![],
            sim_year: 2026,
            sim_month: 1,
            sim_day: 1,
            rng_state: [0, 0, 0, 0],
            decisions: vec![],
            journal: vec![],
            archive: std::collections::HashMap::new(),
            world: csc_entities::world::World::new(),
            vrs: csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: Default::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: csc_core::state::WORLD_SIM_VERSION,
            narrative: Default::default(),
            execution: csc_core::state::ExecutionState::Idle,
        }
    }

    /// 构造一份提交信封（带 reply 接收端）。
    fn submission(
        batch_id: u64,
        decisions: Vec<PlayerDecision>,
    ) -> (DecisionSubmission, mpsc::Receiver<Result<(), String>>) {
        let (tx, rx) = mpsc::channel();
        (
            DecisionSubmission {
                request_id: None,
                batch_id,
                decisions,
                reply: tx,
            },
            rx,
        )
    }

    #[test]
    fn human_channel_decision_source_blocks_then_resumes() {
        // 训练改为主动触发后，早期月份可能没有强制决策批次——直接验证
        // ChannelDecisionSource 的「阻塞等待 → pending 发布 → 回传消费」协议。
        let (tx, rx) = mpsc::sync_channel(1);
        let (events_tx, _) = broadcast::channel(4);
        let (journal_tx, _journal_rx) = mpsc::sync_channel::<WorldEvent>(16);
        let pending = Arc::new(Mutex::new(None));
        let mut source = ChannelDecisionSource {
            decisions_rx: rx,
            events_tx,
            journal_tx,
            pending: pending.clone(),
            next_batch_id: 0,
            decision_timeout: Some(DECISION_TIMEOUT),
            snapshot: Arc::new(Mutex::new(empty_state())),
            story: false,
        };
        let points = vec![DecisionPoint::TrainingFocus {
            id: "2026-02-01|train|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: csc_util::id::PlayerId(0),
            player_name: "MyPlayer".into(),
            options: vec![],
        }];
        let handle = std::thread::spawn(move || source.decide(&points));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let batch = pending
            .lock()
            .expect("pending 锁")
            .clone()
            .expect("decide 阻塞前应发布待决策批次");
        assert_eq!(batch.points.len(), 1);
        let (sub, reply_rx) = submission(
            batch.batch_id,
            vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM")],
        );
        tx.send(sub).expect("决策回传");
        assert_eq!(
            reply_rx.recv().expect("回传确认"),
            Ok(()),
            "合法提交应回传 Ok"
        );
        let decisions = handle.join().expect("决策线程完成");
        assert_eq!(decisions.len(), 1);
        assert!(
            pending.lock().expect("pending 锁").is_none(),
            "消费后 pending 由模拟线程清空"
        );
    }

    /// 22 R1 回归：携带**过期批次号**的提交必须被拒且不消费 pending；
    /// 随后携带**正确批次号**的合法提交必须成功。证明批次身份在通道内
    /// 被状态拥有者重新校验，双击/并发重试无法把旧选择送进新批次。
    #[test]
    fn stale_batch_submission_rejected_then_valid_accepted() {
        let (tx, rx) = mpsc::sync_channel(4);
        let (events_tx, _) = broadcast::channel(4);
        let (journal_tx, _journal_rx) = mpsc::sync_channel::<WorldEvent>(16);
        let pending = Arc::new(Mutex::new(None));
        let mut source = ChannelDecisionSource {
            decisions_rx: rx,
            events_tx,
            journal_tx,
            pending: pending.clone(),
            next_batch_id: 0,
            decision_timeout: Some(DECISION_TIMEOUT),
            snapshot: Arc::new(Mutex::new(empty_state())),
            story: false,
        };
        let points = vec![DecisionPoint::TrainingFocus {
            id: "2026-02-01|train|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: csc_util::id::PlayerId(0),
            player_name: "MyPlayer".into(),
            options: vec![],
        }];
        let handle = std::thread::spawn(move || source.decide(&points));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let batch_id = pending
            .lock()
            .expect("pending 锁")
            .clone()
            .expect("批次已发布")
            .batch_id;

        // ① 过期批次号（batch_id - 1）→ 拒绝。
        let (stale, stale_rx) = submission(
            batch_id.saturating_sub(1),
            vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM")],
        );
        tx.send(stale).expect("投递过期批次");
        let stale_result = stale_rx.recv().expect("过期批次应回传");
        assert!(stale_result.is_err(), "过期批次必须被拒绝");
        assert!(
            pending.lock().expect("pending 锁").is_some(),
            "拒绝后 pending 必须保留"
        );
        // ② 正确批次号 + 合法选项 → 接受。
        let (valid, valid_rx) = submission(
            batch_id,
            vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM")],
        );
        tx.send(valid).expect("投递合法批次");
        assert_eq!(valid_rx.recv().expect("合法批次回传"), Ok(()));
        let decisions = handle.join().expect("决策线程完成");
        assert_eq!(decisions.len(), 1, "合法提交应生效");
    }

    /// 22 R1 回归：正确批次号但**候选外选项**必须被拒且不消费 pending。
    #[test]
    fn invalid_option_submission_rejected_preserves_pending() {
        let (tx, rx) = mpsc::sync_channel(4);
        let (events_tx, _) = broadcast::channel(4);
        let (journal_tx, _journal_rx) = mpsc::sync_channel::<WorldEvent>(16);
        let pending = Arc::new(Mutex::new(None));
        let mut source = ChannelDecisionSource {
            decisions_rx: rx,
            events_tx,
            journal_tx,
            pending: pending.clone(),
            next_batch_id: 0,
            decision_timeout: Some(DECISION_TIMEOUT),
            snapshot: Arc::new(Mutex::new(empty_state())),
            story: false,
        };
        let points = vec![DecisionPoint::TrainingFocus {
            id: "2026-02-01|train|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: csc_util::id::PlayerId(0),
            player_name: "MyPlayer".into(),
            options: vec![],
        }];
        let handle = std::thread::spawn(move || source.decide(&points));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let batch_id = pending
            .lock()
            .expect("pending 锁")
            .clone()
            .unwrap()
            .batch_id;
        // 空提交（漏项）→ 拒绝。
        let (empty, empty_rx) = submission(batch_id, vec![]);
        tx.send(empty).expect("投递空提交");
        assert!(
            empty_rx.recv().expect("空提交应回传").is_err(),
            "空提交应被拒"
        );
        assert!(
            pending.lock().expect("pending 锁").is_some(),
            "拒绝后 pending 保留"
        );
        // 非法选项 → 拒绝。
        let (bad, bad_rx) = submission(
            batch_id,
            vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "NUCLEAR")],
        );
        tx.send(bad).expect("投递非法选项");
        assert!(bad_rx.recv().expect("非法选项应回传").is_err());
        assert!(pending.lock().expect("pending 锁").is_some());
        // 合法提交收尾。
        let (ok, ok_rx) = submission(
            batch_id,
            vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM")],
        );
        tx.send(ok).expect("投递合法");
        assert_eq!(ok_rx.recv().expect("合法回传"), Ok(()));
        let _ = handle.join();
    }

    #[test]
    fn channel_decision_source_times_out_to_auto_defaults() {
        // 断连兜底：sync_channel 留空不发送 → decide() 超时后按 Auto 默认决策继续，
        // 不永久阻塞（S2 决策超时）。
        let (tx, rx) = mpsc::sync_channel(1);
        let (events_tx, mut events_rx) = broadcast::channel(4);
        let (journal_tx, journal_rx) = mpsc::sync_channel::<WorldEvent>(16);
        let pending = Arc::new(Mutex::new(None));
        let mut source = ChannelDecisionSource {
            decisions_rx: rx,
            events_tx,
            journal_tx,
            pending: pending.clone(),
            next_batch_id: 0,
            decision_timeout: Some(Duration::from_millis(50)),
            snapshot: Arc::new(Mutex::new(empty_state())),
            story: false,
        };
        let points = vec![
            DecisionPoint::TrainingFocus {
                id: "2026-02-01|train|MyPlayer".into(),
                date: "2026-02-01".into(),
                player_id: csc_util::id::PlayerId(0),
                player_name: "MyPlayer".into(),
                options: vec![],
            },
            DecisionPoint::MatchIntervention {
                id: "2026-02-01|match|MyPlayer".into(),
                date: "2026-02-01".into(),
                player_id: csc_util::id::PlayerId(0),
                player_name: "MyPlayer".into(),
                event_name: "BLAST Bounty".into(),
                map_number: 1,
                series_score: "0-0".into(),
                options: vec![],
            },
        ];
        // 保持发送端存活但不发送：decide 只能走超时分支（50ms）。
        let _keep_tx = tx;
        let decisions = source.decide(&points);
        assert_eq!(
            decisions.len(),
            points.len(),
            "超时后应返回逐点 Auto 默认决策"
        );
        let expected: Vec<PlayerDecision> = points
            .iter()
            .map(AutoDecisionSource::default_decision_for)
            .collect();
        assert_eq!(
            decisions, expected,
            "超时分支语义应等于 AutoDecisionSource::default_decision_for"
        );
        // P2-2：超时分支应产出 1 条 journal 提示（LiveUpdate）+ 1 条 WS Notice。
        let notices: Vec<WorldEvent> = journal_rx.try_iter().collect();
        assert_eq!(notices.len(), 1, "超时应产生 1 条 journal 提示事件");
        match &notices[0] {
            WorldEvent::LiveUpdate {
                headline,
                detail,
                seq,
                ..
            } => {
                assert_eq!(headline, "决策超时");
                assert!(detail.contains("2 项决策"), "detail 应含决策项数：{detail}");
                assert_eq!(*seq, -1, "提示事件 seq=-1 由 journal record 盖章");
            }
            other => panic!("超时提示应为 LiveUpdate，实际 {other:?}"),
        }
        // WS Notice 事件（前端 toast 通道）。
        let mut notice_count = 0;
        while let Ok(ev) = events_rx.try_recv() {
            if matches!(ev, WsEvent::Notice { .. }) {
                notice_count += 1;
            }
        }
        assert_eq!(notice_count, 1, "超时应广播 1 条 WS Notice");
        assert!(
            pending.lock().expect("pending 锁").is_none(),
            "超时返回后 pending 由模拟线程清空"
        );
    }

    #[test]
    fn channel_decision_source_disconnected_returns_empty() {
        // 服务端断开（通道关闭）→ decide 返回空 vec（unwrap_or_default 语义保留）。
        let (tx, rx) = mpsc::sync_channel(1);
        let (events_tx, _) = broadcast::channel(4);
        let (journal_tx, _journal_rx) = mpsc::sync_channel::<WorldEvent>(16);
        let pending = Arc::new(Mutex::new(None));
        let mut source = ChannelDecisionSource {
            decisions_rx: rx,
            events_tx,
            journal_tx,
            pending: pending.clone(),
            next_batch_id: 0,
            decision_timeout: Some(Duration::from_secs(300)),
            snapshot: Arc::new(Mutex::new(empty_state())),
            story: false,
        };
        let points = vec![DecisionPoint::TrainingFocus {
            id: "2026-02-01|train|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: csc_util::id::PlayerId(0),
            player_name: "MyPlayer".into(),
            options: vec![],
        }];
        drop(tx);
        let decisions = source.decide(&points);
        assert!(decisions.is_empty(), "通道断开应返回空决策列表");
    }
}
