//! Server-owned LIVE map sessions.
//!
//! A session stores only the current engine state and already-played round events.
//! Future outcomes remain inside `LiveMatchEngine` until the client advances them.
//! Critical-round pauses are derived solely from that state, so they consume no RNG.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use csc_domain::live::EconomyBuy;
use csc_simulation::{
    LiveMapConfig, LiveMatchEngine, LiveRoundDecision, LiveRoundEvent, LiveRoundOutput,
    LiveRoundState,
};

/// LIVE 会话推进硬上界（MR12 上半场 12 + 下半场 12 = 24 回合，加加时/防御余量）。
/// 任何推进循环（`skip` 等）绝不越过此界——会话永不悬挂（R2 卡死修复）。
const MAX_ROUNDS: u32 = 48;

/// Stable identifier scope: a match id only needs to be unique within one game.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LiveSessionKey {
    pub game_id: u64,
    pub match_id: String,
}

/// Why the server paused before the next round. Values are deterministic labels, not simulation input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveCriticalReason {
    MatchPoint,
    LateCloseScore,
    EconomyClash,
}

/// A decision request for the *next* round. The candidates intentionally exclude player-targeted
/// morale actions because a generic spectator UI has no selected player target.
#[derive(Debug, Clone, Serialize)]
pub struct LiveDecisionRequest {
    pub round_number: i32,
    pub reason: LiveCriticalReason,
    pub candidates: Vec<LiveRoundDecision>,
    pub economy_a: EconomyBuy,
    pub economy_b: EconomyBuy,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveDecisionRecord {
    pub round_number: i32,
    pub reason: LiveCriticalReason,
    pub decision: LiveRoundDecision,
    pub economy_a: EconomyBuy,
    pub economy_b: EconomyBuy,
}

#[derive(Debug, Clone)]
struct LiveSession {
    state: LiveRoundState,
    pending: Option<LiveDecisionRequest>,
    queued_decision: Option<LiveRoundDecision>,
    decisions: Vec<LiveDecisionRecord>,
    /// R2：玩家是否已「看完」本场 LIVE（watched 语义）。看完后前端不再重复弹
    /// 「观看」入口；幂等——重复标记不重复计次，`skip`/`review` 也置 watched。
    watched: bool,
}

/// Public snapshot returned by all LIVE endpoints.
#[derive(Debug, Clone, Serialize)]
pub struct LiveSessionView {
    pub state: LiveRoundState,
    pub finished: bool,
    pub decision_required: bool,
    pub decision_request: Option<LiveDecisionRequest>,
    /// R2：已看完标记（看完后不再重复弹观看入口；幂等）。
    #[serde(default)]
    pub watched: bool,
    /// R2：已完成回合编号（升序；服务端下发的「已播回合」清单，供前端决定
    /// 哪些回合不再展示——前端不再自行遍历 `state.round_history`）。
    #[serde(default)]
    pub played_rounds: Vec<i32>,
    /// R2：最近一回合（含击杀事件流）；None = 尚未开始任一回合。
    #[serde(default)]
    pub current_round: Option<LiveRoundEvent>,
}

/// A just-played round plus the updated visible session state.
#[derive(Debug, Clone, Serialize)]
pub struct LiveAdvanceView {
    pub output: LiveRoundOutput,
    pub state: LiveRoundState,
    pub decision_required: bool,
    pub decision_request: Option<LiveDecisionRequest>,
}

/// A review item only contains facts from completed rounds and decisions actually submitted.
#[derive(Debug, Clone, Serialize)]
pub struct LiveReviewRound {
    pub event: LiveRoundEvent,
    pub reason: LiveCriticalReason,
    pub decision: Option<LiveRoundDecision>,
    pub economy_a: Option<EconomyBuy>,
    pub economy_b: Option<EconomyBuy>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveReview {
    pub match_id: String,
    pub finished: bool,
    pub final_score_a: i32,
    pub final_score_b: i32,
    pub key_rounds: Vec<LiveReviewRound>,
}

#[derive(Clone, Default)]
pub struct LiveSessionStore {
    sessions: Arc<Mutex<HashMap<LiveSessionKey, LiveSession>>>,
}

impl LiveSessionStore {
    /// 开始（或**重新开始**）一场 LIVE 会话。
    ///
    /// 幂等（2026-09 修复）：同 `(game_id, match_id)` 已存在会话时**覆盖重建**而非报错——
    /// 此前会话永不回收，重连/误关页面/想重看一次都会被 422 拒绝。覆盖即丢弃旧会话的
    /// 回合进度与 watched 标记（review 数据随之重建；想看历史回放请走 `replay` 懒加载端点）。
    pub fn start(&self, game_id: u64, config: LiveMapConfig) -> Result<LiveSessionView, String> {
        if config.match_id.trim().is_empty() {
            return Err("match_id 不能为空".into());
        }
        let key = LiveSessionKey {
            game_id,
            match_id: config.match_id.clone(),
        };
        let mut sessions = self.sessions.lock().expect("LIVE session 锁");
        let state = LiveMatchEngine::start_map(config);
        let session = LiveSession {
            state,
            pending: None,
            queued_decision: None,
            decisions: Vec::new(),
            watched: false,
        };
        let view = Self::view(&session);
        sessions.insert(key, session);
        Ok(view)
    }

    /// 结束单场 LIVE 会话（回收内存）。幂等：不存在即无操作。
    ///
    /// 用于「玩家显式退出 LIVE」或单场清理；游戏整体回收走 [`Self::end_all_for_game`]。
    pub fn end(&self, game_id: u64, match_id: &str) {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        self.sessions.lock().expect("LIVE session 锁").remove(&key);
    }

    /// 结束一个游戏的全部 LIVE 会话（游戏销毁/LRU 淘汰时调用）。
    ///
    /// 2026-09 修复：此前会话只增不删、进程级泄漏，且 DELETE 一局后其 LIVE 会话仍驻留内存。
    /// 本方法按 game_id 一次性清空，使回收路径与 GameManager 的生命周期对齐。
    pub fn end_all_for_game(&self, game_id: u64) {
        self.sessions
            .lock()
            .expect("LIVE session 锁")
            .retain(|k, _| k.game_id != game_id);
    }

    pub fn get(&self, game_id: u64, match_id: &str) -> Option<LiveSessionView> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        self.sessions
            .lock()
            .expect("LIVE session 锁")
            .get(&key)
            .map(Self::view)
    }

    /// Advances exactly one round. A queued decision is consumed here, entering the engine seed
    /// chain; an unselected critical request blocks this endpoint with a conflict.
    pub fn advance(&self, game_id: u64, match_id: &str) -> Result<LiveAdvanceView, String> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        let mut sessions = self.sessions.lock().expect("LIVE session 锁");
        let session = sessions
            .get_mut(&key)
            .ok_or_else(|| format!("LIVE 比赛 {match_id} 不存在"))?;
        if LiveMatchEngine::is_finished(&session.state) {
            return Err("LIVE 地图已结束，不能继续推进".into());
        }
        if session.pending.is_some() {
            return Err("关键回合等待决策；请先提交 /live/decide".into());
        }
        let output = LiveMatchEngine::simulate_next_round(
            &mut session.state,
            session.queued_decision.take(),
        );
        session.pending = Self::critical_request(&session.state);
        Ok(LiveAdvanceView {
            output,
            state: session.state.clone(),
            decision_required: session.pending.is_some(),
            decision_request: session.pending.clone(),
        })
    }

    /// Queues a choice for the current critical round. It is intentionally not simulated here:
    /// advancing consumes it exactly once and makes its decision sequence visible in the output.
    ///
    /// R3 修复（22 号审查）：**先借用校验，成功后才取走 pending**。旧实现先
    /// `session.pending.take()` 再 `candidates.contains`——非法选择会把 pending
    /// 清空且返回错误，随后 `advance` 能绕过原暂停点。现在校验失败时 pending 原样保留，
    /// 调用方仍可提交合法选择。
    pub fn decide(
        &self,
        game_id: u64,
        match_id: &str,
        decision: LiveRoundDecision,
    ) -> Result<LiveSessionView, String> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        let mut sessions = self.sessions.lock().expect("LIVE session 锁");
        let session = sessions
            .get_mut(&key)
            .ok_or_else(|| format!("LIVE 比赛 {match_id} 不存在"))?;
        // 借用校验（不消费）：无 pending 或选项非法 → 直接报错，pending 保持不变。
        let request = session
            .pending
            .as_ref()
            .ok_or_else(|| "当前没有等待中的关键回合决策".to_string())?;
        if !request.candidates.contains(&decision) {
            return Err(format!(
                "该决策不在当前关键回合候选列表中（round {}）；pending 未消费，可重新选择",
                request.round_number
            ));
        }
        // 校验通过：此刻才取走 pending（唯一消费点）。
        let request = session.pending.take().expect("已确认 pending 存在");
        session.decisions.push(LiveDecisionRecord {
            round_number: request.round_number,
            reason: request.reason,
            decision: decision.clone(),
            economy_a: request.economy_a,
            economy_b: request.economy_b,
        });
        session.queued_decision = Some(decision);
        Ok(Self::view(session))
    }

    /// Runs the same one-round engine transition repeatedly. Skipping is an explicit spectator
    /// choice: it declines any pending intervention and never takes a shortcut simulation path.
    ///
    /// R2 卡死修复：`while !is_finished` 无上界——若引擎因边界缺陷不进入 `MapEnd`
    /// （如比分判定失配），本循环会无限推进悬挂会话。现加**硬上界**（`MAX_ROUNDS`，
    /// MR12 上限 24 回合 + 加时余量），一旦触界立即强制收束为 `MapEnd` 并置
    /// `watched`，保证「跳过至赛果」永不悬挂（超时/重连自愈）。
    pub fn skip(&self, game_id: u64, match_id: &str) -> Result<LiveSessionView, String> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        let mut sessions = self.sessions.lock().expect("LIVE session 锁");
        let session = sessions
            .get_mut(&key)
            .ok_or_else(|| format!("LIVE 比赛 {match_id} 不存在"))?;
        session.pending = None;
        session.queued_decision = None;
        let mut rounds_skipped: u32 = 0;
        while !LiveMatchEngine::is_finished(&session.state) {
            if rounds_skipped >= MAX_ROUNDS {
                // 硬上界兜底：强制收束到 MapEnd（无 RNG 消费，仅改 phase）。
                session.state.phase = csc_domain::live::LiveMatchPhase::MapEnd;
                break;
            }
            LiveMatchEngine::simulate_next_round(&mut session.state, None);
            rounds_skipped += 1;
        }
        session.watched = true; // 跳过即视为「看完」，不再重复弹观看入口。
        Ok(Self::view(session))
    }

    /// R2：标记本场 LIVE 已看完（watched 语义，幂等）。返回更新后的视图；
    /// 不存在的会话返回 `Err`。看完后前端不再重复弹「观看回放」入口。
    pub fn mark_watched(&self, game_id: u64, match_id: &str) -> Result<LiveSessionView, String> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        let mut sessions = self.sessions.lock().expect("LIVE session 锁");
        let session = sessions
            .get_mut(&key)
            .ok_or_else(|| format!("LIVE 比赛 {match_id} 不存在"))?;
        session.watched = true; // 幂等：重复标记无副作用。
        Ok(Self::view(session))
    }

    pub fn review(&self, game_id: u64, match_id: &str) -> Option<LiveReview> {
        let key = LiveSessionKey {
            game_id,
            match_id: match_id.to_owned(),
        };
        let sessions = self.sessions.lock().expect("LIVE session 锁");
        let session = sessions.get(&key)?;
        let mut key_rounds = Vec::new();
        let mut previous_a: i32 = 0;
        let mut previous_b: i32 = 0;
        for event in &session.state.round_history {
            let record = session
                .decisions
                .iter()
                .find(|record| record.round_number == event.round_number);
            let reason = record.map(|record| record.reason).or_else(|| {
                // P2-8：复盘与实时暂停共用同一判定（回合前比分口径）——
                // 由「本回合开始前」的累计比分（previous_*）+ 回合前经济档位判定
                // （与 critical_request 完全一致；旧逻辑误用 event.score_* 回合后比分）。
                let economy_a = record.map(|r| r.economy_a);
                let economy_b = record.map(|r| r.economy_b);
                Self::critical_reason(previous_a, previous_b, economy_a, economy_b)
            });
            if let Some(reason) = reason {
                key_rounds.push(LiveReviewRound {
                    event: event.clone(),
                    reason,
                    decision: record.map(|record| record.decision.clone()),
                    economy_a: record.map(|record| record.economy_a),
                    economy_b: record.map(|record| record.economy_b),
                });
            }
            previous_a = event.score_a;
            previous_b = event.score_b;
        }
        Some(LiveReview {
            match_id: session.state.match_id.clone(),
            finished: LiveMatchEngine::is_finished(&session.state),
            final_score_a: session.state.score_a,
            final_score_b: session.state.score_b,
            key_rounds,
        })
    }

    fn view(session: &LiveSession) -> LiveSessionView {
        LiveSessionView {
            state: session.state.clone(),
            finished: LiveMatchEngine::is_finished(&session.state),
            decision_required: session.pending.is_some(),
            decision_request: session.pending.clone(),
            watched: session.watched,
            played_rounds: session
                .state
                .round_history
                .iter()
                .map(|e| e.round_number)
                .collect(),
            current_round: session.state.round_history.last().cloned(),
        }
    }

    /// 关键回合判定——**唯一口径**（P2-8）：基于「本回合开始前」的可见事实
    /// （实时暂停在回合前算 `state.score_*`；复盘对某回合用其 `previous_*`），
    /// 不 inspect 任何随机值/未来结果：
    /// - 任一方回合前已达 12 分 → 赛点（MatchPoint）；
    /// - 回合前合计 ≥20 且分差 ≤1 → 胶着末段（LateCloseScore）；
    /// - ForceBuy vs Eco/HalfBuy → 高杠杆经济交锋（EconomyClash）。
    ///
    /// `economy_a/b` 为 Option：复盘对无决策记录的回合（旧会话）传 None，跳过经济判定；
    /// 实时暂停总是 Some（会话持有当前经济）。
    fn critical_reason(
        score_a_before: i32,
        score_b_before: i32,
        economy_a: Option<EconomyBuy>,
        economy_b: Option<EconomyBuy>,
    ) -> Option<LiveCriticalReason> {
        if score_a_before >= 12 || score_b_before >= 12 {
            return Some(LiveCriticalReason::MatchPoint);
        }
        if score_a_before + score_b_before >= 20 && (score_a_before - score_b_before).abs() <= 1 {
            return Some(LiveCriticalReason::LateCloseScore);
        }
        match (economy_a, economy_b) {
            (Some(a), Some(b)) if Self::is_economy_clash(a, b) => {
                Some(LiveCriticalReason::EconomyClash)
            }
            _ => None,
        }
    }

    /// 实时关键回合暂停请求（下一回合决策面板数据）。
    ///
    /// Critical labels are based on facts available before the next round:
    /// - either side on 12 is a map point;
    /// - after 20 played rounds, a <=1 score gap is a late close round;
    /// - ForceBuy against Eco/HalfBuy is a high-leverage economic clash.
    ///
    /// No random values or future outcomes are inspected.
    fn critical_request(state: &LiveRoundState) -> Option<LiveDecisionRequest> {
        if LiveMatchEngine::is_finished(state) {
            return None;
        }
        // P2-8：与复盘共用 critical_reason（唯一口径，以「回合前比分」判定）；
        // 无关键理由（赛点/胶着/经济交锋都不满足）则不暂停，直接放行。
        let reason = Self::critical_reason(
            state.score_a,
            state.score_b,
            Some(state.economy_a.equipment),
            Some(state.economy_b.equipment),
        )?;
        Some(LiveDecisionRequest {
            round_number: state.round_number,
            reason,
            candidates: vec![
                LiveRoundDecision::CallTimeout,
                LiveRoundDecision::ChangePace(csc_domain::live::Pace::Fast),
                LiveRoundDecision::AggressiveOpening,
                LiveRoundDecision::PlayForTrade,
                LiveRoundDecision::Save,
                LiveRoundDecision::ForceBuy,
            ],
            economy_a: state.economy_a.equipment,
            economy_b: state.economy_b.equipment,
        })
    }

    fn is_economy_clash(a: EconomyBuy, b: EconomyBuy) -> bool {
        matches!(
            (a, b),
            (EconomyBuy::ForceBuy, EconomyBuy::Eco | EconomyBuy::HalfBuy)
                | (EconomyBuy::Eco | EconomyBuy::HalfBuy, EconomyBuy::ForceBuy)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(match_id: &str) -> LiveMapConfig {
        LiveMapConfig {
            match_id: match_id.to_string(),
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
        }
    }

    #[test]
    fn start_overwrites_existing_same_match_id() {
        let store = LiveSessionStore::default();
        // 首次 start 成功。
        store.start(1, config("m-again")).expect("start");
        // 同 (game_id, match_id) 再次 start：应覆盖重建而非 422 —— 这是「重进/重看同一场」的关键修复。
        let again = store
            .start(1, config("m-again"))
            .expect("再次 start 应成功");
        assert!(again.decision_request.is_none(), "新建会话应无关键回合暂停");
        assert!(!again.watched, "覆盖重建后 watched 复位");
    }

    /// R3 回归（22 号审查）：非法 LIVE 决策**不得消费 pending**。
    /// 推进到出现关键回合决策后，先提交候选外选项（报错），再提交合法选项（必须成功）。
    #[test]
    fn invalid_live_decision_preserves_pending() {
        let store = LiveSessionStore::default();
        store.start(1, config("m-invalid")).expect("start");
        // 推进到出现 pending 决策请求（有界循环，避免依赖具体比分）。
        let mut pending_request = None;
        for _ in 0..MAX_ROUNDS {
            let adv = match store.advance(1, "m-invalid") {
                Ok(v) => v,
                Err(_) => break, // 地图已结束
            };
            if let Some(req) = adv.decision_request {
                pending_request = Some(req);
                break;
            }
        }
        let req = pending_request.expect("推进途中应出现关键回合决策请求");
        // 候选外选项：`None`（不干预）不在 critical_request 的候选列表内。
        let illegal = LiveRoundDecision::None;
        assert!(
            !req.candidates.contains(&illegal),
            "测试前提：None 不在该关键回合候选中"
        );
        assert!(
            store.decide(1, "m-invalid", illegal).is_err(),
            "非法选项应报错"
        );
        // 关键：报错后 pending 未被消费，仍可再次提交；合法选项必须成功。
        let legal = req.candidates[0].clone();
        let view = store
            .decide(1, "m-invalid", legal)
            .expect("非法选择后仍应能提交合法选择");
        assert!(
            !view.decision_required,
            "合法提交后 pending 被消费、不再要求决策"
        );
    }

    #[test]
    fn end_removes_single_session_and_is_idempotent() {
        let store = LiveSessionStore::default();
        store.start(1, config("m-end")).expect("start");
        assert!(store.get(1, "m-end").is_some());
        store.end(1, "m-end");
        assert!(store.get(1, "m-end").is_none(), "end 后该场会话应消失");
        // 幂等：对不存在的场次 end 不 panic、无副作用。
        store.end(1, "m-end");
    }

    #[test]
    fn end_all_for_game_clears_only_that_game() {
        let store = LiveSessionStore::default();
        store.start(1, config("g1-a")).expect("start");
        store.start(1, config("g1-b")).expect("start");
        store.start(2, config("g2-a")).expect("start");
        store.end_all_for_game(1);
        assert!(store.get(1, "g1-a").is_none(), "game 1 会话应清空");
        assert!(store.get(1, "g1-b").is_none());
        assert!(store.get(2, "g2-a").is_some(), "game 2 会话应保留");
    }

    #[test]
    fn watched_is_false_until_marked_and_idempotent() {
        let store = LiveSessionStore::default();
        let view = store.start(1, config("m-watch")).expect("start");
        assert!(!view.watched, "新建会话默认未看完");

        // 第一次标记 → 看完。
        let v1 = store.mark_watched(1, "m-watch").expect("mark");
        assert!(v1.watched);

        // 幂等：重复标记无副作用，仍为已看完。
        let v2 = store.mark_watched(1, "m-watch").expect("mark again");
        assert!(v2.watched, "watched 标记应幂等");

        // get 视图也回传 watched。
        let got = store.get(1, "m-watch").expect("get");
        assert!(got.watched);
    }

    #[test]
    fn skip_never_hangs_and_marks_watched() {
        let store = LiveSessionStore::default();
        store.start(1, config("m-skip")).expect("start");
        // 「跳过至赛果」必须在有界回合内完成，且永远不悬挂。
        let view = store.skip(1, "m-skip").expect("skip 不应悬挂");
        assert!(view.finished, "跳过后必须已完赛");
        assert!(view.watched, "跳过即视为看完（watched）");
        // played_rounds 回填（服务端下发已播回合清单）。
        assert!(!view.played_rounds.is_empty());
        assert_eq!(
            view.played_rounds.last(),
            view.current_round.as_ref().map(|e| e.round_number).as_ref()
        );
        // 跳过后继续 advance 应拒绝（地图已结束）→ 不悬挂、不 panic。
        assert!(store.advance(1, "m-skip").is_err());
    }

    #[test]
    fn advance_exposes_played_rounds_and_current_round() {
        let store = LiveSessionStore::default();
        store.start(1, config("m-adv")).expect("start");
        let v0 = store.get(1, "m-adv").expect("get");
        assert!(v0.played_rounds.is_empty());
        assert!(v0.current_round.is_none(), "未开始时无当前回合");

        let adv = store.advance(1, "m-adv").expect("advance");
        // advance 视图含当前回合（带击杀流）。
        assert_eq!(adv.output.event.round_number, 1);
        assert!(!adv.output.event.kills.is_empty(), "推进回合含击杀流");

        let v1 = store.get(1, "m-adv").expect("get");
        assert_eq!(v1.played_rounds, vec![1], "已播回合清单 = [1]");
        assert_eq!(v1.current_round.as_ref().map(|e| e.round_number), Some(1));
    }

    #[test]
    fn live_session_view_current_round_has_kills() {
        let store = LiveSessionStore::default();
        store.start(1, config("m-kill")).expect("start");
        let _ = store.advance(1, "m-kill").expect("advance");
        let view = store.get(1, "m-kill").expect("get");
        let cr = view.current_round.expect("有当前回合");
        assert!(!cr.kills.is_empty(), "当前回合必须携带击杀事件流");
        // 击杀者/受害者必不同队。
        for k in &cr.kills {
            assert_ne!(k.killer.team, k.victim.team);
        }
    }

    /// P2-8：复盘与实时暂停共用 critical_reason——以「回合前比分」为唯一口径。
    /// 关键回归：恰好在「打完本回合才到 12 分」的回合（回合前 11 分）不是赛点，
    /// 实时不暂停，复盘也不得标 MatchPoint（旧 review 误用回合后比分会标）。
    #[test]
    fn critical_reason_uses_before_round_score_only() {
        // 回合前 11:5（未到赛点）——即使本回合 A 赢下到 12:5，也不算 MatchPoint。
        let reason = LiveSessionStore::critical_reason(
            11,
            5,
            Some(EconomyBuy::FullBuy),
            Some(EconomyBuy::FullBuy),
        );
        assert_eq!(reason, None, "回合前 11 分不是赛点（12 分线未到）");

        // 回合前 12:5（已到赛点）——真实暂停发生的场景，必须标 MatchPoint。
        let reason = LiveSessionStore::critical_reason(
            12,
            5,
            Some(EconomyBuy::FullBuy),
            Some(EconomyBuy::FullBuy),
        );
        assert_eq!(reason, Some(LiveCriticalReason::MatchPoint));

        // 回合前 9:11 合计 20 且差 2 → 不是 LateClose（差 ≤1 才算）；
        // 9:10 合计 19 差 1 → 也未到 20 合计线。
        assert_eq!(
            LiveSessionStore::critical_reason(
                9,
                11,
                Some(EconomyBuy::FullBuy),
                Some(EconomyBuy::FullBuy)
            ),
            None
        );
        // 回合前 10:10（合计 20 差 0）→ LateCloseScore。
        assert_eq!(
            LiveSessionStore::critical_reason(
                10,
                10,
                Some(EconomyBuy::FullBuy),
                Some(EconomyBuy::FullBuy)
            ),
            Some(LiveCriticalReason::LateCloseScore)
        );
        // ForceBuy vs Eco → EconomyClash（经济交锋，与比分无关）。
        assert_eq!(
            LiveSessionStore::critical_reason(
                5,
                3,
                Some(EconomyBuy::ForceBuy),
                Some(EconomyBuy::Eco),
            ),
            Some(LiveCriticalReason::EconomyClash)
        );
        // 复盘对无决策记录回合（经济未知 None）→ 只按比分判定，不误报 EconomyClash。
        assert_eq!(
            LiveSessionStore::critical_reason(5, 3, None, None),
            None,
            "经济未知（None）不触发 EconomyClash"
        );
    }
}
