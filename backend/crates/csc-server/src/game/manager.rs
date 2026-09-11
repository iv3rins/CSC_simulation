//! 游戏管理器：每局一个模拟线程，game_id 为索引（D1 多局隔离）+ 容量治理（LRU 淘汰/恢复）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use flate2::read::GzDecoder;

use csc_core::engine::Engine;
use csc_decision::source::{AutoDecisionSource, CareerDecisionSource, DecisionSource};
use csc_domain::season_goal::SeasonGoal;
use csc_events::event::WorldEvent;
use csc_time::clock::SimClock;
use tokio::sync::broadcast;

use super::decision::{
    AdvanceStep, ChannelDecisionSource, Command, finish_advance, run_advance_steps,
};
use super::entry::{GameEntry, GameHandle, RestoreInProgress};
use super::persist::{GameLoader, build_persisted_envelope, write_persisted_package};
use super::types::*;
use crate::live::LiveSessionStore;
/// 游戏管理器：每局一个模拟线程，`game_id` 为索引（D1 多局隔离）。
#[derive(Default)]
pub struct GameManager {
    games: Mutex<HashMap<u64, Arc<GameEntry>>>,
    next_id: AtomicU64,
    /// 正在从磁盘恢复的游戏 id（防止两个并发请求同时恢复同一局、把磁盘包消费两次）。
    pub(crate) restoring: Arc<Mutex<HashSet<u64>>>,
    /// 后台容量收缩进行中（避免健康检查重复排发长档 gzip 落盘任务）。
    rebalancing: AtomicBool,
    /// 空闲游戏持久化目录（None = 不启用 Evict/Resume；写入与主读取都走这里）。
    persist_dir: Option<PathBuf>,
    /// 兼容读回退目录（可选）：主目录找不到存档时尝试从这里恢复——
    /// 用于「persist 迁出资产树」后仍能读取历史写入 `assets/games` 或
    /// `backend/runtime/games` 的旧存档。**只读回退，不写。**
    persist_dir_fallback: Option<PathBuf>,
    /// 活跃游戏数上限（None = 不限制）。
    max_active: Option<usize>,
    /// 最近访问时间（LRU 依据）。
    last_access: Mutex<HashMap<u64, Instant>>,
    /// 恢复配置（至少需要 rating_profile；None = 不可恢复）。
    loader: Option<Arc<GameLoader>>,
    /// LIVE 会话仓库（可选，2026-09 修复 P0-2）：游戏被 LRU 淘汰 / shutdown 时
    /// 据此回收该 game_id 的全部 LIVE 会话，消除「会话只增不删」的进程级泄漏。
    /// None = 不接入（默认/测试/嵌入式路径），回收逻辑成为无操作。
    live_sessions: Option<LiveSessionStore>,
}

impl GameManager {
    /// 新建空管理器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 带容量治理配置的管理器（生产路径）。
    pub fn with_config(
        persist_dir: Option<PathBuf>,
        max_active: Option<usize>,
        loader: Option<Arc<GameLoader>>,
    ) -> Self {
        let next_id = persist_dir
            .as_ref()
            .and_then(|dir| fs::read_dir(dir).ok())
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                entry.ok().and_then(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .and_then(|name| name.strip_suffix(".game.gz"))
                        .and_then(|id| id.parse::<u64>().ok())
                })
            })
            .max()
            .map_or(0, |id| id.saturating_add(1));
        Self {
            next_id: AtomicU64::new(next_id),
            persist_dir,
            max_active,
            loader,
            ..Self::default()
        }
    }

    /// 设置兼容读回退目录（可选）：主 `persist_dir` 找不到存档时，
    /// `get_or_restore`/`restore_from_disk` 会回退到这里读取旧存档。
    /// 仅影响读路径，不影响写盘位置。
    pub fn with_persist_fallback(mut self, fallback: Option<PathBuf>) -> Self {
        self.persist_dir_fallback = fallback;
        self
    }

    /// 接入 LIVE 会话仓库（可选，2026-09 修复 P0-2）：LRU 淘汰与 shutdown 时
    /// 回收该 game_id 的全部 LIVE 会话。生产路径 `AppState::from_dir` 注入；
    /// 测试/默认路径为 None（不回收）。
    pub fn with_live_sessions(mut self, live_sessions: LiveSessionStore) -> Self {
        self.live_sessions = Some(live_sessions);
        self
    }

    /// 创建一局游戏：独占接管 `engine`，按策略装配决策源并启动模拟线程。
    /// 达到活跃上限时先把最久未访问的游戏持久化卸载（LRU）。
    /// 返回 `game_id`。
    pub fn create(self: &Arc<Self>, engine: Engine, policy: Policy) -> u64 {
        self.create_with_story(engine, policy, false)
    }

    /// 创建一局游戏（带故事模式标记）：`story=true` 时关键选择持续等待（不超时代选）。
    pub fn create_with_story(self: &Arc<Self>, engine: Engine, policy: Policy, story: bool) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.evict_lru_if_needed();
        let entry = self.spawn_entry(
            id,
            engine,
            policy,
            story,
            super::career::CareerSession::default(),
        );
        self.games.lock().expect("games 锁").insert(id, entry);
        self.touch(id);
        id
    }

    /// 启动一局游戏的模拟线程与共享状态（create / restore-from-disk 共用）。
    fn spawn_entry(
        self: &Arc<Self>,
        id: u64,
        mut engine: Engine,
        policy: Policy,
        story: bool,
        initial_session: super::career::CareerSession,
    ) -> Arc<GameEntry> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (events_tx, _) = broadcast::channel::<WsEvent>(WS_CHANNEL_CAP);
        let snapshot = Arc::new(Mutex::new(engine.snapshot()));
        let view = Arc::new(Mutex::new(engine.client_state()));
        let pending: Arc<Mutex<Option<PendingBatch>>> = Arc::new(Mutex::new(None));
        let receipts: Arc<Mutex<ReceiptCache>> = Arc::new(Mutex::new(ReceiptCache::default()));
        let advancing = Arc::new(AtomicBool::new(false));
        let saving = Arc::new(AtomicBool::new(false));
        let career_session = Arc::new(Mutex::new(initial_session));
        let sim_career_session = career_session.clone();
        let career_path = self.persist_path(id);
        let leased = AtomicUsize::new(0);
        let persisting = AtomicBool::new(false);

        let (decisions_tx, decisions_rx) = mpsc::sync_channel::<DecisionSubmission>(16);
        let decisions_tx_pub = policy.is_human().then_some(decisions_tx);
        // P2-2：决策源 → 模拟线程的 journal 提示通道（容量 16；`try_send` 满仅丢提示）。
        let (journal_tx, journal_rx) = mpsc::sync_channel::<WorldEvent>(16);

        let sim_events = events_tx.clone();
        let sim_snapshot = snapshot.clone();
        let sim_view = view.clone();
        let sim_pending = pending.clone();
        let sim_advancing = advancing.clone();
        let sim_journal_rx = journal_rx;

        let _handle = std::thread::spawn(move || {
            let mut source: Box<dyn DecisionSource> = match policy {
                Policy::Auto => Box::new(AutoDecisionSource),
                Policy::Human => Box::new(ChannelDecisionSource {
                    decisions_rx,
                    events_tx: sim_events.clone(),
                    journal_tx,
                    pending: sim_pending,
                    next_batch_id: 0,
                    // 故事模式（23 §4.2）：关键选择持续等待（None = 无超时代选）；
                    // 普通 Human 保留 5 分钟断连兜底。
                    decision_timeout: if story { None } else { Some(DECISION_TIMEOUT) },
                    snapshot: sim_snapshot.clone(),
                    story,
                }),
            };
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Command::Career { command, reply } => {
                        let mut session_guard =
                            sim_career_session.lock().expect("career session 锁");
                        let result = if story {
                            super::career::execute(
                                &mut engine,
                                &mut session_guard,
                                command,
                                career_path.as_deref(),
                                policy,
                            )
                        } else {
                            Err("此存档未启用故事模式".into())
                        };
                        if result.is_ok() {
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = engine.snapshot();
                        }
                        let _ = reply.send(result);
                    }
                    Command::Advance { months, reply } => {
                        let result = run_advance_steps(
                            &mut engine,
                            &mut *source,
                            id,
                            policy,
                            &sim_events,
                            &sim_view,
                            &sim_journal_rx,
                            months,
                            AdvanceStep::Month,
                        );
                        finish_advance(
                            &mut engine,
                            &sim_view,
                            &sim_snapshot,
                            &sim_advancing,
                            reply,
                            result,
                        );
                    }
                    Command::AdvanceDays { days, reply } => {
                        let result = run_advance_steps(
                            &mut engine,
                            &mut *source,
                            id,
                            policy,
                            &sim_events,
                            &sim_view,
                            &sim_journal_rx,
                            days,
                            AdvanceStep::Day,
                        );
                        finish_advance(
                            &mut engine,
                            &sim_view,
                            &sim_snapshot,
                            &sim_advancing,
                            reply,
                            result,
                        );
                    }
                    Command::AdvanceSeason { reply } => {
                        // 赛季边界：month % 12 == 0 表示刚完成一整个赛季。
                        let months = 12 - (engine.month() % 12);
                        let months = if months == 0 { 12 } else { months };
                        let mut season_source = CareerDecisionSource;
                        let result = run_advance_steps(
                            &mut engine,
                            &mut season_source,
                            id,
                            policy,
                            &sim_events,
                            &sim_view,
                            &sim_journal_rx,
                            months as u32,
                            AdvanceStep::Month,
                        );
                        finish_advance(
                            &mut engine,
                            &sim_view,
                            &sim_snapshot,
                            &sim_advancing,
                            reply,
                            result,
                        );
                    }
                    Command::State { reply } => {
                        let _ = reply.send(engine.snapshot());
                    }
                    Command::Journal { since, reply } => {
                        let _ = reply.send(engine.journal().since(since));
                    }
                    Command::Load { state, reply } => {
                        if story {
                            let mut session = sim_career_session.lock().expect("career session 锁");
                            let result = super::career::load_state(
                                &mut engine,
                                &mut session,
                                *state,
                                career_path.as_deref(),
                                policy,
                            );
                            if result.is_ok() {
                                *sim_view.lock().expect("view 锁") = engine.client_state();
                                *sim_snapshot.lock().expect("snapshot 锁") = engine.snapshot();
                            }
                            let _ = reply.send(result);
                            continue;
                        }
                        // P1-4：restore_result 构造-交换——坏档返回 Err（可预期业务错误走
                        // Result），引擎不被半恢复；不再用 catch_unwind 承接锁反解失败。
                        let result = engine
                            .restore_result(*state)
                            .map_err(|e| format!("读档失败：{e}"))
                            .map(|_| {
                                let state = engine.snapshot();
                                *sim_view.lock().expect("view 锁") = engine.client_state();
                                *sim_snapshot.lock().expect("snapshot 锁") = state;
                                *sim_career_session.lock().expect("career session 锁") =
                                    super::career::CareerSession::default();
                            });
                        let _ = reply.send(result);
                    }
                    Command::Train { focus, reply } => {
                        let result = (|| -> Result<(), String> {
                            let pid = engine.player().ok_or("尚无主角，无法排定训练计划")?;
                            engine
                                .plan_training(pid, &focus)
                                .map_err(|e| format!("训练计划被拒绝：{e}"))?;
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            Ok(())
                        })();
                        let _ = reply.send(result);
                    }
                    Command::SetSeasonGoal { goal, reply } => {
                        let result = (|| -> Result<(), String> {
                            let parsed = SeasonGoal::from_name(&goal)
                                .ok_or_else(|| format!("非法赛季目标：{goal}"))?;
                            let pid = engine.player().ok_or("尚无主角，无法设置赛季目标")?;
                            let year = engine.clock().year();
                            let pc = engine.world_mut().player_mut(pid).ok_or("主角不存在")?;
                            let career = pc.career_mut().ok_or("主角无生涯数据")?;
                            career.season_goal = Some(parsed);
                            career.season_goal_year = year;
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            Ok(())
                        })();
                        let _ = reply.send(result);
                    }
                    Command::Finance {
                        action,
                        param,
                        reply,
                    } => {
                        let result = (|| -> Result<serde_json::Value, String> {
                            let pid = engine.player().ok_or("尚无主角")?;
                            let year = engine.clock().year();
                            let date = engine.clock().date_label();
                            let cash_of = |e: &Engine| -> i64 {
                                e.world()
                                    .player(pid)
                                    .and_then(|p| p.career.as_ref())
                                    .map(|c| c.finance.cash)
                                    .unwrap_or(0)
                            };
                            let detail: String;
                            let mut skin: Option<csc_entities::career::SkinOwned> = None;
                            match action.as_str() {
                                "BUYOUT" => {
                                    let cost =
                                        engine.buyout_player(pid).map_err(|e| e.to_string())?;
                                    detail = format!(
                                        "你支付 {cost} 违约金买断了自己的合同，进入自由市场——下个转会窗可自由签约新队。"
                                    );
                                }
                                "INVEST" => {
                                    let focus = param
                                        .as_deref()
                                        .ok_or("投资需要方向参数（AIM/ENDURANCE/MENTAL）")?;
                                    detail = engine
                                        .invest_self(pid, focus, year)
                                        .map_err(|e| e.to_string())?;
                                }
                                "SKIN" => {
                                    let rare = param.as_deref() == Some("rare");
                                    let s = engine
                                        .buy_skin(pid, rare, year)
                                        .map_err(|e| e.to_string())?;
                                    detail = format!(
                                        "你购入 CS 饰品「{}」（{}），声誉 +{}",
                                        s.name,
                                        if rare { "稀有" } else { "标准" },
                                        if rare { 3 } else { 1 }
                                    );
                                    skin = Some(s);
                                }
                                other => {
                                    return Err(format!(
                                        "未知资金运用操作：{other}（BUYOUT/INVEST/SKIN）"
                                    ));
                                }
                            }
                            let message = detail.clone();
                            engine.journal_mut().record(WorldEvent::LiveUpdate {
                                date,
                                seq: -1,
                                headline: "资金运用".into(),
                                detail,
                            });
                            let cash = cash_of(&engine);
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            let mut out = serde_json::json!({ "cash": cash, "message": message });
                            if let Some(s) = skin {
                                out["skin"] = serde_json::to_value(s)
                                    .map_err(|e| format!("序列化失败：{e}"))?;
                            }
                            Ok(out)
                        })();
                        let _ = reply.send(result);
                    }
                    Command::TransferOffers { reply } => {
                        let result =
                            (|| -> Result<Vec<csc_decision::offer::TransferOffer>, String> {
                                let pid = engine.player().ok_or("尚无主角")?;
                                engine.transfer_offers(pid).map_err(|e| e.to_string())
                            })();
                        let _ = reply.send(result);
                    }
                    Command::SignTransfer {
                        team_signature,
                        reply,
                    } => {
                        let result = (|| -> Result<serde_json::Value, String> {
                            let pid = engine.player().ok_or("尚无主角")?;
                            let event = engine
                                .sign_transfer(pid, &team_signature)
                                .map_err(|e| e.to_string())?;
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            let message = format!(
                                "转会完成：{} → {}（{} 年，年薪 {}）",
                                event.from_team.as_deref().unwrap_or("自由身"),
                                event.to_team,
                                event.contract_years,
                                event.salary
                            );
                            let mut out = serde_json::json!({
                                "message": message,
                                "event": serde_json::to_value(&event).map_err(|e| format!("序列化失败：{e}"))?,
                            });
                            out["cash"] = serde_json::json!(
                                engine
                                    .world()
                                    .player(pid)
                                    .and_then(|p| p.career.as_ref())
                                    .map(|c| c.finance.cash)
                            );
                            Ok(out)
                        })();
                        let _ = reply.send(result);
                    }
                    Command::Statement { tone, reply } => {
                        let result = (|| -> Result<String, String> {
                            let pid = engine.player().ok_or("尚无主角")?;
                            let message = engine
                                .make_statement(pid, &tone)
                                .map_err(|e| e.to_string())?;
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            Ok(message)
                        })();
                        let _ = reply.send(result);
                    }
                    Command::MatchPlan { style, reply } => {
                        let result = (|| -> Result<(), String> {
                            let pid = engine.player().ok_or("尚无主角")?;
                            engine
                                .set_match_plan(pid, &style)
                                .map_err(|e| e.to_string())?;
                            let state = engine.snapshot();
                            *sim_view.lock().expect("view 锁") = engine.client_state();
                            *sim_snapshot.lock().expect("snapshot 锁") = state;
                            Ok(())
                        })();
                        let _ = reply.send(result);
                    }
                    Command::SkipLiveToday { reply } => {
                        let count = engine.skip_live_today();
                        let state = engine.snapshot();
                        *sim_view.lock().expect("view 锁") = engine.client_state();
                        *sim_snapshot.lock().expect("snapshot 锁") = state;
                        let _ = reply.send(Ok(count));
                    }
                    Command::MarkWatched {
                        event_name,
                        fixture_id,
                        reply,
                    } => {
                        let hit = engine.mark_fixture_watched(&event_name, fixture_id);
                        let state = engine.snapshot();
                        *sim_view.lock().expect("view 锁") = engine.client_state();
                        *sim_snapshot.lock().expect("snapshot 锁") = state;
                        let _ = reply.send(Ok(hit));
                    }
                    Command::Shutdown => break,
                }
            }
        });

        Arc::new(GameEntry {
            id,
            policy,
            story,
            career_session,
            cmd_tx,
            decisions_tx: decisions_tx_pub,
            receipts,
            events_tx,
            snapshot,
            view,
            pending,
            advancing,
            saving,
            leased,
            persisting,
        })
    }

    /// 取一局游戏（仅活跃内存）。
    pub fn get(&self, id: u64) -> Option<Arc<GameEntry>> {
        self.touch(id);
        self.games.lock().expect("games 锁").get(&id).cloned()
    }

    /// 取一局游戏；内存未命中时从磁盘持久化包恢复。
    pub async fn get_or_restore(self: &Arc<Self>, id: u64) -> Option<Arc<GameEntry>> {
        loop {
            if let Some(entry) = self.get(id) {
                return Some(entry);
            }
            // 同一局只允许一个恢复任务：并发请求看到标记后短暂等待，
            // 避免两个任务同时读取/消费同一个磁盘包（压测发现的 404 竞态）。
            let in_progress = {
                let mut restoring = self.restoring.lock().expect("restoring 锁");
                if !restoring.insert(id) {
                    None
                } else {
                    Some(RestoreInProgress {
                        id,
                        restoring: self.restoring.clone(),
                    })
                }
            };
            let Some(_in_progress) = in_progress else {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                continue;
            };
            // 不存在持久化包表示正常的内存未命中；避免把陈旧浏览器 game_id
            // 反复记录为恢复失败。
            if self.read_path(id).is_none_or(|path| !path.is_file()) {
                return None;
            }
            // 恢复前同样执行容量治理：腾出活跃名额再加载磁盘包。
            self.evict_lru_if_needed();
            let manager = self.clone();
            let result = tokio::task::spawn_blocking(move || manager.restore_from_disk(id)).await;
            drop(_in_progress);
            return match result {
                Ok(Ok(entry)) => Some(entry),
                Ok(Err(e)) => {
                    eprintln!("[GameManager] 游戏 {id} 恢复失败：{e}");
                    None
                }
                Err(e) => {
                    eprintln!("[GameManager] 游戏 {id} 恢复线程异常：{e}");
                    None
                }
            };
        }
    }

    /// 在 games 锁内登记租约并返回句柄；若该局在取 Arc 与登记之间被卸载则返回 None
    /// （调用方应重试 `get_or_restore`）。
    pub fn lease_entry(self: &Arc<Self>, id: u64) -> Option<GameHandle> {
        let games = self.games.lock().expect("games 锁");
        let entry = games.get(&id).cloned()?;
        entry.leased.fetch_add(1, Ordering::SeqCst);
        Some(GameHandle { entry })
    }

    /// 关闭并移除一局游戏；同时删除其磁盘持久化包与 LIVE 会话。
    pub fn shutdown(&self, id: u64) -> Option<()> {
        let entry = self.games.lock().expect("games 锁").remove(&id);
        self.last_access.lock().expect("access 锁").remove(&id);
        if let Some(path) = self.persist_path(id) {
            let _ = fs::remove_file(path);
        }
        // 回收该 game_id 的 LIVE 会话（2026-09 修复 P0-2）。
        if let Some(live) = &self.live_sessions {
            live.end_all_for_game(id);
        }
        entry?.shutdown();
        Some(())
    }

    /// 活跃（内存）游戏数。
    pub fn count(&self) -> usize {
        self.games.lock().expect("games 锁").len()
    }

    /// 配置的活跃上限（健康检查/压测脚本观察收缩进度用）。
    pub fn max_active(&self) -> Option<usize> {
        self.max_active
    }

    /// 在阻塞线程池排发一次容量收缩（幂等：已有任务在跑就不重复排发）。
    /// 长档 gzip 落盘是秒级重操作，不能让 `/health` 请求同步等待。
    pub fn spawn_rebalance(self: &Arc<Self>) {
        if self.rebalancing.swap(true, Ordering::SeqCst) {
            return;
        }
        let manager = self.clone();
        // fire-and-forget：JoinHandle drop 即分离后台任务，不等待长档落盘。
        drop(tokio::task::spawn_blocking(move || {
            manager.evict_lru_if_needed();
            manager.rebalancing.store(false, Ordering::SeqCst);
        }));
    }

    /// 已持久化但未活跃的游戏数（活跃局恢复后遗留的陈旧磁盘包不计入）。
    pub fn persisted_count(&self) -> usize {
        let Some(dir) = &self.persist_dir else {
            return 0;
        };
        let Ok(read) = fs::read_dir(dir) else {
            return 0;
        };
        let active: HashSet<u64> = self
            .games
            .lock()
            .expect("games 锁")
            .keys()
            .copied()
            .collect();
        read.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "gz"))
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".game.gz"))
                    .and_then(|n| n.parse::<u64>().ok())
                    .is_none_or(|id| !active.contains(&id))
            })
            .count()
    }

    // —— 容量治理 ——

    pub(crate) fn persist_path(&self, id: u64) -> Option<PathBuf> {
        self.persist_dir
            .as_ref()
            .map(|dir| dir.join(format!("{id}.game.gz")))
    }

    /// 存档读取路径：优先主 `persist_dir`，不存在则回退到兼容读回退目录。
    /// 写盘始终走 `persist_path`（主目录）；本方法只用于恢复读取。
    fn read_path(&self, id: u64) -> Option<PathBuf> {
        let primary = self.persist_path(id)?;
        if primary.is_file() {
            Some(primary)
        } else {
            self.persist_dir_fallback
                .as_ref()
                .map(|dir| dir.join(format!("{id}.game.gz")))
                .filter(|p| p.is_file())
                .or(Some(primary))
        }
    }

    pub(crate) fn touch(&self, id: u64) {
        self.last_access
            .lock()
            .expect("access 锁")
            .insert(id, Instant::now());
    }

    /// 容量治理：活跃数 ≥ 上限时按 LRU 卸载空闲局，直到回落至上限以下。
    ///
    /// 只在 `create` / `get_or_restore` 的预卸载阶段调用；并发长档突发会让
    /// 所有局同时处于推进/保存中（「全忙、本批不淘汰」），活跃数可能暂时超过
    /// 上限，后续 create/restore 会逐步回收。**不要**在普通请求路径上事后收缩：
    /// 并发请求可能已拿到某局的 Arc 但尚未开始操作，事后卸载会把它中途杀掉。
    pub(crate) fn evict_lru_if_needed(&self) {
        let Some(max) = self.max_active else { return };
        if self.persist_dir.is_none() || self.loader.is_none() {
            return;
        }
        while self.count() >= max {
            // 只淘汰既不在推进、也不在保存中的最久未访问游戏；全忙则本批停止。
            let victim = {
                let games = self.games.lock().expect("games 锁");
                let access = self.last_access.lock().expect("access 锁");
                let restoring = self.restoring.lock().expect("restoring 锁");
                access
                    .iter()
                    .filter_map(|(id, at)| games.get(id).map(|entry| (*id, *at, entry.clone())))
                    .filter(|(id, _, entry)| {
                        // 正在从磁盘恢复的局暂不淘汰：restore 已把磁盘包读入内存，
                        // 插入 map 与删除磁盘包之间若被淘汰，可能把新落盘包误删。
                        !restoring.contains(id)
                            && !entry.advancing.load(Ordering::SeqCst)
                            && !entry.saving.load(Ordering::SeqCst)
                            && !entry.persisting.load(Ordering::SeqCst)
                            && entry.leased.load(Ordering::SeqCst) == 0
                    })
                    .min_by_key(|(_, at, _)| *at)
                    .map(|(id, _, _)| id)
            };
            let Some(id) = victim else { break };
            self.persist_and_remove(id);
        }
    }

    fn persist_and_remove(&self, id: u64) {
        // 先落盘、后摘除 map：磁盘包在任一瞬间都不会「既不在 map、又不在磁盘」，
        // 并发恢复请求读到的要么是旧包、要么是新包，不会 No such file。
        let entry = {
            let games = self.games.lock().expect("games 锁");
            let Some(entry) = games.get(&id).cloned() else {
                return;
            };
            entry
        };
        // 写同一局 tmp 的并发淘汰任务只允许一个进入。
        if entry.persisting.swap(true, Ordering::SeqCst) {
            return;
        }
        let persisted = self.persist_entry(&entry);
        entry.persisting.store(false, Ordering::SeqCst);
        if let Err(e) = persisted {
            eprintln!("[GameManager] 游戏 {id} 持久化失败（保留在内存）：{e}");
            return;
        }
        // 写盘期间该局可能重新被请求（租约/推进/保存）：再次确认空闲才摘除。
        let remove = {
            let mut games = self.games.lock().expect("games 锁");
            let still_idle = games.get(&id).is_some_and(|e| {
                !e.advancing.load(Ordering::SeqCst)
                    && !e.saving.load(Ordering::SeqCst)
                    && e.leased.load(Ordering::SeqCst) == 0
            });
            if still_idle {
                games.remove(&id);
                true
            } else {
                false
            }
        };
        if remove {
            self.last_access.lock().expect("access 锁").remove(&id);
            // 回收该 game_id 的 LIVE 会话（2026-09 修复 P0-2：淘汰即清），
            // 使「会话只增不删」随游戏生命周期一起收敛。
            if let Some(live) = &self.live_sessions {
                live.end_all_for_game(id);
            }
            entry.shutdown();
        }
    }

    fn persist_entry(&self, entry: &GameEntry) -> Result<(), String> {
        let Some(path) = self.persist_path(entry.id) else {
            return Err("未配置持久化目录".into());
        };
        let session = entry.career_session.lock().expect("career session 锁");
        let snapshot = entry.snapshot.lock().expect("snapshot 锁").clone();
        let mut envelope = build_persisted_envelope(entry.policy, entry.story, snapshot, 3)?;
        envelope.career_session = session.clone();
        write_persisted_package(&path, &envelope)
    }

    fn restore_from_disk(self: &Arc<Self>, id: u64) -> Result<Arc<GameEntry>, String> {
        let (Some(path), Some(loader)) = (self.read_path(id), self.loader.as_ref()) else {
            return Err(format!("游戏 {id} 不存在"));
        };
        let bytes = fs::read(&path).map_err(|e| format!("读取持久化包失败：{e}"))?;
        let mut json = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut json)
            .map_err(|e| format!("解压失败：{e}"))?;
        let persisted: PersistedGame =
            serde_json::from_slice(&json).map_err(|e| format!("持久化包损坏：{e}"))?;
        if persisted.format != "csc-persisted-game" {
            return Err("不支持的持久化包格式".into());
        }
        // v1（无 CRC 字段）→ 接受并升级为 v2；v2（带 CRC）→ 校验完整性。
        // 其余版本号一律拒绝，避免误恢复未知格式的包。
        let needs_upgrade = match persisted.format_version {
            1 => true,
            2 | 3 => false,
            other => return Err(format!("不支持的持久化包版本：{other}")),
        };
        if !needs_upgrade {
            let state_json = canonical_state_bytes(&persisted.state)?;
            if crc32fast::hash(&state_json) != persisted.state_crc32 {
                let corrupt = path.with_extension("corrupt");
                let _ = fs::rename(&path, &corrupt);
                return Err(format!("持久化包校验失败，已隔离为 {corrupt:?}"));
            }
        }
        let state = persisted.state.migrate_state();
        let profile = loader
            .rating_profile
            .as_deref()
            .map(csc_entities::baseline::RatingProfile::from_json_str)
            .transpose()
            .map_err(|e| format!("rating_profile 资产损坏：{e}"))?;
        let mut engine = Engine::empty(
            0,
            SimClock::of(state.sim_year, state.sim_month, state.sim_day),
        );
        if let Some(content) = &loader.narrative {
            engine
                .set_narrative_content(content.clone())
                .map_err(|e| e.to_string())?;
        }
        engine.restore_with_profile(state, profile);
        let entry = self.spawn_entry(
            id,
            engine,
            persisted.policy,
            persisted.story,
            persisted.career_session,
        );
        self.games
            .lock()
            .expect("games 锁")
            .insert(id, entry.clone());
        self.touch(id);
        // v1 包已在内存恢复：把迁移后的最新状态重写为 v2（带 CRC），
        // 使磁盘包也升级到当前格式，后续不再走 v1 兼容分支。
        if needs_upgrade {
            let snapshot = entry.snapshot.lock().expect("snapshot 锁").clone();
            let upgraded =
                build_persisted_envelope(persisted.policy, persisted.story, snapshot, 2)?;
            write_persisted_package(&path, &upgraded)?;
        }
        // 磁盘包保留为陈旧副本：恢复后不删除。这样「恢复 → 立即被淘汰」的
        // 并发窗口里，别的请求重试恢复时仍有文件可读；下次淘汰会原子覆盖它。
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::super::persist::{
        build_persisted_envelope, transfer_market_from_pending, write_persisted_package,
    };
    use super::*;
    use csc_core::engine::Engine;
    use csc_core::state::GameState;
    use csc_decision::point::DecisionPoint;
    use csc_time::clock::SimClock;
    use csc_util::rng::Xoshiro256StarStar;
    use flate2::Compression;
    use flate2::read::GzDecoder;
    use flate2::write::GzEncoder;
    use std::fs;
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// 40 队满编世界 + 主角（并行日历生态的最小规模；无主角则无决策点，
    /// Human 策略下 decide() 永远不会被调用）。
    fn fixture_engine() -> Engine {
        let mut rankings = String::new();
        for ti in 0..40 {
            if ti > 0 {
                rankings.push(',');
            }
            let roster: Vec<String> = (0..5).map(|i| format!("T{ti}P{i}")).collect();
            rankings.push_str(&format!(
                r#"{{"ranking":{},"points":{},"teamName":"Team{}","roster":[{}]}}"#,
                ti + 1,
                2000 - ti * 50,
                ti,
                roster
                    .iter()
                    .map(|n| format!(r#""{n}""#))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let files = vec![(
            "standings_global_2026_01_05.json".to_string(),
            format!(r#"{{"rankings":[{rankings}]}}"#),
        )];
        let mut eng = Engine::load_from_standings(
            &files,
            &csc_core::CalibrationAssets::default(),
            42,
            SimClock::of(2026, 1, 1),
        )
        .expect("fixture 合法");
        // 主角挂在榜首队伍：推进到 2 月（PGL Cluj-Napoca 等顶级赛事）会触发
        // 场内干预决策点（决策弹窗仅限 T1+ 赛事的新规则下仍可验证干预路径）
        eng.create_protagonist(
            "MyPlayer",
            csc_domain::tier::Tier::Tier1,
            csc_util::id::TeamId(0),
            &mut Xoshiro256StarStar::seed(7),
            None,
        )
        .expect("主角创建");
        eng
    }

    #[test]
    fn auto_game_advances_and_reports() {
        let mgr = Arc::new(GameManager::new());
        let id = mgr.create(fixture_engine(), Policy::Auto);
        let entry = mgr.get(id).expect("游戏存在");
        let summaries = entry
            .advance_blocking(1, std::time::Duration::from_secs(120))
            .expect("推进成功");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].month, 1);
        assert!(summaries[0].journal_added > 0, "推进应产生事件");
        let state = entry.state().expect("取状态");
        assert_eq!(state.month, 1);
        mgr.shutdown(id);
        assert_eq!(mgr.count(), 0);
    }

    #[tokio::test]
    async fn lru_eviction_persists_and_restores_game() {
        let dir = std::env::temp_dir().join(format!("csc-game-lifecycle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let loader = Arc::new(GameLoader::default());
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(loader),
        ));

        let id0 = mgr.create(fixture_engine(), Policy::Auto);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let id1 = mgr.create(fixture_engine(), Policy::Auto);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let id2 = mgr.create(fixture_engine(), Policy::Auto);

        assert_eq!(mgr.count(), 2, "超过 max_active 后应淘汰最旧游戏");
        assert_eq!(mgr.persisted_count(), 1, "被淘汰游戏应写入磁盘");
        assert!(mgr.get(id0).is_none(), "LRU 受害者应为 id0");

        let restored = mgr.get_or_restore(id0).await.expect("磁盘恢复成功");
        assert_eq!(restored.id, id0);
        assert_eq!(mgr.count(), 2, "恢复不应突破活跃上限");
        assert!(
            mgr.get(id1).is_none(),
            "恢复前应为 id0 腾位置，LRU 转存 id1"
        );
        assert_eq!(mgr.persisted_count(), 1, "id0 磁盘包被消费，id1 磁盘包驻留");
        let state = restored.state().expect("恢复后状态可用");
        assert_eq!(state.month, 0, "恢复状态与卸载前一致");

        mgr.shutdown(id0);
        mgr.shutdown(id1);
        mgr.shutdown(id2);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn capacity_model_six_games_keeps_four_active() {
        let dir = std::env::temp_dir().join(format!("csc-capacity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(4),
            Some(Arc::new(GameLoader::default())),
        ));
        let mut ids = Vec::new();
        for _ in 0..6 {
            let id = mgr.create(fixture_engine(), Policy::Auto);
            ids.push(id);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert_eq!(mgr.count(), 4, "容量模型：6 局只保留 4 局活跃");
        assert_eq!(mgr.persisted_count(), 2, "其余 2 局应落盘");

        // 被淘汰的最旧局可恢复且状态一致。
        let restored = mgr.get_or_restore(ids[0]).await.expect("恢复最旧局");
        assert_eq!(restored.id, ids[0]);
        assert_eq!(restored.state().expect("状态可用").month, 0);

        for id in ids {
            mgr.shutdown(id);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn busy_burst_recovers_to_capacity_ceiling() {
        let dir = std::env::temp_dir().join(format!("csc-burst-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(Arc::new(GameLoader::default())),
        ));
        let mut ids = Vec::new();
        for _ in 0..4 {
            let id = mgr.create(fixture_engine(), Policy::Auto);
            ids.push(id);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert_eq!(mgr.count(), 2, "稳态：活跃数不超过上限");

        // 模拟并发突发：两个活跃局都在保存中 → 本轮无法淘汰 →
        // 恢复最旧局会让活跃数暂时超过上限。
        let active: Vec<_> = ids.iter().filter_map(|id| mgr.get(*id)).collect();
        assert_eq!(active.len(), 2);
        for entry in &active {
            entry.saving.store(true, Ordering::SeqCst);
        }
        let restored = mgr.get_or_restore(ids[0]).await.expect("全忙时也应能恢复");
        assert_eq!(restored.id, ids[0]);
        assert_eq!(mgr.count(), 3, "突发中活跃数暂时超上限");
        for entry in &active {
            entry.saving.store(false, Ordering::SeqCst);
        }

        // 突发结束后的下一次治理调用应连续卸载，把活跃数收缩回上限以下。
        mgr.evict_lru_if_needed();
        assert!(mgr.count() < 2, "突发后应收缩到上限以下");
        assert!(
            mgr.get(restored.id).is_some(),
            "最近恢复/访问的局不应被卸载"
        );

        for id in ids {
            mgr.shutdown(id);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn eviction_skips_game_while_restoring() {
        let dir = std::env::temp_dir().join(format!("csc-evict-restoring-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(1),
            Some(Arc::new(GameLoader::default())),
        ));
        let id = mgr.create(fixture_engine(), Policy::Auto);
        mgr.restoring.lock().expect("restoring 锁").insert(id);
        mgr.evict_lru_if_needed();
        assert!(mgr.get(id).is_some(), "恢复中的局不得被容量治理卸载");
        mgr.restoring.lock().expect("restoring 锁").remove(&id);
        mgr.shutdown(id);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn concurrent_restore_of_same_game_never_404s() {
        let dir = std::env::temp_dir().join(format!("csc-restore-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(Arc::new(GameLoader::default())),
        ));
        let id0 = mgr.create(fixture_engine(), Policy::Auto);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let id1 = mgr.create(fixture_engine(), Policy::Auto);
        std::thread::sleep(std::time::Duration::from_millis(5));
        mgr.create(fixture_engine(), Policy::Auto);
        assert!(mgr.get(id0).is_none(), "id0 已被 LRU 卸载");

        let (a, b) = tokio::join!(mgr.get_or_restore(id0), mgr.get_or_restore(id0));
        assert_eq!(
            a.as_ref().map(|e| e.id),
            Some(id0),
            "两个并发恢复请求都应命中同一局"
        );
        assert_eq!(
            b.as_ref().map(|e| e.id),
            Some(id0),
            "不得因磁盘包被并发消费而 404"
        );

        mgr.shutdown(id0);
        mgr.shutdown(id1);
        let _ = fs::remove_dir_all(dir);
    }

    /// 往磁盘写一个 v1（无 CRC 字段）持久化包，模拟旧版 {id}.game.gz。
    fn write_v1_package(path: &std::path::Path, policy: Policy, snapshot: &GameState) {
        let mut json = serde_json::to_value(PersistedGame {
            format: "csc-persisted-game".into(),
            format_version: 1,
            policy,
            story: false,
            career_session: super::super::career::CareerSession::default(),
            state_crc32: 0,
            state: snapshot.clone(),
        })
        .expect("v1 envelope 序列化");
        // 显式移除 state_crc32 键，构造真正的 v1 包（无 CRC 字段）。
        json.as_object_mut()
            .expect("envelope 是对象")
            .remove("state_crc32");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut encoder, &json).expect("v1 包写 json");
        let bytes = encoder.finish().expect("v1 包压缩");
        fs::write(path, bytes).expect("v1 包写盘");
    }

    /// 从 v2 磁盘包构造一个篡改包（改 state 内标量使 CRC 失配），写回原路径。
    fn tamper_v2_package(path: &std::path::Path) {
        let bytes = fs::read(path).expect("读包");
        let mut json = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut json)
            .expect("解压");
        let mut envelope: serde_json::Value = serde_json::from_slice(&json).expect("包可解析");
        envelope["state"]["sim_year"] = serde_json::json!(9999);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut encoder, &envelope).expect("篡改包写 json");
        let tampered = encoder.finish().expect("篡改包压缩");
        fs::write(path, tampered).expect("写篡改包");
    }

    /// 用手工快照在磁盘写一个 v2 包（带 CRC），供恢复测试使用。
    fn write_v2_package(path: &std::path::Path, policy: Policy, snapshot: &GameState) {
        let envelope =
            build_persisted_envelope(policy, false, snapshot.clone(), 2).expect("构建 v2 envelope");
        write_persisted_package(path, &envelope).expect("写 v2 包");
    }

    #[tokio::test]
    async fn v1_package_restores_and_upgrades_to_v2() {
        let dir = std::env::temp_dir().join(format!("csc-v1-upgrade-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(Arc::new(GameLoader::default())),
        ));
        // 取一个合法快照，写 v1 包；shutdown 释放内存后磁盘只剩 v1 包。
        let src = mgr.create(fixture_engine(), Policy::Auto);
        let snapshot = mgr
            .get(src)
            .expect("源局存在")
            .snapshot
            .lock()
            .expect("锁")
            .clone();
        let path = mgr.persist_path(src).expect("持久化路径");
        mgr.shutdown(src);
        fs::create_dir_all(&dir).expect("建目录");
        write_v1_package(&path, Policy::Auto, &snapshot);

        // v1 包应能恢复成功（不 panic、不产生半恢复 entry）。
        let restored = mgr.get_or_restore(src).await.expect("v1 包应可恢复");
        assert_eq!(restored.id, src);
        assert_eq!(restored.state().expect("状态可用").month, 0);

        // 磁盘包应已升级为 v2：读回应包含 state_crc32 且 format_version=2。
        let bytes = fs::read(&path).expect("读升级后包");
        let mut json = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut json)
            .expect("解压");
        let upgraded: PersistedGame = serde_json::from_slice(&json).expect("升级后包可解析");
        assert_eq!(upgraded.format_version, 2, "v1 包应升级为 v2");
        assert_eq!(
            upgraded.state_crc32,
            crc32fast::hash(&canonical_state_bytes(&snapshot).expect("状态"))
        );

        mgr.shutdown(src);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn v2_crc_roundtrip_restores_unchanged() {
        let dir = std::env::temp_dir().join(format!("csc-v2-roundtrip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(Arc::new(GameLoader::default())),
        ));
        // 取快照 → 写 v2 包 → 释放内存，恢复时应返回原状。
        let id = mgr.create(fixture_engine(), Policy::Auto);
        let snapshot = mgr
            .get(id)
            .expect("局存在")
            .snapshot
            .lock()
            .expect("锁")
            .clone();
        let path = mgr.persist_path(id).expect("路径");
        mgr.shutdown(id);
        fs::create_dir_all(&dir).expect("建目录");
        write_v2_package(&path, Policy::Auto, &snapshot);
        assert!(mgr.get(id).is_none(), "应已卸载");

        let restored = mgr.get_or_restore(id).await.expect("v2 正常恢复");
        assert_eq!(restored.id, id);
        assert_eq!(restored.state().expect("状态可用").month, 0);
        // 磁盘包保留为 v2（不被误升级/降级）。
        let bytes = fs::read(&path).expect("读包");
        let mut json = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut json)
            .expect("解压");
        let on_disk: PersistedGame = serde_json::from_slice(&json).expect("包可解析");
        assert_eq!(on_disk.format_version, 2, "v2 包保持 v2");

        mgr.shutdown(id);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn crc_tamper_isolates_corrupt_and_leaves_no_entry() {
        let dir = std::env::temp_dir().join(format!("csc-crc-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mgr = Arc::new(GameManager::with_config(
            Some(dir.clone()),
            Some(2),
            Some(Arc::new(GameLoader::default())),
        ));
        // 写合法 v2 包后篡改 state，使 CRC 失配。
        let id = mgr.create(fixture_engine(), Policy::Auto);
        let snapshot = mgr
            .get(id)
            .expect("局存在")
            .snapshot
            .lock()
            .expect("锁")
            .clone();
        let path = mgr.persist_path(id).expect("路径");
        mgr.shutdown(id);
        fs::create_dir_all(&dir).expect("建目录");
        write_v2_package(&path, Policy::Auto, &snapshot);
        tamper_v2_package(&path);

        // 恢复应失败（优雅降级），不得产生半恢复 entry。
        let restored = mgr.get_or_restore(id).await;
        assert!(restored.is_none(), "篡改包应恢复失败");
        assert!(mgr.get(id).is_none(), "不得残留半恢复 entry");
        // 原包应被改名隔离为 .corrupt。
        assert!(!path.is_file(), "篡改包应被移走");
        let corrupt = path.with_extension("corrupt");
        assert!(corrupt.is_file(), "应生成 .corrupt 隔离文件");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn transfer_market_extracts_pending_transfer_window_offer() {
        // 构造一个挂起批次：含一个 TransferWindow 决策点（转会窗报价）+ 一个无关决策点。
        let pending = Some(PendingBatch {
            batch_id: 1,
            date: Some("2026-06-08".into()),
            points: vec![
                DecisionPoint::TransferWindow {
                    id: "2026-06-08|transfer|ZywOo".into(),
                    date: "2026-06-08".into(),
                    player_id: csc_util::id::PlayerId(0),
                    player_name: "ZywOo".into(),
                    offers: vec![csc_decision::offer::TransferOffer {
                        point_id: "2026-06-08|transfer|ZywOo".into(),
                        player_id: csc_util::id::PlayerId(0),
                        player_name: "ZywOo".into(),
                        from_team_id: Some(csc_util::id::TeamId(5)),
                        from_team_signature: Some("TeamX|a,b,c,d,e".into()),
                        candidates: vec![csc_decision::offer::TransferTarget {
                            team_id: csc_util::id::TeamId(1),
                            team_signature: "TeamY|p,q,r,s,t".into(),
                            ranking: 2,
                            power_delta: 3.5,
                            salary: 150_000,
                        }],
                        discounted: false,
                        current_ranking: Some(3),
                    }],
                },
                DecisionPoint::InjuryDecision {
                    id: "2026-06-08|injury|ZywOo".into(),
                    date: "2026-06-08".into(),
                    player_id: csc_util::id::PlayerId(0),
                    player_name: "ZywOo".into(),
                    injury: csc_entities::injury::Injury {
                        kind: csc_entities::injury::InjuryKind::Wrist,
                        severity: csc_entities::injury::InjurySeverity::Minor,
                        days_left: 1,
                        sustained_date: "2026-06-08".into(),
                        source: "s".into(),
                        ticked: false,
                    },
                    options: Vec::new(),
                },
            ],
        });
        let market = transfer_market_from_pending(&pending).expect("应有挂起转会窗报价");
        assert_eq!(market["date"], "2026-06-08");
        // 截至日期 = 报价月月末（"有效期至 6 月 30 日"）。
        assert_eq!(market["deadline"], "2026-06-30");
        // 只提取 TransferWindow，无关决策点（伤病）被过滤。
        let offers = market["offers"].as_array().unwrap();
        assert_eq!(offers.len(), 1, "仅转会窗 offer 入市场");
        assert_eq!(offers[0]["point_id"], "2026-06-08|transfer|ZywOo");
        assert_eq!(
            offers[0]["candidates"][0]["salary"], 150_000,
            "候选携带确定性薪资"
        );

        // 无挂起报价 → None。
        assert!(
            transfer_market_from_pending(&None).is_none(),
            "无挂起报价应返回 None"
        );
        // 有批次但无转会窗点 → None。
        let no_transfer = Some(PendingBatch {
            batch_id: 2,
            date: Some("2026-06-08".into()),
            points: vec![DecisionPoint::TrainingFocus {
                id: "x".into(),
                date: "d".into(),
                player_id: csc_util::id::PlayerId(0),
                player_name: "p".into(),
                options: vec![],
            }],
        });
        assert!(
            transfer_market_from_pending(&no_transfer).is_none(),
            "无转会窗点应返回 None"
        );
    }

    /// P0-2 集成回归：`shutdown` 必须回收该 game_id 的全部 LIVE 会话。
    /// 此前 LiveSessionStore 与 GameManager 生命周期解耦，DELETE 一局后其 LIVE 会话
    /// 仍驻留进程内存（泄漏），且同场重进被旧会话 422 拒绝。
    #[test]
    fn shutdown_clears_live_sessions_for_game() {
        let dir = std::env::temp_dir().join(format!("csc-shutdown-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let live = LiveSessionStore::default();
        let mgr = Arc::new(
            GameManager::with_config(
                Some(dir.clone()),
                Some(4),
                Some(Arc::new(GameLoader::default())),
            )
            .with_live_sessions(live.clone()),
        );

        let id = mgr.create(fixture_engine(), Policy::Auto);
        let config = csc_simulation::LiveMapConfig {
            match_id: "m-shutdown".into(),
            series_id: "series-1".into(),
            map_id: "map_1".into(),
            map_number: 1,
            team_a_id: csc_util::id::TeamId(0),
            team_b_id: csc_util::id::TeamId(1),
            team_a_sig: "A|a0,a1,a2,a3,a4".into(),
            team_b_sig: "B|b0,b1,b2,b3,b4".into(),
            best_of: 1,
            world_seed: 42,
            base_win_prob_a: 0.55,
            team_a_players: (0..5)
                .map(|i| csc_simulation::LivePlayerSlot {
                    id: csc_util::id::PlayerId(i),
                    name: format!("a{i}"),
                })
                .collect(),
            team_b_players: (0..5)
                .map(|i| csc_simulation::LivePlayerSlot {
                    id: csc_util::id::PlayerId(100 + i),
                    name: format!("b{i}"),
                })
                .collect(),
        };
        live.start(id, config).expect("start");

        // shutdown 后：Live 会话必须被回收。
        mgr.shutdown(id);
        assert!(
            live.get(id, "m-shutdown").is_none(),
            "shutdown 应清 Live 会话"
        );

        let _ = fs::remove_dir_all(dir);
    }
}
