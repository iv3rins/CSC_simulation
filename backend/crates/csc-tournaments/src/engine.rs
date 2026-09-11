//! 赛事子系统 sub-engine（Kotlin `TournamentEngine.kt` 转写）：整个赛事的模拟
//! （排程 → 邀请 → 逐场 series 模拟 → 结算编排）。
//!
//! **职责边界**：本引擎只做**编排**——
//! - 排程门面：委托 [`crate::scheduler::TournamentScheduler`]；
//! - 赛事运行：按 `Tournament.format` 分派赛制引擎（瑞士轮/双败/单败），
//!   每场对局经 `MatchRunner` 注入 [`crate::conductor::run_lod_series`]
//!   （对局级 LOD + 玩家队伍的图间决策循环）；
//! - 编排结算：委托 [`crate::settlement::SeriesSettlement`]（生涯回写/疲劳/奖金/荣誉/年度）；
//! - 比赛级决策（场内干预/队友失误）完全在 conductor，本引擎零感知；
//! - 账目（结算公式）完全在 settlement，本引擎零公式。
//!
//! 转写差异：Kotlin 构造注入全部依赖 → Rust 只持有**跨调用状态**
//! （`TournamentScheduler` 排程产物 + `SeriesSettlement` 年度累计器 +
//! 已模拟赛事列表 + 最近排程），world/vrs/clock/journal/决策源全部方法参数化。

use csc_decision::log::DecisionLog;
use csc_decision::source::DecisionSource;
use csc_domain::tier_profile::tier_best_of;
use csc_domain::tournament::Tournament;
use csc_domain::tournament_format::TournamentFormat;
use csc_domain::tourney_tier::TourneyTier;
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::series::{SeriesResult, SeriesStage};
use csc_time::clock::SimClock;
use csc_util::id::TeamId;
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::conductor::run_lod_series_with_importance;
use crate::format::bracket::{BracketResult, TeamEntry};
use crate::format::double_elim::DoubleElimGroupsBracket;
use crate::format::single_elim::SingleElimPlayoff;
use crate::format::swiss::SwissStage;
use crate::format::swiss_playoff::SwissPlayoffBracket;
use crate::scheduled::{
    FixtureStatus, ScheduledStatus, ScheduledTournament, ScheduledTournamentRecord,
    TournamentResult,
};
use crate::scheduler::{RankedTeam, TournamentScheduler};
use crate::settlement::SeriesSettlement;

/// 赛事子系统 sub-engine：编排壳。
pub struct TournamentEngine {
    /// 排程器（排程产物状态）
    scheduler: TournamentScheduler,
    /// 结算层（年度 Rating 累计器等跨调用状态）
    settlement: SeriesSettlement,
    /// 已完整模拟的赛事（含冠军与全部系列赛；存档时并入 GameState）
    events: Vec<TournamentResult>,
    /// 最近一次 `simulate_event` 生成的参赛名单（Engine 用于并行日历的队伍级互斥锁定）
    last_scheduled: Option<ScheduledTournament>,
    /// 已排程确认赛事的权威记录（P1-1 future 赛程事实源；存档时并入 GameState）
    scheduled_records: Vec<ScheduledTournamentRecord>,
}

impl Default for TournamentEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TournamentEngine {
    pub fn new() -> Self {
        Self {
            scheduler: TournamentScheduler::new(),
            settlement: SeriesSettlement::default(),
            events: Vec::new(),
            last_scheduled: None,
            scheduled_records: Vec::new(),
        }
    }

    /// 从存档恢复（GameState 还原）：结算层（年度累计器 + 历届 TOP20 榜单）
    /// + 已完赛结果 + 排程记录。排程产物为临时状态（恢复后由 Engine 重新排程），不存档。
    pub fn restore_from(
        tracker: crate::yearly_rating::YearlyRatingTracker,
        top20_history: Vec<crate::top20::Top20YearBoard>,
        events: Vec<TournamentResult>,
        scheduled_records: Vec<ScheduledTournamentRecord>,
    ) -> Self {
        let mut settlement = SeriesSettlement::default();
        settlement.restore_yearly_rating(tracker);
        settlement.restore_top20_history(top20_history);
        Self {
            scheduler: TournamentScheduler::new(),
            settlement,
            events,
            last_scheduled: None,
            scheduled_records,
        }
    }

    // —— 排程门面（委托排程器）——

    /// 全部已排程赛事。
    pub fn scheduled_tournaments(&self) -> Vec<ScheduledTournament> {
        self.scheduler.scheduled_tournaments()
    }

    /// 指定等级赛事已排程的参赛队伍（直邀 + 预选）。
    pub fn participants_of(&self, tier: TourneyTier) -> Vec<TeamId> {
        self.scheduler.participants_of(tier)
    }

    /// 已完整模拟的全部赛事结果。
    pub fn results(&self) -> Vec<TournamentResult> {
        self.events.clone()
    }

    /// 已完整模拟的赛事结果只读切片（**零 clone**）——客户端视图等只读派生
    /// 不应为「看历史」支付全量 clone（20 年末全量 clone ≈72ms/次）。
    pub fn results_ref(&self) -> &[TournamentResult] {
        &self.events
    }

    /// 已完整模拟的赛事数量（**只读计数，不 clone 结果**——world_summary 高频调用，
    /// 此前经 `results().len()` 每次全量 clone 全部赛事对象，20 年末单次 72ms）。
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// 结算层只读引用（编排层仅经此观察结算状态，不再接触私有字段）。
    pub fn settlement(&self) -> &SeriesSettlement {
        &self.settlement
    }

    /// 结算层可变引用（仅同 crate 编排/测试构造用）。
    pub fn settlement_mut(&mut self) -> &mut SeriesSettlement {
        &mut self.settlement
    }

    /// 年度 Rating 累计器只读快照（存档/查询的越层读取口；P1 收窄）。
    pub fn yearly_rating_snapshot(&self) -> crate::yearly_rating::YearlyRatingTracker {
        self.settlement.yearly_rating_snapshot()
    }

    /// 历届年度 TOP20 榜单快照（存档/查询；含 NPC 名次与入选依据）。
    pub fn top20_history(&self) -> Vec<crate::top20::Top20YearBoard> {
        self.settlement.top20_history().to_vec()
    }

    /// 从存档恢复结算层（读档边界；P1 收窄后的唯一写入口）。
    pub fn restore_settlement(&mut self, tracker: crate::yearly_rating::YearlyRatingTracker) {
        self.settlement.restore_yearly_rating(tracker);
    }

    /// 已排程确认赛事的权威记录快照（P1-1 future 赛程；存档时并入 GameState）。
    pub fn scheduled_records_snapshot(&self) -> Vec<ScheduledTournamentRecord> {
        self.scheduled_records.clone()
    }

    /// 把**今天**（开赛日 == today）主角队伍全部 pending 对阵标记为 `skipped`。
    ///
    /// 这是「跳过比赛日 LIVE 提示」的**持久语义**：返回被标记的 fixture 计数。
    /// 语义边界（确定性关键）：
    /// - 只置 `skipped` 标志，**不**改 `status`、**不**回填 `result`、**不**碰 RNG——
    ///   比赛结果仍由月末 `settle_month` 走确定性后台模拟结算（跳过的比赛照常出比分，
    ///   杜绝「跳过即丢结果 / 图数不涨」）。
    /// - `player_live_fixtures`（`live/today`）据此不再返回这些对阵 → 前端跳过/关闭后
    ///   20s 提示条不再重新弹出（闭环解除），而日期可继续推进。
    /// - 非比赛日/无主角对阵时返回 0（no-op）。
    pub fn skip_live_today(
        &mut self,
        protagonist_team: TeamId,
        year: i32,
        month: u32,
        day: u32,
    ) -> usize {
        let today = format!("{year:04}-{month:02}-{day:02}");
        let mut skipped = 0usize;
        for rec in &mut self.scheduled_records {
            if rec.player_team_id != Some(protagonist_team) {
                continue;
            }
            if matches!(
                rec.status,
                ScheduledStatus::Completed | ScheduledStatus::Cancelled
            ) {
                continue;
            }
            if rec.date != today {
                continue;
            }
            for f in &mut rec.fixtures {
                if f.skipped {
                    continue;
                }
                if f.status != FixtureStatus::Pending {
                    continue;
                }
                if f.team_b == TeamId::NONE {
                    continue; // 轮空无比赛
                }
                f.skipped = true;
                skipped += 1;
            }
        }
        skipped
    }

    /// 最近一次 `simulate_event` 生成的参赛队伍（Engine 用于并行日历的队伍级互斥锁定）。
    pub fn last_event_teams(&self) -> Vec<TeamId> {
        self.last_scheduled
            .as_ref()
            .map(|s| s.all_participants())
            .unwrap_or_default()
    }

    /// R2：把指定赛事对局标记为「已看完」（`watched` 持久语义，幂等）。
    ///
    /// 按 `event_name` + `fixture_id` 精确定位（或 team 对兜底），置 `watched=true`。
    /// 返回是否命中（幂等：重复标记返回 false 表示「早已看过」，仍算成功）。
    /// 只改 `watched` 标志，不碰 `status`/`result`/RNG——纯展示语义，不影响结算。
    pub fn mark_fixture_watched(&mut self, event_name: &str, fixture_id: u32) -> bool {
        for rec in &mut self.scheduled_records {
            if rec.event_name != event_name {
                continue;
            }
            for f in &mut rec.fixtures {
                if f.fixture_id == fixture_id {
                    f.watched = true;
                    return true;
                }
            }
        }
        false
    }

    // —— 赛事运行 ——

    /// 模拟并结算一场系列赛（bo1/bo3/bo5，含逐图比分与全员 KDA）。
    ///
    /// 流程：调 `MatchSimulator::simulate_series` 生成系列赛，然后委托
    /// `SeriesSettlement::settle_series` 结算（生涯回写 + 疲劳 + 日志 + VRS 积分）。
    /// 注意：本方法走**无干预**路径（外部直接调用时不产生决策点）；
    /// 赛制引擎内的对局经 [`run_lod_series`]（含玩家队伍的图间决策）。
    ///
    /// 结构拆分：`run_match`/`run_tournament` 便捷壳已删除（零生产调用方；
    /// 生产路径直接走 `run_tournament_detailed`），本方法与 `simulate_event`
    /// 是赛事模拟的两个真实入口。
    #[allow(clippy::too_many_arguments)]
    pub fn run_series(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        tier: TourneyTier,
        team_a: TeamId,
        team_b: TeamId,
        best_of: i32,
        rng: &mut Xoshiro256StarStar,
    ) -> SeriesResult {
        let sig_a = world.signature_of(team_a);
        let sig_b = world.signature_of(team_b);
        let ta = csc_simulation::match_simulator::SeriesTeam {
            id: team_a,
            signature: sig_a,
            roster: world.roster(team_a),
            cohesion: csc_systems::chemistry::ChemistryEngine::cohesion_factor_of(world, team_a),
        };
        let tb = csc_simulation::match_simulator::SeriesTeam {
            id: team_b,
            signature: sig_b,
            roster: world.roster(team_b),
            cohesion: csc_systems::chemistry::ChemistryEngine::cohesion_factor_of(world, team_b),
        };
        let series = csc_simulation::match_simulator::MatchSimulator::simulate_series(
            tier,
            ta,
            tb,
            best_of,
            rng,
            SeriesStage::Unknown,
        );
        self.settlement
            .settle_series(world, vrs, clock, journal, &series, "match");
        series
    }

    /// 运行赛事并返回完整结果（冠军 + 全部系列赛 + 折算比分）。
    /// 按 `ScheduledTournament.format` 分派到赛制引擎（瑞士轮/双败/单败）。
    ///
    /// @param event_name 赛事真实全名（比赛日志与决策点的叙事归属；不是
    ///                   `ScheduledTournament::to_event` 的泛化「{tier} Event」名）
    /// @param first_round_fixtures 赛前物化的首轮对阵（T3；SingleElim 时传入以
    ///                             保证运行时配对与预览一致；None = 历史随机序）
    /// @return None 当参赛名单为空
    #[allow(clippy::too_many_arguments)]
    pub fn run_tournament_detailed(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        scheduled: &ScheduledTournament,
        event_name: &str,
        rng: &mut Xoshiro256StarStar,
        first_round_fixtures: Option<&[crate::scheduled::ScheduledFixture]>,
    ) -> Option<TournamentResult> {
        if scheduled.all_participants().is_empty() {
            return None;
        }
        let bracket = self.run_bracket(
            world,
            vrs,
            clock,
            journal,
            decision,
            decision_log,
            scheduled,
            event_name,
            rng,
            first_round_fixtures,
        );
        // 赛事收官：把 VRS 实时排名同步回队伍快照（供下一轮排程使用）——
        // Rust 侧 Team.vrs_ranking 缓存由 VRS 结算直接刷新，此处无额外动作
        Some(TournamentResult {
            event: scheduled.to_event_named(event_name),
            champion: bracket.champion.id,
            matches: bracket.matches,
            series: bracket.series,
        })
    }

    // —— 整个赛事模拟（接入 Tournament 模型：赛事 → series 模拟）——

    /// 模拟一场完整赛事：按 [`Tournament`] 描述的邀请策略生成参赛名单，跑赛制，
    /// 记录冠军与全部系列赛，结果加入 [`Self::results`]。
    ///
    /// 该入口在调用瞬间生成月首资格快照；月度日历请走
    /// [`Self::simulate_event_with_ranking`]（整个月共用同一快照）。
    #[allow(clippy::too_many_arguments)]
    pub fn simulate_event(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        event: &Tournament,
        rng: &mut Xoshiro256StarStar,
    ) -> TournamentResult {
        let ranking = Self::ranking_snapshot(world, vrs);
        self.simulate_event_with_ranking(
            world,
            vrs,
            clock,
            journal,
            decision,
            decision_log,
            event,
            &ranking,
            rng,
        )
    }

    /// 用**调用方提供的月首资格快照**排定一场赛事并登记为 `Future` 权威记录
    /// （LIVE 闸门两阶段拆解的**阶段 1：登记，不结算**）。
    ///
    /// 只在排程/登记阶段消费排程 RNG；**模拟 RNG 在 `settle_pending_events`（阶段 2）
    /// 消费**。参赛名单一经排程器确认即写入 `scheduled_records`（future 赛程事实源），
    /// 生命周期状态在 `settle_pending_events` 中推进到 Completed。这样比赛日当天
    /// 该记录以 `Future` + pending fixtures 存在，`live/today` 可观测。
    #[allow(clippy::too_many_arguments)]
    pub fn plan_event_with_ranking(
        &mut self,
        world: &mut World,
        _vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        _decision_log: &mut DecisionLog,
        event: &Tournament,
        ranking: &[RankedTeam],
        rng: &mut Xoshiro256StarStar,
    ) -> bool {
        let is_team_free = |id: TeamId| {
            world
                .team(id)
                .map(|t| t.current_tournament_id.is_none())
                .unwrap_or(false)
        };
        // P0-B：定位主角队伍并保证其 T3 入赛（项目统一约定见注释）。
        let player_team = world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .and_then(|p| p.team);
        let scheduled =
            self.scheduler
                .schedule_for(&is_team_free, ranking, event, rng, player_team);
        self.last_scheduled = Some(scheduled.clone());

        // R2.1 排程去重（防御性护栏）：同名赛事（如开局月物化与常规月重排撞名）
        // 已存在未完结记录时直接跳过——旧实现会再 push 一条重复实例，造成
        // player_events 里同名 future/active 交错并存（t5 `#0--7` 证据）。
        // 返回 false 通知调用方跳过 lock_event（避免用新名单覆盖旧锁）。
        // 正常情况下日历命名已按月唯一（m 单调递增），本守卫只在修复前旧存档
        // 或异常排程时序下兜底，不改变正常路径的确定性。
        if self
            .scheduled_records
            .iter()
            .any(|r| r.event_name == event.name && r.status == ScheduledStatus::Future)
        {
            return false;
        }

        // P1-1：排程确认时点登记权威排程记录（future 赛程事实源）。
        let player_team_id = player_team.filter(|pt| scheduled.all_participants().contains(pt));
        // T3：赛前物化首轮对阵（纯函数、确定性；不消费 RNG，不绑定引擎执行序）。
        let fixtures = crate::scheduled::first_round_fixtures(
            scheduled.format,
            &scheduled.all_participants(),
            scheduled.tier,
        );
        let record = ScheduledTournamentRecord {
            event_name: event.name.clone(),
            date: event.date.clone(),
            tier: event.tier,
            team_ids: scheduled.all_participants(),
            player_team_id,
            status: ScheduledStatus::Future,
            fixtures: fixtures.clone(),
            format: scheduled.format,
        };
        let record_idx = self.scheduled_records.len();
        self.scheduled_records.push(record);
        // 参赛不足（名单为空/过少无法配对）→ 立即标记取消（阶段 1 判定，可观察）。
        if scheduled.all_participants().is_empty() {
            self.scheduled_records[record_idx].status = ScheduledStatus::Cancelled;
            journal.record(WorldEvent::TournamentCancelled {
                date: clock.date_label(),
                seq: -1,
                event_name: event.name.clone(),
                tier: event.tier,
                reason: format!("参赛不足 / 未凑满 {} 队", 8),
            });
        }
        true
    }

    /// 结算某月已登记（`Future`）的赛事（LIVE 闸门两阶段拆解的**阶段 2：结算**）。
    ///
    /// 按 `month_prefix`（"YYYY-MM"）匹配该月登记的待结算记录，按登记顺序逐场：
    /// 重构 `ScheduledTournament` → `run_tournament_detailed` 模拟 → 回填结果 →
    /// 结算荣誉/奖金 → `status → Completed`。消费模拟 RNG（确定性，顺序固定）。
    ///
    /// 已取消（`Cancelled`）或已完成（`Completed`）的记录跳过；不匹配月份的记录跳过。
    #[allow(clippy::too_many_arguments)]
    pub fn settle_pending_events(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &mut SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        month_prefix: &str,
        rng: &mut Xoshiro256StarStar,
    ) {
        // 先收集待结算索引（避开对 self.scheduled_records 的借用冲突）。
        let pending_idx: Vec<usize> = (0..self.scheduled_records.len())
            .filter(|&i| {
                let rec = &self.scheduled_records[i];
                rec.status == ScheduledStatus::Future && rec.date.starts_with(month_prefix)
            })
            .collect();
        for i in pending_idx {
            // 推进时钟到该场开赛日再结算，保证每场结果/日志日期 = 该场开始日
            // （与月推进 `plan_month` 的 advance_to 语义一致，跨推进粒度可复现）。
            let day = rec_day(&self.scheduled_records[i].date);
            if let Some(day) = day {
                let (y, m, _) = clock.now();
                clock.restore_to(y, m, day.max(1) as u32);
            }
            self.settle_pending_event(world, vrs, &*clock, journal, decision, decision_log, i, rng);
        }
    }

    /// 结算单场已登记赛事（`settle_pending_events` 的单元实现）。
    #[allow(clippy::too_many_arguments)]
    fn settle_pending_event(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        record_idx: usize,
        rng: &mut Xoshiro256StarStar,
    ) {
        let rec = self.scheduled_records[record_idx].clone();
        if rec.status != ScheduledStatus::Future {
            return;
        }
        // 从权威记录重构 `ScheduledTournament`（排程时捕获的 format/tier/participants）。
        let scheduled = ScheduledTournament {
            tier: rec.tier,
            importance: scheduled_importance(rec.tier),
            direct_invitees: rec.team_ids.clone(),
            replacements: Vec::new(),
            qualifiers: Vec::new(),
            event_date: rec.date.clone(),
            format: rec.format,
        };
        // R2 修复：用**真实赛事名**（rec.event_name，如 `Exort Fiesta Series #1-8`）
        // 构建结算归档用的 Tournament，而非 `to_event` 的占位名 `QUALIFY Event`——
        // 保证 `attended_tournaments`/`TournamentResult.event` 与排程记录同名，
        // 前端按名关联「已完赛回放/比分/冠军」才能命中。
        let event = scheduled.to_event_named(rec.event_name.clone());
        // 参赛记录写入（在原 simulate_event_with_ranking 中于模拟前执行）。
        SeriesSettlement::record_participation(world, &scheduled.all_participants(), &event);
        let Some(detailed) = self.run_tournament_detailed(
            world,
            vrs,
            clock,
            journal,
            decision,
            decision_log,
            &scheduled,
            &rec.event_name,
            rng,
            Some(&rec.fixtures),
        ) else {
            // 参赛不足（正常情况下登记时已取消；防御性兜底）。
            self.scheduled_records[record_idx].status = ScheduledStatus::Cancelled;
            return;
        };
        // 赛事完整模拟完成 → 推进为 Completed，并按队伍匹配回填对阵结果 + 比分（T3/R2）。
        self.scheduled_records[record_idx].status = ScheduledStatus::Completed;
        backfill_fixture_results(
            &mut self.scheduled_records[record_idx],
            world,
            &detailed.matches,
            &detailed.series,
        );
        let result = TournamentResult {
            event: event.clone(),
            ..detailed
        };
        let mvp = self
            .settlement
            .award_event_honours(world, clock, journal, &result); // 冠军团队荣誉 + MVP 个人荣誉（玩家）
        self.settlement.award_prize_money(world, clock, &result); // 经济闭环：奖池奖金 → 队伍预算 + 玩家分成
        self.events.push(result.clone());
        // 世界事件日志：冠军 + 本赛事 MVP（荣誉颁发处计算，此处回填明细）
        let champion_name = world
            .team(result.champion)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        let (mvp_name, mvp_id) = mvp
            .map(|(name, id)| (Some(name), Some(id)))
            .unwrap_or((None, None));
        journal.record(WorldEvent::Championship {
            date: clock.date_label(),
            seq: -1,
            event_name: rec.event_name.clone(),
            tier: rec.tier,
            champion: champion_name,
            champion_id: result.champion,
            mvp: mvp_name,
            mvp_id,
        });
    }

    /// 兼容壳（单场 plan + settle 一步完成）：排程登记本场为 `Future`，再立即结算。
    ///
    /// 保留旧 `simulate_event_with_ranking` 的调用语义（返回 `TournamentResult`），
    /// 供 `simulate_event`（单场入口）与既有单场集成测试使用。月度日历改走
    /// [`Self::plan_event_with_ranking`] + [`Self::settle_pending_events`] 两阶段。
    #[allow(clippy::too_many_arguments)]
    pub fn simulate_event_with_ranking(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        event: &Tournament,
        ranking: &[RankedTeam],
        rng: &mut Xoshiro256StarStar,
    ) -> TournamentResult {
        // 先收集计划前的记录数，plan 后定位本次登记的记录并结算它。
        let base = self.scheduled_records.len();
        let planned = self.plan_event_with_ranking(
            world,
            vrs,
            clock,
            journal,
            decision_log,
            event,
            ranking,
            rng,
        );
        // R2.1 排程去重：同名 Future 已存在 → plan 未登记新记录，返回空占位
        // （调用方语义：本场已被早先排程持有，不得重复模拟）。
        if !planned || base >= self.scheduled_records.len() {
            return TournamentResult {
                event: event.clone(),
                champion: TeamId::NONE,
                matches: Vec::new(),
                series: Vec::new(),
            };
        }
        let record_idx = base;
        let month_prefix = event.date[..7].to_string(); // "YYYY-MM"
        // 仅当记录为 Future 且属于本场日期所在月时才结算（plan 已登记，属本月）。
        let rec = self.scheduled_records[record_idx].clone();
        if rec.status == ScheduledStatus::Future && rec.date.starts_with(&month_prefix) {
            self.settle_pending_event(
                world,
                vrs,
                clock,
                journal,
                decision,
                decision_log,
                record_idx,
                rng,
            );
        }
        // 返回本场结果（已 push 进 events 的末条；未结算则为空占位）。
        self.events.last().cloned().unwrap_or(TournamentResult {
            event: event.clone(),
            champion: TeamId::NONE,
            matches: Vec::new(),
            series: Vec::new(),
        })
    }

    // —— 年度结算（委托结算层）——

    /// 年度统计（生涯档案素材；供 Engine 跨年归档，随后 [`Self::year_end_awards`] 清空累计器）。
    pub fn yearly_stats(
        &self,
    ) -> std::collections::HashMap<csc_util::id::PlayerId, crate::yearly_rating::YearlyStat> {
        self.settlement.yearly_stats()
    }

    /// 年度结算：按本年度**场均 Rating** 排名，给前 `top` 名玩家颁发 TOP20 荣誉（委托结算层）。
    pub fn year_end_awards(
        &mut self,
        world: &mut World,
        clock: &SimClock,
        journal: &mut WorldJournal,
        year: i32,
        top: usize,
    ) -> Vec<csc_util::id::PlayerId> {
        self.settlement
            .year_end_awards(world, clock, journal, year, top)
    }

    // —— 内部实现 ——

    /// 按赛制分派到对应阶段引擎。结算回调 `MatchRunner` 注入
    /// [`run_lod_series`]（对局级 LOD + 图间决策）——赛制引擎只负责对阵推进。
    ///
    /// T3 一致性：`first_round_fixtures` 为赛前物化的首轮对阵（SingleElim 时传入，
    /// 首轮按物化配对落位；其它赛制天然种子序一致，无需传入）。
    #[allow(clippy::too_many_arguments)]
    fn run_bracket(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        decision: &mut dyn DecisionSource,
        decision_log: &mut DecisionLog,
        scheduled: &ScheduledTournament,
        event_name: &str,
        rng: &mut Xoshiro256StarStar,
        first_round_fixtures: Option<&[crate::scheduled::ScheduledFixture]>,
    ) -> BracketResult {
        let participants: Vec<TeamEntry> = scheduled
            .all_participants()
            .iter()
            .map(|tid| TeamEntry {
                id: *tid,
                signature: world.signature_of(*tid),
            })
            .collect();

        // 入口统一校验赛制队伍数约束
        if let TournamentFormat::SwissPlayoff { qualifiers, .. } = scheduled.format {
            assert!(
                participants.len() >= qualifiers as usize,
                "瑞士轮赛制需要 ≥ {qualifiers} 支参赛队（实际 {}）",
                participants.len()
            );
        }

        let event_name = event_name.to_string();
        let settlement = &mut self.settlement;
        // Major 分阶段推进：小组赛（瑞士轮）→ 淘汰赛，各阶段写里程碑日志
        // （开幕/晋级），决赛收官由 simulate_event 的 Championship 事件承接。
        if scheduled.tier == TourneyTier::Major
            && let TournamentFormat::SwissPlayoff {
                rounds,
                qualifiers,
                swiss_best_of,
                playoff_best_of,
                final_best_of,
            } = scheduled.format
        {
            let date = clock.date_label();
            journal.record(WorldEvent::TournamentStage {
                date: date.clone(),
                seq: -1,
                event_name: event_name.clone(),
                stage: "小组赛".into(),
                detail: format!("{} 支队伍瑞士轮开赛", participants.len()),
            });
            let swiss = {
                // swiss 段一定 Group（忽略 format 引擎传入的 stage，Major 专用闭包）
                let mut runner = |a: TeamId, b: TeamId, best_of: i32, _stage: SeriesStage| {
                    run_lod_series_with_importance(
                        world,
                        vrs,
                        clock,
                        decision,
                        decision_log,
                        journal,
                        settlement,
                        scheduled.tier,
                        a,
                        b,
                        best_of,
                        rng,
                        &event_name,
                        SeriesStage::Group,
                        scheduled.importance,
                    )
                };
                SwissStage::new(rounds, qualifiers as usize, swiss_best_of)
                    .run(&participants, &mut runner)
            };
            journal.record(WorldEvent::TournamentStage {
                date: date.clone(),
                seq: -1,
                event_name: event_name.clone(),
                stage: "淘汰赛".into(),
                detail: format!("小组赛收官，{} 支队伍晋级淘汰赛", swiss.qualified.len()),
            });
            let playoff = {
                // 淘汰段一定 Playoff
                let mut runner = |a: TeamId, b: TeamId, best_of: i32, _stage: SeriesStage| {
                    run_lod_series_with_importance(
                        world,
                        vrs,
                        clock,
                        decision,
                        decision_log,
                        journal,
                        settlement,
                        scheduled.tier,
                        a,
                        b,
                        best_of,
                        rng,
                        &event_name,
                        SeriesStage::Playoff,
                        scheduled.importance,
                    )
                };
                SingleElimPlayoff::new(playoff_best_of, final_best_of)
                    .run(&swiss.qualified, &mut runner)
            };
            let mut series = swiss.series;
            series.extend(playoff.series);
            return BracketResult {
                champion: playoff.champion,
                series: series.clone(),
                matches: series.iter().map(|s| s.to_match_result()).collect(),
            };
        }

        // 首轮随机配对预打乱（Kotlin `shuffled(random)` 语义；随机消费在闭包创建**前**）
        let mut shuffled_field: Vec<TeamEntry> = participants.clone();
        for i in (1..shuffled_field.len()).rev() {
            let j = rng.next_i32_bound((i + 1) as i32) as usize;
            shuffled_field.swap(i, j);
        }
        // 结算回调：conductor 图间决策循环（决策源/日志/结算层/rng 全在此闭包捕获）
        let mut match_runner =
            move |a: TeamId, b: TeamId, best_of: i32, stage: SeriesStage| -> SeriesResult {
                run_lod_series_with_importance(
                    world,
                    vrs,
                    clock,
                    decision,
                    decision_log,
                    journal,
                    settlement,
                    scheduled.tier,
                    a,
                    b,
                    best_of,
                    rng,
                    &event_name,
                    stage,
                    scheduled.importance,
                )
            };

        match scheduled.format {
            TournamentFormat::SingleElim => legacy_single_elim(
                &shuffled_field,
                scheduled.tier,
                &mut match_runner,
                first_round_fixtures,
            ),
            TournamentFormat::SwissPlayoff {
                rounds,
                qualifiers,
                swiss_best_of,
                playoff_best_of,
                final_best_of,
            } => SwissPlayoffBracket::new(
                rounds,
                qualifiers as usize,
                swiss_best_of,
                playoff_best_of,
                final_best_of,
            )
            .run(&participants, &mut match_runner),
            TournamentFormat::DoubleElimGroups {
                groups,
                group_best_of,
                playoff_best_of,
                final_best_of,
            } => DoubleElimGroupsBracket::new(
                groups as usize,
                group_best_of,
                playoff_best_of,
                final_best_of,
            )
            .run(&participants, &mut match_runner),
        }
    }

    /// 月首资格快照（按 VRS 排名升序；空 VRS 库时按队伍缓存排名）。
    /// 返回 `(排名, 积分, TeamId)`，是整个月 T1/T2/T3 排程的**唯一资格来源**。
    pub fn ranking_snapshot(world: &World, vrs: &VrsEngine) -> Vec<RankedTeam> {
        if !vrs.all_teams().is_empty() {
            // VRS 库非空：VRS 排名的实体映射
            let mut out: Vec<RankedTeam> = vrs
                .all_teams()
                .iter()
                .filter_map(|st| {
                    let sig = VrsEngine::signature_of(&st.team_name, &st.roster);
                    world
                        .teams
                        .iter()
                        .find(|t| world.signature_of(t.id) == sig)
                        .map(|t| (st.ranking, st.points, t.id))
                })
                .collect();
            out.sort_by_key(|(r, _, _)| *r);
            out
        } else {
            // 空 VRS 库：按队伍缓存排名（世界初始化时的快照）
            let mut out: Vec<RankedTeam> = world
                .teams
                .iter()
                .map(|t| (t.vrs_ranking, t.vrs_value, t.id))
                .collect();
            out.sort_by_key(|(r, _, _)| *r);
            out
        }
    }
}

/// 由赛事等级推导玩家体验重要度（与 [`ScheduledTournament::build_event`] /
/// `conductor::run_lod_series` 同口径，唯一实现见 [`tier_importance`]；
/// 本函数仅为别名，用于结算阶段从权威记录重构 `ScheduledTournament`）。
#[inline]
fn scheduled_importance(tier: TourneyTier) -> csc_domain::event_importance::EventImportance {
    csc_domain::tier_profile::tier_importance(tier)
}

/// 从 "YYYY-MM-DD" 记录日期解析日号（供结算前把时钟推进到该场开赛日）。
/// 解析失败返回 `None`（调用方保持当前时钟）。
fn rec_day(date: &str) -> Option<i32> {
    let day = date.rsplit('-').next()?;
    day.parse::<i32>().ok()
}

/// 赛事完整模拟后，按队伍 ID 匹配把实际结果回填到赛前物化的对阵（T3）。
///
/// 签名 → 队伍 ID 反查用 `world.signature_of`（与排程/结算同源）；
/// 只回填能匹配到的 fixture（SingleElim 运行时随机配对可能与预览不同，
/// 未匹配的保持 pending，不影响状态机）。
///
/// R2 扩展：除 `result`（胜/负签名）外，另从系列赛逐图比分回填 `score`
/// （HLTV `13:12` / 系列 `2:0` 样式），修复「对局无比分可看」。
fn backfill_fixture_results(
    record: &mut crate::scheduled::ScheduledTournamentRecord,
    world: &World,
    matches: &[csc_domain::match_result::MatchResult],
    series: &[csc_simulation::series::SeriesResult],
) {
    if record.fixtures.is_empty() {
        return;
    }
    let id_of_sig = |sig: &str| -> Option<TeamId> {
        world
            .all_teams()
            .iter()
            .find(|t| world.signature_of(t.id) == sig)
            .map(|t| t.id)
    };
    for m in matches {
        let (Some(wa), Some(lb)) = (id_of_sig(&m.winner_sig), id_of_sig(&m.loser_sig)) else {
            continue;
        };
        let hit = record.fixtures.iter_mut().find(|f| {
            !f.is_bye()
                && f.result.is_none()
                && ((f.team_a == wa && f.team_b == lb) || (f.team_a == lb && f.team_b == wa))
        });
        if let Some(f) = hit {
            f.result = Some(m.clone());
            f.status = crate::scheduled::FixtureStatus::Completed;
            f.score = backfill_fixture_score(f, series);
        }
    }
}

/// 从系列赛逐图比分推导某 fixture 的比分（以 fixture team_a/team_b 方向表达）。
///
/// - BO1：取该系列唯一/决定性图的回合比分（`13:12` 样式）；
/// - BO3/BO5：取系列胜场数（`2:0` / `3:2` 样式）；
/// - 找不到对应系列（旧存档/异常）→ None（前端回退无比分展示）。
fn backfill_fixture_score(
    f: &crate::scheduled::ScheduledFixture,
    series: &[csc_simulation::series::SeriesResult],
) -> Option<crate::scheduled::FixtureScore> {
    // 按 team 对匹配系列（方向无关）。
    let s = series.iter().find(|s| {
        (s.team_a_id == f.team_a && s.team_b_id == f.team_b)
            || (s.team_a_id == f.team_b && s.team_b_id == f.team_a)
    })?;
    let a_won_maps = s
        .maps
        .iter()
        .filter(|m| s.team_a_sig == m.winner_sig)
        .count() as i32;
    let b_won_maps = s.maps.len() as i32 - a_won_maps;
    if f.best_of <= 1 {
        // BO1：单图回合比分（决定性图 = 最后一张）。
        let last = s.maps.last()?;
        // last 的 team_a_score/team_b_score 是系列 team_a/team_b 方向；换算到 fixture 方向。
        let (a_score, b_score) = if s.team_a_id == f.team_a {
            (last.team_a_score, last.team_b_score)
        } else {
            (last.team_b_score, last.team_a_score)
        };
        Some(crate::scheduled::FixtureScore {
            team_a_score: a_score,
            team_b_score: b_score,
        })
    } else {
        // 系列胜场数。
        let (a_won, b_won) = if s.team_a_id == f.team_a {
            (a_won_maps, b_won_maps)
        } else {
            (b_won_maps, a_won_maps)
        };
        Some(crate::scheduled::FixtureScore {
            team_a_score: a_won,
            team_b_score: b_won,
        })
    }
}

/// 历史随机单败淘汰（`TournamentFormat::SingleElim` 默认赛制；全程按 tier 赛制，
/// 无决赛 BO5 特判）：首轮随机配对，奇数队伍末位轮空。
///
/// T3 一致性（2026-08-20）：`first_round` 传入赛前物化的首轮对阵
/// （`ScheduledTournamentRecord.fixtures`）时，**首轮按物化对阵落位**而非随机序——
/// 保证「赛前预览」与「实际赛程」一致，赛后回填 100% 命中。调用方仍保留随机打乱
/// 的 RNG 消费（消费序不变），仅首轮配对次序改用 fixtures；后续轮次照常推进。
fn legacy_single_elim(
    participants: &[TeamEntry],
    tier: TourneyTier,
    match_runner: &mut crate::format::bracket::MatchRunner,
    first_round: Option<&[crate::scheduled::ScheduledFixture]>,
) -> BracketResult {
    let mut series_all: Vec<SeriesResult> = Vec::new();
    let mut matches: Vec<csc_domain::match_result::MatchResult> = Vec::new();
    // 首轮配对已由调用方预打乱（Kotlin `shuffled(random)` 语义）
    let mut field: Vec<TeamEntry> = participants.to_vec();
    let mut round_no = 0usize;
    while field.len() > 1 {
        let mut next: Vec<TeamEntry> = Vec::new();
        // 首轮按物化对阵落位（T3）：fixtures 内 team_a/team_b 直接配对、bye 轮空；
        // 与赛前预览完全一致。后续轮次（及无 fixtures 时）退回随机序配对。
        let pairs: Vec<(TeamId, TeamId)> = if round_no == 0 {
            first_round
                .map(|fx| {
                    fx.iter()
                        .filter(|f| {
                            !f.is_bye() && f.team_a != TeamId::NONE && f.team_b != TeamId::NONE
                        })
                        .map(|f| (f.team_a, f.team_b))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut consumed: std::collections::HashSet<TeamId> = std::collections::HashSet::new();
        let pending: Vec<TeamEntry> = if !pairs.is_empty() {
            // 已按物化配对消费的队伍。
            for (a, b) in &pairs {
                let Some(ta) = field.iter().find(|t| t.id == *a).cloned() else {
                    continue;
                };
                let Some(tb) = field.iter().find(|t| t.id == *b).cloned() else {
                    continue;
                };
                consumed.insert(ta.id);
                consumed.insert(tb.id);
                let series = match_runner(ta.id, tb.id, tier_best_of(tier), SeriesStage::Playoff);
                series_all.push(series.clone());
                matches.push(series.to_match_result());
                next.push(if series.winner_sig == ta.signature {
                    ta
                } else {
                    tb
                });
            }
            // 未出现在 fixtures 中的队伍（旧存档/异常）：保留按剩余顺序兜底配对。
            field
                .iter()
                .filter(|t| !consumed.contains(&t.id))
                .cloned()
                .collect()
        } else {
            field.clone()
        };
        let mut i = 0;
        while i < pending.len() {
            let a = pending[i].clone();
            if i + 1 >= pending.len() {
                // 奇数 → 末位轮空
                next.push(a);
                i += 1;
                continue;
            }
            let b = pending[i + 1].clone();
            let series = match_runner(a.id, b.id, tier_best_of(tier), SeriesStage::Playoff);
            series_all.push(series.clone());
            matches.push(series.to_match_result());
            next.push(if series.winner_sig == a.signature {
                a
            } else {
                b
            });
            i += 2;
        }
        field = next;
        round_no += 1;
    }
    BracketResult {
        champion: field.into_iter().next().unwrap_or(TeamEntry {
            id: TeamId::NONE,
            signature: String::new(),
        }),
        series: series_all,
        matches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduled::{FixtureStatus, ScheduledFixture};
    use csc_domain::tier::Tier;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn empty_participants_returns_none() {
        let mut eng = TournamentEngine::new();
        let mut world = World::new();
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut auto = csc_decision::source::AutoDecisionSource;
        let sched = ScheduledTournament {
            tier: TourneyTier::T1,
            importance: csc_domain::event_importance::EventImportance::Important,
            direct_invitees: vec![],
            replacements: vec![],
            qualifiers: vec![],
            event_date: "2026-06-01".into(),
            format: TournamentFormat::SingleElim,
        };
        let r = eng.run_tournament_detailed(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &sched,
            "T1 Test Event",
            &mut rng,
            None,
        );
        assert!(r.is_none());
    }

    /// A（排程去重 · 引擎护栏）：同名 Future 记录已存在时，
    /// `plan_event_with_ranking` 拒绝重复登记（返回 false），不再产生
    /// player_events 里同名 future/active 交错并存（t5 `#0--7` 证据根因）。
    #[test]
    fn plan_event_dedups_same_name_future_record() {
        let mut eng = TournamentEngine::new();
        let mut world = World::new();
        let mut teams = Vec::new();
        for i in 0..8 {
            let t = world.create_team(format!("Team{i}"), 33 + i, 800 - i * 10);
            for j in 0..5 {
                world
                    .create_npc(
                        Tier::Tier1,
                        Some(t),
                        None,
                        Some(&format!("T{i}P{j}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
            teams.push(t);
        }
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产"),
        );
        let clock = SimClock::of(2026, 2, 20);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut rng = Xoshiro256StarStar::seed(7);
        let ranking: Vec<RankedTeam> = teams
            .iter()
            .enumerate()
            .map(|(i, &t)| (33 + i as i32, 800 - i as i32 * 10, t))
            .collect();
        let event = Tournament::new(
            "CCT Open Cup #1-1",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("Online", "WW", csc_domain::city::Region::Other),
            1,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_team_slots(8)
        .with_direct_invites(8)
        .with_open_qualifier_slots(0)
        .with_date("2026-02-20")
        .with_duration_days(1)
        .with_teams_per_event(Some(8))
        .with_pool_offset(0);

        let first = eng.plan_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        assert!(first, "首次登记应成功");
        let count_after_first = eng.scheduled_records_snapshot().len();

        // 同名 + Future 未结算 → 拒绝重复登记。
        let second = eng.plan_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        assert!(!second, "同名 Future 已存在时必须拒绝重复登记");
        assert_eq!(
            eng.scheduled_records_snapshot().len(),
            count_after_first,
            "拒绝后记录数不得增加"
        );
        assert_eq!(
            eng.scheduled_records_snapshot()
                .iter()
                .filter(|r| r.event_name == "CCT Open Cup #1-1")
                .count(),
            1,
            "同名记录必须唯一（A：跨月去重唯一实例）"
        );

        // 结算后（Completed）同名可再排程（新日期/新赛季）——护栏只挡未完结。
        let mut auto = csc_decision::source::AutoDecisionSource;
        eng.settle_pending_event(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            0,
            &mut rng,
        );
        assert_eq!(
            eng.scheduled_records_snapshot()[0].status,
            ScheduledStatus::Completed,
            "结算后应为 Completed"
        );
        let third = eng.plan_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        assert!(third, "已完结同名赛事允许再次排程（护栏只挡未完结）");
    }

    /// skip_live_today：只把「今天」主角 pending 对阵置 `skipped`，不动 status/result/RNG；
    /// 非比赛日或非主角赛事无副作用（返回 0）。
    #[test]
    fn skip_live_today_marks_todays_pending_fixtures_only() {
        let tid = TeamId(3);
        let other = TeamId(99);
        let mk = |event: &str, date: &str, pid: Option<TeamId>, status: ScheduledStatus| {
            ScheduledTournamentRecord {
                event_name: event.into(),
                date: date.into(),
                tier: TourneyTier::T1,
                team_ids: vec![tid],
                player_team_id: pid,
                status,
                fixtures: vec![ScheduledFixture {
                    fixture_id: 0,
                    round: 1,
                    team_a: tid,
                    team_b: TeamId(8),
                    stage: SeriesStage::Group,
                    best_of: 1,
                    status: FixtureStatus::Pending,
                    result: None,
                    score: None,
                    skipped: false,
                    watched: false,
                }],
                format: TournamentFormat::SingleElim,
            }
        };
        let records = vec![
            // 主角今日比赛 → 应被标记。
            mk(
                "Today Mine",
                "2026-06-08",
                Some(tid),
                ScheduledStatus::Future,
            ),
            // 主角非今日比赛 → 不标记。
            mk(
                "Later Mine",
                "2026-06-20",
                Some(tid),
                ScheduledStatus::Future,
            ),
            // 非主角赛事 → 不标记。
            mk(
                "Not Mine",
                "2026-06-08",
                Some(other),
                ScheduledStatus::Future,
            ),
            // 已完结 → 不标记。
            mk("Done", "2026-06-08", Some(tid), ScheduledStatus::Completed),
        ];

        let mut eng = TournamentEngine::restore_from(
            crate::yearly_rating::YearlyRatingTracker::default(),
            vec![],
            vec![],
            records,
        );

        let skipped = eng.skip_live_today(tid, 2026, 6, 8);
        assert_eq!(skipped, 1, "仅主角今日 pending 对阵被标记");

        let snap = eng.scheduled_records_snapshot();
        assert!(snap[0].fixtures[0].skipped, "今日主角对阵应 skipped");
        assert!(!snap[1].fixtures[0].skipped, "非今日对阵不应 skipped");
        assert!(!snap[2].fixtures[0].skipped, "非主角赛事不应 skipped");
        assert!(!snap[3].fixtures[0].skipped, "已完结赛事不应 skipped");
        // 不改 status/result：比赛结果仍由月末结算回填。
        assert_eq!(snap[0].fixtures[0].status, FixtureStatus::Pending);
        assert!(snap[0].fixtures[0].result.is_none());
        assert_eq!(snap[0].status, ScheduledStatus::Future);

        // 幂等：重复 skip 同一批不再计数（已 skipped 跳过）。
        let again = eng.skip_live_today(tid, 2026, 6, 8);
        assert_eq!(again, 0, "已 skipped 的场次不重复计数");
    }

    #[test]
    fn mark_fixture_watched_is_persistent_and_idempotent() {
        use csc_util::id::{PlayerId, TeamId as TId};
        let mut world = World::new();
        let tid = world.create_team("Heroes", 1, 2000);
        let mut rng = csc_util::rng::Xoshiro256StarStar::seed(1);
        world.create_player(Tier::Tier4, Some("Hero"), &mut rng, None, None);
        let pid = PlayerId(world.players.len() as u32 - 1);
        world.assign_player_to_team(pid, tid).expect("入队");

        let rec = ScheduledTournamentRecord {
            event_name: "Exort Fiesta Series #1-8".into(),
            date: "2026-02-27".into(),
            tier: csc_domain::tourney_tier::TourneyTier::Qualify,
            team_ids: vec![tid, TId(9)],
            player_team_id: Some(tid),
            status: ScheduledStatus::Completed,
            fixtures: vec![ScheduledFixture {
                fixture_id: 7,
                round: 1,
                team_a: tid,
                team_b: TId(9),
                stage: SeriesStage::Group,
                best_of: 1,
                status: FixtureStatus::Completed,
                result: None,
                score: None,
                skipped: false,
                watched: false,
            }],
            format: TournamentFormat::SingleElim,
        };
        let mut eng = TournamentEngine::restore_from(
            crate::yearly_rating::YearlyRatingTracker::default(),
            vec![],
            vec![],
            vec![rec],
        );

        // 未命中：不存在的赛事名 → false。
        assert!(!eng.mark_fixture_watched("No Such Event", 7));
        // 命中 → true，且持久写入 scheduled_records。
        assert!(eng.mark_fixture_watched("Exort Fiesta Series #1-8", 7));
        let snap = eng.scheduled_records_snapshot();
        assert!(snap[0].fixtures[0].watched, "对局应标记为已看完");
        // 幂等：重复标记仍返回 true（早已看完），状态不变。
        assert!(eng.mark_fixture_watched("Exort Fiesta Series #1-8", 7));
        assert!(snap[0].fixtures[0].watched);
    }

    #[test]
    fn legacy_single_elim_four_teams() {
        let mut eng = TournamentEngine::new();
        let mut world = World::new();
        for i in 0..4 {
            let t = world.create_team(format!("Team{i}"), i + 1, 1000 - i * 100);
            for j in 0..5 {
                world
                    .create_npc(
                        Tier::Tier1,
                        Some(t),
                        None,
                        Some(&format!("T{i}P{j}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
        }
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let sched = ScheduledTournament {
            tier: TourneyTier::T1,
            importance: csc_domain::event_importance::EventImportance::Important,
            direct_invitees: (0..4).map(TeamId).collect(),
            replacements: vec![],
            qualifiers: vec![],
            event_date: "2026-06-01".into(),
            format: TournamentFormat::SingleElim,
        };
        let mut rng = Xoshiro256StarStar::seed(1);
        let result = eng.run_tournament_detailed(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &sched,
            "T1 Test Event",
            &mut rng,
            None,
        );
        let r = result.unwrap();
        assert_eq!(r.series.len(), 3, "4 队单败 3 场");
        assert!((0..4).contains(&r.champion.0));
        // 冠军队伍存在
        assert!(world.team(r.champion).is_some());
    }

    #[test]
    fn simulate_event_empty_roster_cancels_without_panic() {
        // P0-D：空世界（无任何队伍可参赛）→ simulate_event_with_ranking 必须
        // 取消赛事而非 panic；不写事件、不发奖、不产生冠军（champion=NONE）。
        let mut eng = TournamentEngine::new();
        let mut world = World::new(); // 无队伍
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut auto = csc_decision::source::AutoDecisionSource;
        let event = Tournament::new(
            "T3 Fallback Cup",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("TBD", "TBD", csc_domain::city::Region::Other),
            1,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_team_slots(8)
        .with_direct_invites(8)
        .with_date("2026-06-20")
        .with_format(TournamentFormat::SingleElim);
        let ranking: Vec<RankedTeam> = Vec::new();
        let events_before = eng.event_count();
        let result = eng.simulate_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        // 不 panic；取消（无冠军、无对局）。
        assert_eq!(result.champion, TeamId::NONE, "取消赛事不应产生冠军");
        assert!(result.series.is_empty(), "取消赛事不应产生对局");
        assert_eq!(eng.event_count(), events_before, "取消赛事不应进入结果列表");
        // ROUND11 可观察契约：取消必须写 journal 事件（玩家可见，非静默消失）。
        assert_eq!(journal.len(), 1, "取消赛事应写入 1 条取消事件");
        let all = journal.all();
        let cancelled = all.first().expect("取消事件应存在");
        assert!(
            matches!(cancelled, WorldEvent::TournamentCancelled { .. }),
            "取消事件类型应为 TournamentCancelled，实际 {cancelled:?}"
        );
        // 取消记录状态为 Cancelled。
        assert_eq!(
            eng.scheduled_records.last().map(|r| r.status),
            Some(ScheduledStatus::Cancelled),
            "scheduled record 应标 Cancelled"
        );
    }

    #[test]
    fn single_elim_runtime_pairs_match_preview_fixtures_and_backfill_all() {
        // T3 一致性：运行时首轮配对必须与赛前物化 fixtures 一致（预览=实际），
        // 赛后回填 100% 命中（无 pending 残留）。同时验证 RNG 消费序未被改变
        // （打乱照常执行，仅配对次序采用 fixtures）。
        let mut eng = TournamentEngine::new();
        let mut world = World::new();
        for i in 0..8 {
            let t = world.create_team(format!("Team{i}"), i + 1, 1000 - i * 100);
            for j in 0..5 {
                world
                    .create_npc(
                        Tier::Tier1,
                        Some(t),
                        None,
                        Some(&format!("T{i}P{j}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
        }
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let event = Tournament::new(
            "T3 Fixture Cup",
            TourneyTier::T1,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("TBD", "TBD", csc_domain::city::Region::Other),
            8,
            csc_domain::tournament::InvitePolicy::VrsGlobal,
        )
        .with_team_slots(8)
        .with_direct_invites(8)
        .with_date("2026-06-20")
        .with_format(TournamentFormat::SingleElim);
        let ranking: Vec<RankedTeam> = (0..8u32)
            .map(|i| (i as i32 + 1, 1000 - i as i32 * 100, TeamId(i)))
            .collect();
        let mut rng = Xoshiro256StarStar::seed(7);
        let result = eng.simulate_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        // 赛事完整跑完（8 队单败 = 7 场）。
        assert_eq!(result.series.len(), 7, "8 队单败应产生 7 场");
        // 物化记录存在且已完成。
        let rec = eng
            .scheduled_records
            .iter()
            .find(|r| r.event_name == "T3 Fixture Cup")
            .expect("排程记录必须存在");
        assert_eq!(rec.status, ScheduledStatus::Completed);
        // 预览 fixtures 非空（8 队 → 4 场 pending）。
        assert_eq!(rec.fixtures.len(), 4, "8 队首轮 4 场");
        // 首轮实际配对 == 预览配对（集合一致；方向可交换）。
        let preview_pairs: std::collections::HashSet<(u32, u32)> = rec
            .fixtures
            .iter()
            .map(|f| {
                let (a, b) = (f.team_a.0, f.team_b.0);
                (a.min(b), a.max(b))
            })
            .collect();
        let runtime_first_round: std::collections::HashSet<(u32, u32)> = result
            .series
            .iter()
            .take(4)
            .map(|s| {
                let a = s.team_a_id.0;
                let b = s.team_b_id.0;
                (a.min(b), a.max(b))
            })
            .collect();
        assert_eq!(
            preview_pairs, runtime_first_round,
            "运行时首轮配对必须与赛前预览一致（T3）"
        );
        // 回填 100% 命中：全部 fixtures 都有结果，无 pending 残留。
        assert!(
            rec.fixtures.iter().all(|f| f.result.is_some()),
            "赛后回填应全部命中（预览=实际），无 pending 残留"
        );
        // R2：比分回填（无比分→有比分）。T1 单败为 BO3 → 系列胜场数（≥1）。
        for f in &rec.fixtures {
            if f.is_bye() {
                continue;
            }
            let score = f.score.expect("非轮空对局应回填比分");
            assert!(
                (score.team_a_score + score.team_b_score) >= 1,
                "系列胜场数应 ≥1（{score:?}）"
            );
        }
    }

    /// 构造 N 队世界（每队 5 NPC）+ 空 VRS + 固定时钟，返回 (engine, world, vrs)。
    fn fixture_world(n: u32) -> (TournamentEngine, World, VrsEngine) {
        use csc_entities::world::World;
        let mut world = World::new();
        for i in 0..n {
            let t = world.create_team(format!("Team{i}"), i as i32 + 1, 1000 - i as i32 * 100);
            for j in 0..5 {
                world
                    .create_npc(
                        Tier::Tier1,
                        Some(t),
                        None,
                        Some(&format!("T{i}P{j}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
        }
        let vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        (TournamentEngine::new(), world, vrs)
    }

    #[test]
    fn double_elim_8_teams_runtime_pairs_match_preview_and_backfill_all() {
        // P1 契约：T3（8 队 = 4 倍数）预览按 snake_seed 蛇形分组，
        // 运行时 `DoubleElimGroupsBracket` 同用 snake_seed —— 首轮配对必须
        // 完全一致，赛后回填 100% 命中（无 pending 残留）。
        // 直接走 `run_tournament_detailed`（绕开排程归一化），聚焦
        // 「预览配对 == 运行时配对」这一契约本身。
        let (mut eng, mut world, mut vrs) = fixture_world(8);
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let sched = ScheduledTournament {
            tier: TourneyTier::Qualify,
            importance: csc_domain::event_importance::EventImportance::Background,
            direct_invitees: (0..8).map(TeamId).collect(),
            replacements: vec![],
            qualifiers: vec![],
            event_date: "2026-06-20".into(),
            format: TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
        };
        let fixtures = crate::scheduled::first_round_fixtures(
            sched.format,
            &sched.all_participants(),
            sched.tier,
        );
        let mut rng = Xoshiro256StarStar::seed(7);
        let result = eng
            .run_tournament_detailed(
                &mut world,
                &mut vrs,
                &clock,
                &mut journal,
                &mut auto,
                &mut log,
                &sched,
                "T3 Double Elim Cup",
                &mut rng,
                Some(&fixtures),
            )
            .expect("8 队双败必开赛");
        assert!(!result.series.is_empty(), "8 队双败应产生对局");
        // 预览配对 = 蛇形分组：组1 [1,4,5,8]→(1,8)(4,5)；组2 [2,3,6,7]→(2,7)(3,6)。
        let preview_pairs: std::collections::HashSet<(u32, u32)> = fixtures
            .iter()
            .map(|f| {
                let (a, b) = (f.team_a.0, f.team_b.0);
                (a.min(b), a.max(b))
            })
            .collect();
        assert_eq!(
            preview_pairs,
            std::collections::HashSet::from([(0, 7), (3, 4), (1, 6), (2, 5)]),
            "预览必须按 snake_seed 蛇形分组配对"
        );
        // 运行时全部系列赛配对（双败组内 series 顺序 = WB半决→WB决赛→LB→LB决赛，
        // 前 4 场并非纯首轮）——预览的每个配对必须都出现在运行时（集合子集）。
        let runtime_pairs: std::collections::HashSet<(u32, u32)> = result
            .series
            .iter()
            .map(|s| {
                let a = s.team_a_id.0;
                let b = s.team_b_id.0;
                (a.min(b), a.max(b))
            })
            .collect();
        assert!(
            preview_pairs.is_subset(&runtime_pairs),
            "8 队双败预览配对必须全部在运行时出现（P1）：预览 {preview_pairs:?} ⊄ 运行时 {runtime_pairs:?}"
        );
        // 赛后回填 100% 命中（按队伍对匹配，与预览配对一致则全部命中）。
        // 直接对预览 fixtures 按队伍对匹配运行时 matches（= backfill 语义）。
        let mut backfilled = 0;
        for f in &fixtures {
            let hit = result.matches.iter().find(|m| {
                let wid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.winner_sig)
                    .map(|t| t.id);
                let lid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.loser_sig)
                    .map(|t| t.id);
                wid.is_some_and(|w| {
                    lid.is_some_and(|l| {
                        (f.team_a == w && f.team_b == l) || (f.team_a == l && f.team_b == w)
                    })
                })
            });
            if hit.is_some() {
                backfilled += 1;
            }
        }
        assert_eq!(
            backfilled,
            fixtures.len(),
            "8 队双败预览 4 场全部应按队伍对命中回填（P1 契约）"
        );
    }

    #[test]
    fn double_elim_9_teams_degrade_runtime_matches_preview_and_backfill_all() {
        // P1 契约：9 队 T3（直接跑赛制引擎绕开排程归一化）→ 运行时
        // `DoubleElimGroupsBracket` 弹性降级单败（奇偶轮空：每轮奇数队时
        // 最强种子轮空直接晋级，其余两两配对）；预览 `first_round_fixtures`
        // 必须同语义（floor(9/2)=4 真赛 + 1 轮空），赛后回填 100% 命中。
        let (mut eng, mut world, mut vrs) = fixture_world(9);
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let sched = ScheduledTournament {
            tier: TourneyTier::Qualify,
            importance: csc_domain::event_importance::EventImportance::Background,
            direct_invitees: (0..9).map(TeamId).collect(),
            replacements: vec![],
            qualifiers: vec![],
            event_date: "2026-06-20".into(),
            format: TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
        };
        let fixtures = crate::scheduled::first_round_fixtures(
            sched.format,
            &sched.all_participants(),
            sched.tier,
        );
        // 预览 = 单败奇偶轮空：4 场真赛 + 1 轮空（9 队 → 首轮 4 真赛 + 种子 1 轮空）。
        assert_eq!(
            fixtures.iter().filter(|f| !f.is_bye()).count(),
            4,
            "9 队单败首轮 4 场真赛（奇偶轮空）"
        );
        assert_eq!(
            fixtures.iter().filter(|f| f.is_bye()).count(),
            1,
            "9 队首轮 1 个轮空（最强种子）"
        );
        let mut rng = Xoshiro256StarStar::seed(7);
        let result = eng
            .run_tournament_detailed(
                &mut world,
                &mut vrs,
                &clock,
                &mut journal,
                &mut auto,
                &mut log,
                &sched,
                "T3 Nine Team Cup",
                &mut rng,
                Some(&fixtures),
            )
            .expect("9 队降级单败必开赛");
        assert_eq!(result.series.len(), 8, "9 队单败应产生 8 场（N-1 守恒）");
        // 预览全部真赛配对必须出现在运行时（配对一致）。
        let preview_pairs: std::collections::HashSet<(u32, u32)> = fixtures
            .iter()
            .filter(|f| !f.is_bye())
            .map(|f| (f.team_a.0.min(f.team_b.0), f.team_a.0.max(f.team_b.0)))
            .collect();
        assert_eq!(preview_pairs.len(), 4, "9 队预览应有 4 个真赛配对");
        let all_pairs: std::collections::HashSet<(u32, u32)> = result
            .series
            .iter()
            .map(|s| {
                let a = s.team_a_id.0;
                let b = s.team_b_id.0;
                (a.min(b), a.max(b))
            })
            .collect();
        assert!(
            preview_pairs.is_subset(&all_pairs),
            "9 队降级单败预览配对必须全部在运行时出现（P1）：预览 {preview_pairs:?} ⊄ 运行时 {all_pairs:?}"
        );
        // 按队伍对回填：预览每场真赛必须命中（奇偶轮空配对一致）。
        for real in fixtures.iter().filter(|f| !f.is_bye()) {
            let hit = result.matches.iter().find(|m| {
                let wid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.winner_sig)
                    .map(|t| t.id);
                let lid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.loser_sig)
                    .map(|t| t.id);
                wid.is_some_and(|w| {
                    lid.is_some_and(|l| {
                        (real.team_a == w && real.team_b == l)
                            || (real.team_a == l && real.team_b == w)
                    })
                })
            });
            assert!(hit.is_some(), "9 队降级单败预览真赛必须回填命中（P1 契约）");
        }
    }

    #[test]
    fn single_elim_5_teams_runtime_first_round_matches_preview_exactly() {
        // t7 验收标准：预览首轮真赛无序队伍对 == SingleElim runtime 首轮
        // 完全相等（不只子集）；BYE 为 runtime 奇数轮 field[0]（最强种子）。
        // 5 队 → 预览 2 真赛 + 1 轮空（种子 1），运行时首轮必须同配对。
        let (mut eng, mut world, mut vrs) = fixture_world(5);
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let sched = ScheduledTournament {
            tier: TourneyTier::Qualify,
            importance: csc_domain::event_importance::EventImportance::Background,
            direct_invitees: (0..5).map(TeamId).collect(),
            replacements: vec![],
            qualifiers: vec![],
            event_date: "2026-06-20".into(),
            format: TournamentFormat::SingleElim,
        };
        let fixtures = crate::scheduled::first_round_fixtures(
            sched.format,
            &sched.all_participants(),
            sched.tier,
        );
        // 预览：2 真赛 + 1 轮空（种子 1 = TeamId(0)）。
        let real: Vec<_> = fixtures.iter().filter(|f| !f.is_bye()).collect();
        let byes: Vec<_> = fixtures.iter().filter(|f| f.is_bye()).collect();
        assert_eq!(real.len(), 2, "5 队预览 2 真赛");
        assert_eq!(byes.len(), 1, "5 队预览 1 轮空");
        assert_eq!(byes[0].team_a, TeamId(0), "轮空 = 最强种子");
        let preview_pairs: std::collections::HashSet<(u32, u32)> = real
            .iter()
            .map(|f| (f.team_a.0.min(f.team_b.0), f.team_a.0.max(f.team_b.0)))
            .collect();
        let mut rng = Xoshiro256StarStar::seed(7);
        let result = eng
            .run_tournament_detailed(
                &mut world,
                &mut vrs,
                &clock,
                &mut journal,
                &mut auto,
                &mut log,
                &sched,
                "T3 Five Team Cup",
                &mut rng,
                Some(&fixtures),
            )
            .expect("5 队单败必开赛");
        assert_eq!(result.series.len(), 4, "5 队单败 4 场（N-1 守恒）");
        // 运行时**首轮**（前 2 场 = 首轮真赛；轮空不产生 series）配对。
        let first_round_pairs: std::collections::HashSet<(u32, u32)> = result
            .series
            .iter()
            .take(real.len())
            .map(|s| {
                let a = s.team_a_id.0;
                let b = s.team_b_id.0;
                (a.min(b), a.max(b))
            })
            .collect();
        assert_eq!(
            preview_pairs, first_round_pairs,
            "5 队预览首轮配对必须与运行时首轮完全相等（t7 验收）：预览 {preview_pairs:?} ≠ 运行时 {first_round_pairs:?}"
        );
        // 回填 100%：预览每场真赛命中运行时 matches。
        for real_f in real {
            let hit = result.matches.iter().find(|m| {
                let wid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.winner_sig)
                    .map(|t| t.id);
                let lid = world
                    .all_teams()
                    .iter()
                    .find(|t| world.signature_of(t.id) == m.loser_sig)
                    .map(|t| t.id);
                wid.is_some_and(|w| {
                    lid.is_some_and(|l| {
                        (real_f.team_a == w && real_f.team_b == l)
                            || (real_f.team_a == l && real_f.team_b == w)
                    })
                })
            });
            assert!(hit.is_some(), "5 队预览真赛必须回填命中（P1 契约）");
        }
    }

    #[test]
    fn t3_resolver_normalizes_to_8_and_runs_double_elim() {
        // ROUND11 端到端：T3 走完整 simulate_event_with_ranking 链路——
        // resolver 固定 8 队（9+ 候选截断，不升 16），8 队走标准双败小组
        // （snake_seed 分组），赛事正常开赛、无轮空过载。
        let (mut eng, mut world, mut vrs) = fixture_world(12); // 12 队池（含 9+ 候选路径）
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let event = Tournament::new(
            "T3 Normalize Cup",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("TBD", "TBD", csc_domain::city::Region::Other),
            1,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_team_slots(8)
        .with_direct_invites(8)
        .with_date("2026-06-20")
        .with_format(TournamentFormat::DoubleElimGroups {
            groups: 2,
            group_best_of: 1,
            playoff_best_of: 3,
            final_best_of: 5,
        });
        // 排名快照：全部 12 队 > 32 名（T3 段），候选切片 8 队 + 4 队兜底。
        let ranking: Vec<RankedTeam> = (33..=44)
            .map(|r| (r, 2500 - r * 10, TeamId((r - 33) as u32)))
            .collect();
        let mut rng = Xoshiro256StarStar::seed(9);
        let result = eng.simulate_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        // resolver 归一化：参赛名单 = 8 队（非 9+ 升 16）。
        let parts = eng
            .scheduled_records
            .last()
            .expect("应有排程记录")
            .team_ids
            .clone();
        assert_eq!(parts.len(), 8, "T3 端到端参赛名单必须为 8 队");
        // 正常开赛：有冠军、有对局、无取消事件。
        assert_ne!(result.champion, TeamId::NONE, "8 队 T3 正常开赛应产生冠军");
        assert!(!result.series.is_empty(), "8 队 T3 正常开赛应产生对局");
        assert!(
            !journal
                .all()
                .iter()
                .any(|e| matches!(e, WorldEvent::TournamentCancelled { .. })),
            "8 队 T3 正常开赛不应写取消事件"
        );
    }

    #[test]
    fn t3_underfilled_candidates_cancel_with_observable_event() {
        // ROUND11 端到端可观察契约：候选不足 8 且兜底补不满 → 取消整场，
        // 但必须写入 TOURNAMENT_CANCELLED 事件（玩家可见）+ scheduled 标 Cancelled。
        let (mut eng, mut world, mut vrs) = fixture_world(3); // 仅 3 队（凑不满 8）
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut auto = csc_decision::source::AutoDecisionSource;
        let event = Tournament::new(
            "T3 Underfilled Cup",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("TBD", "TBD", csc_domain::city::Region::Other),
            1,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_team_slots(8)
        .with_direct_invites(8)
        .with_date("2026-06-20")
        .with_format(TournamentFormat::DoubleElimGroups {
            groups: 2,
            group_best_of: 1,
            playoff_best_of: 3,
            final_best_of: 5,
        });
        let ranking: Vec<RankedTeam> = (33..=35)
            .map(|r| (r, 2500 - r * 10, TeamId((r - 33) as u32)))
            .collect();
        let mut rng = Xoshiro256StarStar::seed(3);
        let result = eng.simulate_event_with_ranking(
            &mut world,
            &mut vrs,
            &clock,
            &mut journal,
            &mut auto,
            &mut log,
            &event,
            &ranking,
            &mut rng,
        );
        assert_eq!(result.champion, TeamId::NONE, "取消赛事无冠军");
        assert!(result.series.is_empty(), "取消赛事无对局");
        // 可观察：1 条取消事件 + 排程记录 Cancelled。
        assert_eq!(journal.len(), 1, "取消必须写 1 条取消事件");
        let all = journal.all();
        assert!(
            matches!(all.first(), Some(WorldEvent::TournamentCancelled { .. })),
            "取消事件类型应为 TournamentCancelled"
        );
        assert_eq!(
            eng.scheduled_records.last().map(|r| r.status),
            Some(ScheduledStatus::Cancelled),
            "排程记录应标 Cancelled"
        );
    }
}
