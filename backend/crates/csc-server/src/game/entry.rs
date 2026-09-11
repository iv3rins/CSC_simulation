//! 单局句柄：GameEntry（actor 客户端）+ GameHandle 租约 + SaveGuard/RestoreInProgress。

use std::collections::HashSet;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use csc_core::client::ClientState;
use csc_core::state::GameState;
use csc_decision::point::PlayerDecision;
use csc_events::event::WorldEvent;
use tokio::sync::broadcast;

use super::decision::Command;
use super::persist::transfer_market_from_pending;
use super::types::*;
/// 一局游戏的服务端句柄。
pub struct GameEntry {
    pub id: u64,
    pub policy: Policy,
    /// 是否故事模式（关键选择持续等待；写入 `execution.story` 与持久化包）。
    pub story: bool,
    pub career_session: Arc<Mutex<super::career::CareerSession>>,
    pub(super) cmd_tx: mpsc::Sender<Command>,
    /// Human 策略的决策回传通道（Auto 为 None；有界缓冲 FIFO，见 create 注释）。
    /// 22 R1：通道载荷为携带 batch_id/request_id 的 [`DecisionSubmission`] 信封，
    /// 接收端（ChannelDecisionSource）在状态拥有者处再校验批次身份并返回 receipt。
    pub decisions_tx: Option<mpsc::SyncSender<DecisionSubmission>>,
    /// request_id 幂等缓存（同 ID 同载荷返回原确认；异载荷拒绝）。
    pub receipts: Arc<Mutex<ReceiptCache>>,
    /// 服务端 → 客户端事件广播（WS 订阅源）
    pub events_tx: broadcast::Sender<WsEvent>,
    /// 最新完整快照（纯值；每个 advance 批次完成后更新——**不再逐月 clone 全量存档**）
    pub snapshot: Arc<Mutex<GameState>>,
    /// 最新客户端轻量视图（纯值；`GET /games/{id}/view` 数据源）。
    /// Human 策略下逐月更新（step 事件前就位），Auto 策略只在批次结束时更新。
    pub view: Arc<Mutex<ClientState>>,
    /// 当前待决策批次（Human；REST 拉取入口）
    pub pending: Arc<Mutex<Option<PendingBatch>>>,
    /// 推进中标志（防并发推进；Human 等待决策期间保持 true）
    pub advancing: Arc<AtomicBool>,
    /// 存档任务进行中标志（防连续点击触发多个大型序列化/压缩任务）
    pub saving: Arc<AtomicBool>,
    /// 请求租约计数：>0 表示某 HTTP 请求正持有该局句柄，容量治理不得卸载。
    pub leased: AtomicUsize,
    /// 正在执行 gzip 落盘（容量淘汰写盘）：防止两个淘汰任务写同一 tmp 路径。
    pub persisting: AtomicBool,
}

/// 一次 HTTP 请求持有的游戏句柄：Deref 到 GameEntry，Drop 时释放租约。
///
/// 容量治理只淘汰 `leased == 0` 的局；`game_of` 取到 Arc 后、返回给路由前
/// 在 games 锁内登记租约。若另一并发请求刚好把该局卸载，`game_of` 会重试
/// `get_or_restore`，因此路由永远拿不到「已退出的模拟线程」。
pub struct GameHandle {
    pub(super) entry: Arc<GameEntry>,
}

impl GameHandle {
    /// 需要跨线程移动的闭包/任务请用 `arc()` 拿共享 Arc（租约仍由本句柄持有）。
    pub fn arc(&self) -> Arc<GameEntry> {
        self.entry.clone()
    }
}

impl Drop for GameHandle {
    fn drop(&mut self) {
        self.entry.leased.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Deref for GameHandle {
    type Target = GameEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl GameEntry {
    pub fn career_command(
        &self,
        command: super::career::CareerCommand,
    ) -> Result<serde_json::Value, String> {
        if !self.story {
            return Err("此存档未启用故事模式".into());
        }
        if self.advancing.load(Ordering::SeqCst) {
            return Err("世界仍在推进，请稍后再试".into());
        }
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Career { command, reply })
            .map_err(|_| "模拟线程已退出")?;
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "提交仍待确认，请使用相同 request_id 重试")?
    }
    /// 发送推进命令（fire-and-forget 形态；结果经 Step 事件/快照观察）。
    pub fn request_advance(&self, months: u32) -> Result<(), String> {
        if self.story {
            return Err("故事世界推进尚未完成，不能使用旧推进接口".into());
        }
        if self.advancing.swap(true, Ordering::SeqCst) {
            return Err("本局正在推进中（或等待决策）".into());
        }
        let (reply, _rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Advance { months, reply })
            .map_err(|_| {
                // 模拟线程已退出：必须复位 advancing，否则本局永久卡在"推进中"，
                // 后续 advance/decide 全部 409 且 LRU 不淘汰（同 advance_blocking 的复位语义）。
                self.advancing.store(false, Ordering::SeqCst);
                "模拟线程已退出".to_string()
            })
    }

    /// 发送按天推进命令（Human；结果经 Step 事件/快照观察）。
    pub fn request_advance_days(&self, days: u32) -> Result<(), String> {
        if self.story {
            return Err("故事世界推进尚未完成，不能使用旧推进接口".into());
        }
        if self.advancing.swap(true, Ordering::SeqCst) {
            return Err("本局正在推进中（或等待决策）".into());
        }
        let (reply, _rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::AdvanceDays { days, reply })
            .map_err(|_| {
                self.advancing.store(false, Ordering::SeqCst);
                "模拟线程已退出".to_string()
            })
    }

    /// 同步推进（Auto 策略：等待完成返回摘要列表）。
    pub fn advance_blocking(
        &self,
        months: u32,
        timeout: std::time::Duration,
    ) -> Result<Vec<StepSummary>, String> {
        if self.advancing.swap(true, Ordering::SeqCst) {
            return Err("本局正在推进中（或等待决策）".into());
        }
        let (reply, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(Command::Advance { months, reply })
            .is_err()
        {
            self.advancing.store(false, Ordering::SeqCst);
            return Err("模拟线程已退出".into());
        }
        let result = rx
            .recv_timeout(timeout)
            .map_err(|_| "推进超时（模拟线程可能阻塞在决策等待）".to_string());
        self.advancing.store(false, Ordering::SeqCst);
        result?
    }

    /// FIFA 式赛季推进：从当前月模拟到下一个赛季边界。
    ///
    /// - 当前处于赛季中途 → 模拟到本赛季结束（12 - month%12 个月）；
    /// - 当前正好在赛季末 → 模拟完整下个赛季（12 个月）。
    ///
    /// 赛季内使用 [`CareerDecisionSource`]：不弹转会/伤病/代言/人生事件决策，
    /// 全部采用稳妥默认；玩家的换队/BP/发言在赛季间通过主动接口完成。
    pub fn advance_season_blocking(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Vec<StepSummary>, String> {
        if self.advancing.swap(true, Ordering::SeqCst) {
            return Err("本局正在推进中（或等待决策）".into());
        }
        let (reply, rx) = mpsc::channel();
        if self.cmd_tx.send(Command::AdvanceSeason { reply }).is_err() {
            self.advancing.store(false, Ordering::SeqCst);
            return Err("模拟线程已退出".into());
        }
        let result = rx
            .recv_timeout(timeout)
            .map_err(|_| "赛季推进超时（模拟线程可能阻塞）".to_string());
        self.advancing.store(false, Ordering::SeqCst);
        result?
    }

    /// 同步按天推进（Auto 策略：逐日产出文字直播摘要）。
    pub fn advance_days_blocking(
        &self,
        days: u32,
        timeout: std::time::Duration,
    ) -> Result<Vec<StepSummary>, String> {
        if self.advancing.swap(true, Ordering::SeqCst) {
            return Err("本局正在推进中（或等待决策）".into());
        }
        let (reply, rx) = mpsc::channel();
        if self
            .cmd_tx
            .send(Command::AdvanceDays { days, reply })
            .is_err()
        {
            self.advancing.store(false, Ordering::SeqCst);
            return Err("模拟线程已退出".into());
        }
        let result = rx
            .recv_timeout(timeout)
            .map_err(|_| "推进超时（模拟线程可能阻塞在决策等待）".to_string());
        self.advancing.store(false, Ordering::SeqCst);
        result?
    }

    /// 尝试进入保存互斥；已在进行中则返回 false（调用方应返回 409）。
    pub fn try_begin_save(&self) -> bool {
        !self.saving.swap(true, Ordering::SeqCst)
    }

    /// 保存任务结束，释放互斥。
    pub fn end_save(&self) {
        self.saving.store(false, Ordering::SeqCst);
    }

    /// 取当前快照（阻塞等待模拟线程响应）。
    ///
    /// 20 年 128 队快照 clone 本身是重操作；10 局长档并发保存时，
    /// 每局都在各自模拟线程构建快照，CPU 争抢下可能超过 10s——
    /// 这里放宽到 120s，外层 HTTP 另有 300s 整体超时兜底。
    pub fn state(&self) -> Result<GameState, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::State { reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(120))
            .map_err(|_| "取状态超时（>120s）".to_string())
    }

    /// 排定主动训练计划（同步等待模拟线程响应；推进中/等待决策时应拒绝）。
    pub fn plan_training(&self, focus: String) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Train { focus, reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "排定训练超时".to_string())?
    }

    /// 设置本赛季主目标（同步等待模拟线程响应；写入存档 v6 字段）。
    pub fn set_season_goal(&self, goal: String) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::SetSeasonGoal { goal, reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "设置赛季目标超时".to_string())?
    }

    /// 资金运用（2026 复审新增）：买断自己换队 / 投资自己 / 购买 CS 饰品。
    /// @param action BUYOUT | INVEST | SKIN；param 供 INVEST 方向 / SKIN 稀有度。
    /// @return { cash, message, skin? }
    pub fn finance(
        &self,
        action: String,
        param: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Finance {
                action,
                param,
                reply,
            })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "资金运用超时".to_string())?
    }

    /// 增量事件（seq 之后）。
    pub fn journal_since(&self, since: i32) -> Result<Vec<WorldEvent>, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Journal { since, reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "取事件流超时".to_string())
    }

    /// 读档（完整 GameState）。
    pub fn load(&self, state: GameState) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Load {
                state: Box::new(state),
                reply,
            })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "读档超时".to_string())?
    }

    /// 主动转会：当前可签约候选（合同到期/买断后才有）。
    pub fn transfer_offers(&self) -> Result<Vec<csc_decision::offer::TransferOffer>, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::TransferOffers { reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "查询转会候选超时".to_string())?
    }

    /// 当前挂起的转会窗市场（去弹窗契约）：从 `pending` 批次提取主角的
    /// `DecisionPoint::TransferWindow`，返回报价清单 + 截至日期（报价月月末）。
    ///
    /// 无挂起报价（世界未停在转会窗 / 已提交 / auto 策略）→ `None`。
    pub fn transfer_market(&self) -> Option<serde_json::Value> {
        let pending = self.pending.lock().expect("pending 锁").clone();
        transfer_market_from_pending(&pending)
    }

    /// 玩家今日可 LIVE 的对阵（比赛日闸门查询）：读快照投影。
    pub fn live_today(&self) -> Vec<csc_core::client::LiveGateFixture> {
        let snap = self.snapshot.lock().expect("snapshot 锁");
        csc_core::client::player_live_fixtures(&snap)
    }

    /// 比赛日闸门是否激活（Human 策略下推进拦截判断）。
    ///
    /// 有主角可 LIVE 的 pending 对阵时返回 `true`——**任何粒度**的推进（日级/月级/
    /// 赛季）若继续都会把这些对阵越过/结算，导致 LIVE 消失。前端应在比赛日暂停 +
    /// 展示 20s LIVE 提示条（进入 LIVE / 跳过）；玩家先处理（进入 LIVE 或
    /// `POST /live/today/skip`）后闸门关闭，推进放行（P2：手动「下一天」不再绕过）。
    pub fn live_gate_active(&self) -> bool {
        !self.live_today().is_empty()
    }

    /// 主动签约目标队。
    pub fn sign_transfer(&self, team_signature: String) -> Result<serde_json::Value, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::SignTransfer {
                team_signature,
                reply,
            })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "签约处理超时".to_string())?
    }

    /// 主动发言。
    pub fn statement(&self, tone: String) -> Result<String, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::Statement { tone, reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "表态处理超时".to_string())?
    }

    /// 主动赛前 BP / 战术预案。
    pub fn set_match_plan(&self, style: String) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::MatchPlan { style, reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "设置赛前预案超时".to_string())?
    }

    /// 跳过今日比赛日 LIVE 提示：把主角今日 pending 对阵置 `skipped` 标记，
    /// 返回被标记的场次数。不消费 RNG、不改结算（月末照常确定性结算出比分）。
    pub fn skip_live_today(&self) -> Result<usize, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::SkipLiveToday { reply })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "跳过比赛日处理超时".to_string())?
    }

    /// R2：标记某赛事对局已看完（watched 持久语义，幂等）。返回是否命中。
    pub fn mark_watched(&self, event_name: String, fixture_id: u32) -> Result<bool, String> {
        let (reply, rx) = mpsc::channel();
        self.cmd_tx
            .send(Command::MarkWatched {
                event_name,
                fixture_id,
                reply,
            })
            .map_err(|_| "模拟线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "标记已看完处理超时".to_string())?
    }

    /// 提交决策（Human；唤醒阻塞中的 decide()）。
    ///
    /// 三件套（P0 G1，OpenTTD ReceiveClientCommand 模式）：
    /// a. `advancing` 前置校验——未在推进中 = 无待决策批次，立即拒绝（409 语义）；
    ///    用 `load` 不用 `swap`——advancing 的 set/reset 归模拟线程/request_advance 所有。
    /// b. `batch_id` 校验——比对当前 pending 批次号，比对完立即释放锁（不持锁 send）。
    /// c. `try_send` 防阻塞——Full → 429 语义（不钉死 tokio worker），Disconnected → 通道断开。
    ///
    /// **22 R1 修复**：实际投递的是携带 `batch_id/request_id` 的信封，由模拟线程内的
    /// [`ChannelDecisionSource`] 在**状态拥有者**处再校验批次身份并原子消费；本函数
    /// 只在预检后投递并等待权威结果。同 `request_id` 同载荷 → 返回原确认（幂等），
    /// 异载荷 → 拒绝；不能仅用按钮 disabled 代替后端幂等。
    pub fn submit_decisions(
        &self,
        request_id: Option<String>,
        decisions: Vec<PlayerDecision>,
        batch_id: Option<u64>,
    ) -> Result<(), String> {
        // a. advancing 前置校验：未在推进中 = 无待决策批次，立即拒绝（409 语义）。
        if !self.advancing.load(Ordering::SeqCst) {
            return Err("本局未在推进中，无待决策批次".into());
        }
        // b. batch_id 校验：比对当前 pending 批次号，比对完立即释放锁（不持锁 send）。
        let current = {
            let pending = self.pending.lock().expect("pending 锁");
            pending.as_ref().map(|p| p.batch_id)
        };
        if batch_id.is_none() || batch_id != current {
            return Err("批次已过期或不存在（请刷新决策面板后重试）".into());
        }
        let batch_id = batch_id.expect("已校验 Some");
        let fingerprint = submission_fingerprint(&decisions);
        // request_id 幂等：见过同 ID。
        if let Some(rid) = request_id.as_deref() {
            let cache = self.receipts.lock().expect("receipt 锁");
            if let Some(entry) = cache.get(rid) {
                if entry.fingerprint == fingerprint {
                    // 同 ID 同载荷 = 重试：返回原确认（不重复投递/应用）。
                    return Ok(());
                }
                return Err(format!(
                    "request_id「{rid}」已用于不同内容的提交——拒绝重放（请刷新后重试）"
                ));
            }
        }
        // c. try_send 防阻塞：Full → 429 语义，Disconnected → 通道断开。
        let tx = self
            .decisions_tx
            .as_ref()
            .ok_or("本局为 auto 策略，无决策通道")?;
        let (reply_tx, reply_rx) = mpsc::channel::<Result<(), String>>();
        let submission = DecisionSubmission {
            request_id: request_id.clone(),
            batch_id,
            decisions,
            reply: reply_tx,
        };
        tx.try_send(submission).map_err(|e| match e {
            mpsc::TrySendError::Full(_) => "决策通道已满（429 语义，请稍后重试）".to_string(),
            mpsc::TrySendError::Disconnected(_) => "决策通道已断开".to_string(),
        })?;
        // 等待状态拥有者的权威结果（校验失败不消费 pending；成功即接受）。
        match reply_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(())) => {
                if let Some(rid) = request_id {
                    self.receipts
                        .lock()
                        .expect("receipt 锁")
                        .insert(rid, fingerprint, batch_id);
                }
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("决策提交确认超时（模拟线程未在 10s 内响应）".into()),
        }
    }

    /// 关闭模拟线程（发 Shutdown 命令，线程自行退出；JoinHandle 丢弃即分离）。
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(Command::Shutdown);
    }
}

/// 保存/读档互斥的 RAII 守卫：请求取消（HTTP future 被 drop）也会释放
/// `saving` 原子位，避免一次断连把该局永久锁成 409。
pub struct SaveGuard(Arc<GameEntry>);

impl SaveGuard {
    /// 尝试进入保存互斥；已在进行中则返回 None（调用方应返回 409）。
    pub fn acquire(entry: &Arc<GameEntry>) -> Option<Self> {
        entry.try_begin_save().then(|| Self(entry.clone()))
    }
}

impl Drop for SaveGuard {
    fn drop(&mut self) {
        self.0.end_save();
    }
}

/// `get_or_restore` 的进行中标记；Drop 自动清除（含请求取消路径）。
pub(super) struct RestoreInProgress {
    pub(super) id: u64,
    pub(super) restoring: Arc<Mutex<HashSet<u64>>>,
}

impl Drop for RestoreInProgress {
    fn drop(&mut self) {
        self.restoring
            .lock()
            .expect("restoring 锁")
            .remove(&self.id);
    }
}
