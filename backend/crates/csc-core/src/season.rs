//! 赛季导演（Season Director，Kotlin `SeasonDirector.kt` 转写）——
//! **世界推进编排**：把「一个月怎么推进」组织为阶段管线，并持有推进状态
//! （月计数、合同结转锚点、确定性 RNG）。
//!
//! ```text
//! advance_month()
//! 1. 时钟进新月 + 清上月锁（SeasonLoop::clear_locks）
//! 2. 应用玩家**主动排定**的训练计划（若有；不再是月度强制决策）
//! 3. 【跨年时】YearlySettlement::settle（颁奖/归档/成长/人口/薪资/合同）
//! 4. SeasonLoop::run_monthly_events（月度赛事 + 队伍级互斥锁）
//! 5. vrs.reseed（滚动窗口）
//! 6. month += 1
//! 7. chemistry.monthly_recovery（队内关系收敛）
//! 8. WorldDecisionBatch::run（伤病tick + 转会/代言决策批次）
//! ```

use csc_decision::source::DecisionSource;
use csc_events::event::WorldEvent;
use csc_simulation::training::{TrainingContext, TrainingFocus, TrainingModel};
use csc_systems::chemistry::ChemistryEngine;
use csc_tournaments::calendar::SeasonCalendar;
use csc_util::rng::Xoshiro256StarStar;

use crate::batch::WorldDecisionBatch;
use crate::context::WorldContext;
use crate::loop_::SeasonLoop;
use crate::settlement::YearlySettlement;
use crate::state::GameState;

/// 赛季导演：推进状态（月计数/合同锚点/RNG）+ 阶段管线。
#[derive(Debug, Clone)]
pub struct SeasonDirector {
    /// 当前赛季月份计数（0 起，每次 `advance_month` +1；真实日期见 clock）
    pub month: i32,
    /// 上次合同结转的年份（跨年检测锚点）
    pub last_contract_year: i32,
    /// 确定性随机源（引擎唯一随机源，可存档/恢复）
    pub rng: Xoshiro256StarStar,
}

impl SeasonDirector {
    /// Seed 时间窗口回看月数（官方默认最近 6 个月）。
    pub const SEED_MONTHS: i64 = 6;
    /// 转会窗间隔（模拟月数）：每 4 个月一次。
    pub const TRANSFER_WINDOW_MONTHS: i32 = 4;
    /// 每年模拟月数。
    pub const MONTHS_PER_YEAR: i32 = 12;

    /// 新建导演（月计数 0、合同锚点 = 时钟当前年、确定性 RNG）。
    pub fn new(clock_year: i32, rng: Xoshiro256StarStar) -> Self {
        Self {
            month: 0,
            last_contract_year: clock_year,
            rng,
        }
    }

    /// 从快照恢复推进状态（月份/合同锚点/RNG；锁与其余状态由 Engine 恢复）。
    pub fn restore(&mut self, state: &GameState) {
        self.month = state.month;
        self.last_contract_year = state.last_contract_year;
        self.rng = Xoshiro256StarStar::from_state(state.rng_state);
    }

    /// RNG 状态快照（4×u64）。
    pub fn rng_snapshot(&self) -> [u64; 4] {
        self.rng.snapshot()
    }
}

/// 应用玩家排定的主动训练计划（若有）——训练计划存在状态里（存档可复现），
/// 因此**不需要决策源回灌**：回放时同样读到 pending_training 并在同一月应用。
/// 随机消费与旧月度训练决策一致（1 次 RNG，硬训练判定）。
fn apply_pending_training(ctx: &mut WorldContext<'_>, rng: &mut Xoshiro256StarStar) {
    // 先收集（避开对 arena 的可变借用在迭代内失效）
    let pending: Vec<(csc_util::id::PlayerId, String, String)> = ctx
        .world
        .all_players_only()
        .iter()
        .filter_map(|p| {
            p.career
                .as_ref()
                .and_then(|c| c.pending_training.as_ref())
                .map(|focus| (p.id, p.name.clone(), focus.clone()))
        })
        .collect();
    for (pid, name, focus_name) in pending {
        let focus = TrainingFocus::from_name(&focus_name);
        let cohesion = ctx
            .world
            .player(pid)
            .and_then(|pc| pc.team)
            .and_then(|tid| ctx.world.team(tid))
            .map(|t| t.chemistry.cohesion)
            .unwrap_or(50.0);
        let season_goal = ctx
            .world
            .player(pid)
            .and_then(|pc| pc.career.as_ref())
            .and_then(|c| c.season_goal);
        if let Some(pc) = ctx.world.player_mut(pid) {
            let _outcome = TrainingModel::apply_training(
                pc,
                focus,
                rng,
                TrainingContext {
                    team_cohesion: cohesion,
                    season_goal,
                },
            );
            if let Some(career) = pc.career_mut() {
                career.pending_training = None;
            }
        }
        ctx.journal.record(WorldEvent::TrainingDone {
            date: ctx.clock.date_label(),
            seq: -1,
            player_name: name,
            player_id: pid,
            focus: focus.name().to_string(),
        });
    }
}

/// 把 VRS 数据库的实时排名/积分回写 `Team.vrs_ranking / vrs_value` 缓存。
///
/// **2026 修复（VRS 未接入）**：此前这两个缓存只在世界初始化时写入一次，
/// 赛后与 reseed 从不回写——前端/世界故事看到的排名永远静止，用户感知
/// 「战队排名基本不变」。每月赛事 + reseed 后调用本函数单向刷新。
pub fn sync_vrs_cache(world: &mut csc_entities::world::World, vrs: &csc_vrs::engine::VrsEngine) {
    let pending: Vec<(csc_util::id::TeamId, String)> = world
        .teams
        .iter()
        .map(|t| (t.id, world.signature_of(t.id)))
        .collect();
    for (tid, sig) in pending {
        let Some(st) = vrs.team_of(&sig) else {
            eprintln!("[Season] VRS 缓存未匹配队伍签名「{sig}」——保留现有排名缓存");
            continue;
        };
        let Some(team) = world.team_mut(tid) else {
            continue;
        };
        team.vrs_ranking = st.ranking;
        team.vrs_value = st.points;
    }
}

/// 进入新月并登记新月赛事为 `Future`（LIVE 闸门两阶段拆解的阶段 1），**不结算**。
/// 全部随机消费引擎内部确定性 RNG；外部输入只经决策点管线。
/// 决策源返回非法选项 → 传播 `Err(SimError::Protocol)`（不 panic）。
///
/// 月推进（[`Self::advance_month`]）在 plan 后立即 settle（快进）；日级推进
/// （`Engine::advance_day` 月末分支）只 plan、月出口才 settle，保证比赛日当天
/// 主角 pending 对阵跨 HTTP 调用可观测。
pub fn advance_month_plan(
    director: &mut SeasonDirector,
    loop_: &mut SeasonLoop,
    ctx: &mut WorldContext<'_>,
    calendar: &SeasonCalendar,
    _decision: &mut dyn DecisionSource,
) -> Result<(), csc_util::SimError> {
    ctx.clock.next_month_start(); // 日期线程：进入新月（1 号）
    loop_.clear_locks(ctx.world); // 跨月清锁：上月赛事全部结束
    apply_pending_training(ctx, &mut director.rng); // 主动训练计划：月首生效并影响本月赛事
    if ctx.clock.year() > director.last_contract_year {
        // 跨年结算（业务契约顺序见 YearlySettlement）
        YearlySettlement::settle(ctx, director.last_contract_year, &mut director.rng);
        director.last_contract_year = ctx.clock.year();
    }
    loop_.plan_month(
        // 按日历排期登记本月全部赛事为 Future（不结算）
        ctx,
        calendar,
        director.month,
        &mut director.rng,
    );
    Ok(())
}

/// 月推进收尾：VRS 观察/reseed + 月计数 + 关系收敛 + 世界决策批次。
/// 在 plan（与 settle，月推进时）之后调用，保证赛事按各自开始日结算后世界批次
/// 在统一的月边界执行（伤病/转会/代言时间戳一致）。
///
/// @param restore_to_end 是否把时钟收束到本月最后一天：
///   - `true`（月推进/快进路径）：整月一步走完，时钟收束到月终（旧语义）——
///     赛事全部结算后再收束到月终，时间戳统一到月终而非最后一场赛事的开始日。
///   - `false`（日级跨月路径）：**不收束时钟**，保留当前日期（新月 1 号，
///     由调用方先回拨），让后续 `advance_day` 逐日经过新月每一天——否则新月
///     赛事从登记到月终一次性跳过，其比赛日从未被逐日经过，`live/today` 恒空、
///     LIVE 20s 提示条永不触发（t16 根因）。
pub fn advance_month_finish(
    director: &mut SeasonDirector,
    _loop_: &mut SeasonLoop,
    ctx: &mut WorldContext<'_>,
    decision: &mut dyn DecisionSource,
    restore_to_end: bool,
) -> Result<(), csc_util::SimError> {
    // 时钟收束到本月最后一天（2026 修复）：旧实现停在最后一场 T3 的 26 日，
    // 导致伤病/转会/代言/生活事件全部打上「26 日」时间戳，世界新闻像同一天发生。
    // 赛事自身的日期仍保留在各赛事开始日；只有月度世界批次使用月终日。
    // 仅月推进（restore_to_end=true）收束到月终；日级跨月（false）保留新月 1 号。
    if restore_to_end {
        let (year, month, _) = ctx.clock.now();
        ctx.clock
            .restore_to(year, month, csc_time::civil::days_in_month(year, month));
    }
    // VRS 观察（2026 复审修复）：reseed 前记录主角队伍旧排名，结算后排名
    // 变化时只发**数值变动**事件（#128 → #127）；无变化不发——不再每天刷
    // 一条固定文案的「VRS 观察」文字直播。
    let (vrs_old_rank, vrs_old_team) = {
        let p = ctx
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired);
        match p.and_then(|p| p.team).and_then(|tid| ctx.world.team(tid)) {
            Some(t) => (Some(t.vrs_ranking), Some(t.name.clone())),
            None => (None, None),
        }
    };
    ctx.vrs
        .reseed(&ctx.clock.rolling_window(SeasonDirector::SEED_MONTHS)); // 滚动窗口 reseed：历史比赛按时间衰减
    sync_vrs_cache(ctx.world, ctx.vrs); // VRS 实时排名 → 队伍展示缓存（2026 修复：排名不再静止）
    if let (Some(old_rank), Some(team_name)) = (vrs_old_rank, vrs_old_team) {
        let new_rank = ctx
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .and_then(|p| p.team)
            .and_then(|tid| ctx.world.team(tid))
            .map(|t| t.vrs_ranking);
        if let Some(new_rank) = new_rank {
            let detail = if new_rank != old_rank {
                ctx.text.format(
                    "season.vrs.changed",
                    &[&team_name, &old_rank.to_string(), &new_rank.to_string()],
                )
            } else {
                ctx.text
                    .format("season.vrs.same", &[&team_name, &new_rank.to_string()])
            };
            ctx.journal.record(WorldEvent::LiveUpdate {
                date: ctx.clock.date_label(),
                seq: -1,
                headline: ctx.text.get("season.vrs.headline").to_string(),
                detail,
            });
        }
    }
    director.month += 1;
    ChemistryEngine::monthly_recovery(ctx.world); // 关系向默认收敛（时间治愈一切）
    WorldDecisionBatch::run(
        // 伤病tick + 世界级决策批次
        ctx,
        decision,
        director.month,
        &mut director.rng,
        director.month % SeasonDirector::TRANSFER_WINDOW_MONTHS == 0,
    )?;
    // 世界批次里的 NPC 转会会迁移 VRS 签名与排名；结束后必须再次回写缓存，
    // 否则 128 队规模下（NPC 互换频繁）Team.vrs_ranking 会落后一个转会窗。
    sync_vrs_cache(ctx.world, ctx.vrs);
    Ok(())
}

/// 推进一个月（月推进/快进路径）：先结算当前月（R2.1，含开局月物化的赛事，
/// 消除 future 滞留），再登记新月为 `Future` 并立即结算（阶段 1 + 阶段 2）。
/// 保持既有 `advance_month` 的调用语义与确定性；日级推进走 `Engine::advance_day`。
pub fn advance_month(
    director: &mut SeasonDirector,
    loop_: &mut SeasonLoop,
    ctx: &mut WorldContext<'_>,
    calendar: &SeasonCalendar,
    decision: &mut dyn DecisionSource,
) -> Result<(), csc_util::SimError> {
    // R2.1：进新月**之前**先结算当前月——`settle_pending_events` 按时钟当前月
    // 前缀匹配，晚于此步会永远漏掉开局月（如 2026-01）物化的记录。
    loop_.settle_month(ctx, decision, &mut director.rng);
    advance_month_plan(director, loop_, ctx, calendar, decision)?;
    loop_.settle_month(ctx, decision, &mut director.rng);
    advance_month_finish(director, loop_, ctx, decision, true)
}

/// 连续推进 `months` 个月。
pub fn run_season(
    director: &mut SeasonDirector,
    loop_: &mut SeasonLoop,
    ctx: &mut WorldContext<'_>,
    calendar: &SeasonCalendar,
    decision: &mut dyn DecisionSource,
    months: i32,
) -> Result<(), csc_util::SimError> {
    for _ in 0..months {
        advance_month(director, loop_, ctx, calendar, decision)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use csc_entities::world::World;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn director_state_roundtrip() {
        let mut d = SeasonDirector::new(2026, Xoshiro256StarStar::seed(42));
        d.month = 7;
        d.last_contract_year = 2027;
        let mut rng2 = Xoshiro256StarStar::seed(1);
        rng2.next_u64();
        d.rng = rng2;
        let state = GameState {
            version: GameState::CURRENT_VERSION,
            month: d.month,
            last_contract_year: d.last_contract_year,
            locks: vec![],
            sim_year: 2026,
            sim_month: 6,
            sim_day: 1,
            rng_state: d.rng_snapshot(),
            decisions: vec![],
            journal: vec![],
            archive: Default::default(),
            world: World::new(),
            vrs: csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: Default::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: crate::state::WORLD_SIM_VERSION,
            narrative: Default::default(),
            execution: crate::state::ExecutionState::Idle,
        };
        let mut d2 = SeasonDirector::new(2026, Xoshiro256StarStar::seed(0));
        d2.restore(&state);
        assert_eq!(d2.month, 7);
        assert_eq!(d2.last_contract_year, 2027);
        assert_eq!(d2.rng_snapshot(), state.rng_state);
        // 恢复后推进与原始推进一致（可复现性）
        let mut a = Xoshiro256StarStar::from_state(state.rng_state);
        let mut b = d2.rng;
        assert_eq!(a.next_u64(), b.next_u64());
    }
}
