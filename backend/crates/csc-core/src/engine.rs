//! 总引擎（Kotlin `Engine.kt` 转写）——**接线层**（core 只做三件事：装配 / 存档 / 门面）。
//!
//! 职责边界：
//! - **装配**：`create` 创建全部系统实例并注入共享依赖（journal/decisionLog/...）；
//! - **推进**：`advance_month` 委托 [`crate::season`] 阶段管线（8 步）；
//! - **存档**：`snapshot`/`restore`（GameState 完整聚合根，含实体/VRS/赛事/年度累计）；
//! - **门面**：对外的便捷接口（转会窗/合同摘要/年度颁奖/玩家查询）。
//!
//! 可复现性契约：**种子 + 决策日志 = 完全一致的世界**。

use csc_career::archive::CareerArchive;
use csc_decision::log::DecisionLog;
use csc_decision::offer::TransferOffer;
use csc_decision::point::PlayerDecision;
use csc_decision::source::{AutoDecisionSource, DecisionSource};
use csc_entities::world::World;
use csc_events::journal::WorldJournal;
use csc_time::clock::SimClock;
use csc_tournaments::calendar::SeasonCalendar;
use csc_tournaments::engine::TournamentEngine;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::context::WorldContext;
use crate::loop_::SeasonLoop;
use crate::query::WorldQuery;
use crate::season::SeasonDirector;
use crate::state::{GameState, WORLD_SIM_VERSION};

/// 可选校准资产（真实数据先验的 JSON 内容——零 IO，由调用方从 `assets/` 读入）。
///
/// 四个字段都是「真实选手/生态校准」：`baseline`（角色+年龄）、`ratings`（真实
/// Rating 先验）、`rating_profile`（新选手实力分布）、`text`（叙事文案覆盖包）。
/// 打包为一，避免 [`Engine::load_from_standings`] 的参数随资产种类膨胀。
#[derive(Debug, Clone, Copy, Default)]
pub struct CalibrationAssets<'a> {
    /// roles_baseline.json 内容（可选——缺失时按阵容槽位分配位置/随机年龄）
    pub baseline: Option<&'a str>,
    /// player_ratings.json 内容（可选——真实选手 Rating 先验，校准档位）
    pub ratings: Option<&'a str>,
    /// rating_profile.json 内容（可选——新选手实力分布，校准新秀起始档位）
    pub rating_profile: Option<&'a str>,
    /// 文案覆盖包 JSON（可选——覆盖 `assets/text/zh-CN.json` 的默认文案；
    /// None = 编译期嵌入默认包，行为与硬编码时代一致）
    pub text: Option<&'a str>,
}

/// 总引擎——接线层（装配/推进/存档/门面）。
///
/// **封装边界**：全部字段私有。外部仅能通过只读访问器观察状态，任何可变推进
/// 都必须经门面方法（advance_month/run_season 等）——防止绕过阶段管线。
pub struct Engine {
    /// 实体 arena（选手/队伍/自由市场）
    world: World,
    /// VRS 子系统（排名结算）
    vrs: VrsEngine,
    /// 赛事子系统（排程/赛制/结算编排）
    tournaments: TournamentEngine,
    /// 共享日期线程
    clock: SimClock,
    /// 世界事件日志（叙事/复盘/增量同步）
    journal: WorldJournal,
    /// 生涯档案（逐赛季记录）
    archive: CareerArchive,
    /// 决策日志（可复现性第二支柱）
    decision_log: DecisionLog,
    /// 赛事日历（月份 → 赛事模板 + 时间片）
    calendar: SeasonCalendar,
    /// 锁与月度赛事编排
    loop_: SeasonLoop,
    /// 推进编排（月计数/合同锚点/确定性 RNG）
    director: SeasonDirector,
    /// 新选手实力分布模型（装配配置，不入存档；None = 新秀固定 Tier4）
    rating_profile: Option<csc_entities::baseline::RatingProfile>,
    /// 叙事文案包（装配配置，不入存档；默认 = 编译期嵌入的 zh-CN 包）
    text: csc_text::TextBundle,
    /// 剧情进度（弧进度/承诺/flags/冷却/已见场景/终态；入存档 v10）
    narrative: crate::narrative::NarrativeState,
    /// 可序列化执行状态（持久暂停；入存档 v10）
    execution: crate::state::ExecutionState,
}

impl Engine {
    /// 默认随机种子（可复现性的起点；世界 = seed + 决策日志）。
    pub const DEFAULT_SEED: u64 = 42;

    /// 装配总引擎：空世界 + 空 VRS 库（测试/渐进装载用）。
    pub fn empty(seed: u64, clock: SimClock) -> Self {
        let rng = Xoshiro256StarStar::seed(seed);
        let year = clock.year();
        Self {
            world: World::new(),
            vrs: VrsEngine::from_database(
                csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            ),
            tournaments: TournamentEngine::new(),
            clock,
            journal: WorldJournal::default(),
            archive: CareerArchive::default(),
            decision_log: DecisionLog::default(),
            calendar: SeasonCalendar,
            loop_: SeasonLoop::default(),
            director: SeasonDirector::new(year, rng),
            rating_profile: None,
            text: csc_text::TextBundle::default(),
            narrative: crate::narrative::NarrativeState::default(),
            execution: crate::state::ExecutionState::Idle,
        }
    }

    /// 主角玩家（当前单主角 = 全部玩家实体；未来可扩展）。
    pub fn player(&self) -> Option<PlayerId> {
        self.world.all_players_only().first().map(|p| p.id)
    }

    // —— 只读访问器（封装边界）——

    /// 实体 arena 只读视图。
    pub fn world(&self) -> &World {
        &self.world
    }

    /// VRS 排名引擎只读视图。
    pub fn vrs(&self) -> &VrsEngine {
        &self.vrs
    }

    /// 赛事子系统只读视图。
    pub fn tournaments(&self) -> &TournamentEngine {
        &self.tournaments
    }

    /// 赛事子系统可变视图（服务端外部操作「跳过比赛日 LIVE 提示」写标记用；
    /// 引擎内部推进统一走阶段管线，不经此入口）。
    pub fn tournaments_mut(&mut self) -> &mut TournamentEngine {
        &mut self.tournaments
    }

    /// 把主角队伍**今天**的 pending 对阵标记为「已跳过」（`skip` 持久语义），
    /// 返回被标记的 fixture 数。仅置 `skipped` 标志，不改比赛结算——
    /// 结果仍由月末 `settle_month` 确定性模拟回填（保证跳过后比赛照常出比分）。
    pub fn skip_live_today(&mut self) -> usize {
        let protagonist_team = self
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .and_then(|p| p.team);
        let Some(protagonist_team) = protagonist_team else {
            return 0;
        };
        let (year, month, day) = self.clock.now();
        self.tournaments
            .skip_live_today(protagonist_team, year, month, day)
    }

    /// R2：把指定赛事对局标记为「已看完」（`watched` 持久语义，幂等）。
    /// 返回是否命中（false = 赛事或对局不存在）。
    pub fn mark_fixture_watched(&mut self, event_name: &str, fixture_id: u32) -> bool {
        self.tournaments
            .mark_fixture_watched(event_name, fixture_id)
    }

    /// 模拟时钟只读视图。
    pub fn clock(&self) -> &SimClock {
        &self.clock
    }

    /// 世界事件日志只读视图。
    pub fn journal(&self) -> &WorldJournal {
        &self.journal
    }

    /// 世界事件日志（可变——服务端外部操作（资金运用等）写叙事用；
    /// 引擎内部推进统一走阶段管线，不经此入口）。
    pub fn journal_mut(&mut self) -> &mut WorldJournal {
        &mut self.journal
    }

    /// 引擎确定性 RNG（可变——服务端外部操作（买断补位等）需要确定性随机源；
    /// 注意：任何消费都会推进 RNG 序列，影响后续世界——外部操作请优先用
    /// 纯函数哈希派生，仅在必须时（如补位新秀生成）使用）。
    pub fn rng_mut(&mut self) -> &mut Xoshiro256StarStar {
        &mut self.director.rng
    }

    /// 生涯档案只读视图。
    pub fn archive(&self) -> &CareerArchive {
        &self.archive
    }

    /// 决策日志只读视图。
    pub fn decision_log(&self) -> &DecisionLog {
        &self.decision_log
    }

    /// 叙事状态只读视图（内容资产 + 进度）。
    pub fn narrative(&self) -> &crate::narrative::NarrativeState {
        &self.narrative
    }

    /// 叙事状态可变视图（服务端应用剧情选择时用）。
    pub fn narrative_mut(&mut self) -> &mut crate::narrative::NarrativeState {
        &mut self.narrative
    }

    /// 可序列化执行状态只读视图（持久暂停）。
    pub fn execution(&self) -> &crate::state::ExecutionState {
        &self.execution
    }

    /// 可序列化执行状态可变视图。
    pub fn execution_mut(&mut self) -> &mut crate::state::ExecutionState {
        &mut self.execution
    }

    /// 装配叙事内容资产（服务端启动/建局时注入 `assets/narrative/zh-CN/*.json`）。
    pub fn set_narrative_content(
        &mut self,
        content: crate::narrative::NarrativeContent,
    ) -> Result<(), csc_util::SimError> {
        content.validate()?;
        Self::validate_narrative_progress(&self.narrative.progress, &content)?;
        self.narrative.progress.content_version = content.content_version;
        self.narrative.content = content;
        Ok(())
    }

    fn validate_narrative_progress(
        progress: &csc_entities::narrative::NarrativeProgress,
        content: &crate::narrative::NarrativeContent,
    ) -> Result<(), csc_util::SimError> {
        let started = progress.active_scene.is_some()
            || !progress.seen_scenes.is_empty()
            || !progress.promises.is_empty()
            || !progress.arc_stage.is_empty();
        if started
            && (content.scenes.is_empty() || progress.content_version != content.content_version)
        {
            return Err(csc_util::SimError::save(
                "剧情进度要求匹配的内容版本，拒绝替换正文",
            ));
        }
        if let Some(active) = &progress.active_scene
            && content.scene(&active.scene_id).is_none()
        {
            return Err(csc_util::SimError::save("活动场景在当前内容中不存在"));
        }
        Ok(())
    }

    /// 推进一个单位（月步/日步；故事步进驱动的唯一入口）。
    pub fn advance_unit(
        &mut self,
        decision: &mut dyn DecisionSource,
        step_kind: crate::state::StepKind,
    ) -> Result<(), csc_util::SimError> {
        match step_kind {
            crate::state::StepKind::Month => self.advance_month(decision),
            crate::state::StepKind::Day => self.advance_day(decision),
        }
    }

    /// 组装叙事导演所需的只读世界事实（主角系列赛/失利/转会接触/日期）。
    pub fn narrative_facts(&self) -> crate::narrative::NarrativeFacts {
        let protagonist_id = self
            .world
            .players
            .iter()
            .find(|p| p.is_player())
            .map(|p| p.id);
        // 旧 MatchPlayed 只保存队伍 ID，逐图 PlayerLine 只保存名字。
        // 无法可靠证明主角当时参赛；当前队伍反推会继承转入队的旧战绩。
        // 待 settlement 持久记录 PlayerId 后启用赛后触发；0 是保守的已证明下界，
        // 不是“历史没有比赛”的产品事实。故事世界推进能力暂不开放。
        let series_played = 0;
        let series_lost = 0;
        let has_transfer_contact = protagonist_id
            .and_then(|pid| self.world.player(pid))
            .and_then(|p| p.career.as_ref())
            .is_some_and(|c| !c.transfer_contacts.is_empty());
        let (y, m, _) = self.clock.now();
        crate::narrative::NarrativeFacts {
            series_played,
            series_lost,
            month_index: y * 12 + m as i32,
            date: self.clock.date_label(),
            has_transfer_contact,
        }
    }

    /// 开始一个剧情场景（写入 `active_scene`；演员绑定真实人物）。
    ///
    /// 仅 teammate 绑定真实队友。当前实体没有可靠教练/队长身份，其他角色保持概括，
    /// 不把任意队友冒充教练或队长。
    pub fn begin_scene(
        &mut self,
        scene: &crate::narrative::SceneDef,
        facts: &crate::narrative::NarrativeFacts,
    ) {
        let mut run = crate::narrative::instantiate(scene, facts);
        let protagonist_id = self
            .world
            .players
            .iter()
            .find(|p| p.is_player())
            .map(|p| p.id);
        let team_id = protagonist_id
            .and_then(|pid| self.world.player(pid))
            .and_then(|p| p.team);
        // 角色 → 真实队友绑定（存在才绑；找不到即保留概括角色）。
        let role = scene.actor_role.as_str();
        let wants_teammate = role == "teammate";
        if wants_teammate && let (Some(pid), Some(tid)) = (protagonist_id, team_id) {
            let mate = self
                .world
                .roster(tid)
                .into_iter()
                .find(|p| p.id != pid)
                .map(|p| (p.id, p.name.clone()));
            if let Some((mid, mname)) = mate {
                run.actor_id = Some(mid);
                run.actor_name = Some(mname);
            }
        }
        self.narrative.progress.active_scene = Some(run);
    }

    /// 应用一个剧情选择（校验选项确属候选；原子应用并记录）。
    pub fn apply_story_choice(
        &mut self,
        choice_id: &str,
    ) -> Result<crate::narrative::AppliedOutcome, csc_util::SimError> {
        let scene = self
            .narrative
            .active_scene_def()
            .ok_or_else(|| csc_util::SimError::protocol("当前没有进行中的剧情场景"))?
            .clone();
        let choice = scene
            .choices
            .iter()
            .find(|c| c.id == choice_id)
            .ok_or_else(|| {
                csc_util::SimError::protocol(format!(
                    "选项「{choice_id}」不在场景「{}」的候选列表中",
                    scene.id
                ))
            })?
            .clone();
        let player_id = self
            .world
            .players
            .iter()
            .find(|p| p.is_player())
            .map(|p| p.id)
            .ok_or_else(|| csc_util::SimError::protocol("尚无主角"))?;
        let actor_id = self
            .narrative
            .progress
            .active_scene
            .as_ref()
            .and_then(|a| a.actor_id);
        let (y, m, _) = self.clock.now();
        let month_index = y * 12 + m as i32;
        let date = self.clock.date_label();
        let outcome = crate::narrative::apply_choice(
            &mut self.world,
            &mut self.journal,
            &mut self.narrative.progress,
            crate::narrative::ChoiceContext {
                scene: &scene,
                choice: &choice,
                player_id,
                actor_id,
                month_index,
                date: &date,
            },
        )?;
        Ok(outcome)
    }

    /// 当前赛季月份计数（0 起）。
    pub fn month(&self) -> i32 {
        self.director.month
    }

    /// RNG 状态快照（4×u64，诊断用）。
    pub fn rng_snapshot(&self) -> [u64; 4] {
        self.director.rng_snapshot()
    }

    /// 客户端轻量视图（`GET /games/{id}/view` 数据源）：只投影主角/roster/
    /// 主角赛事与主角年度标量——**不 clone 全量赛事结果与事件流**。
    /// 网页端高频轮询用它替代 [`Self::snapshot`]（20 年完整快照 ≈68MB）。
    pub fn client_state(&self) -> crate::client::ClientState {
        let (sim_year, sim_month, sim_day) = self.clock.now();
        crate::client::client_state_from(
            self.director.month,
            sim_year,
            sim_month,
            sim_day,
            self.loop_.lock_records(),
            &self.world,
            &self.tournaments,
            &self.archive,
            self.journal.len(),
            &self.decision_log,
            &self.tournaments.scheduled_records_snapshot(),
        )
    }

    /// 只读查询视图（表现层/服务端协议边界）。
    pub fn query(&self) -> WorldQuery<'_> {
        WorldQuery {
            world: &self.world,
            vrs: &self.vrs,
            tournaments: &self.tournaments,
            journal: &self.journal,
            archive: &self.archive,
            decision_log: &self.decision_log,
            clock: &self.clock,
            text: &self.text,
        }
    }

    /// 实体 arena 可变视图——仅供装配/测试构造。
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// VRS 引擎可变视图——仅供装配/测试。
    pub fn vrs_mut(&mut self) -> &mut VrsEngine {
        &mut self.vrs
    }

    /// 推进一个月（阶段管线见 [`crate::season`]）。
    /// 决策源返回非法选项（协议错误）→ `Err(SimError::Protocol)`。
    ///
    /// LIVE 闸门两阶段拆解（t12）：月推进 = `advance_month_plan`（登记新月为 Future）
    /// 之后 `settle_month`（立即结算快进）再 `advance_month_finish`（月终收束/世界批次）。
    /// Auto/月推进路径不受闸门影响。
    ///
    /// R2.1 修复（开局月滞留）：月推进在 plan **之前**先结算当前月（月入口
    /// `settle_month`）——`plan_opening_month` 物化的开局月（如 2026-01，month_idx=-1）
    /// 记录以「01-」为日期前缀，而 `settle_pending_events` 只按**时钟当前月前缀**
    /// 匹配。旧实现 `advance_month_plan` 先 `next_month_start` 进入 2 月，1 月记录
    /// 永远匹配不到 → 永久滞留 `Future`（t5 证据：`#0--7` date 01-20 status=future）。
    /// 结算完成后记录推进为 `Completed`，随后清锁进新月重排——同种子消费序固定，
    /// 确定性不变（日级月出口分支已有同样语义，此处对齐）。
    pub fn advance_month(
        &mut self,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        self.settle_month(decision)?; // R2.1：先结算当前月（含开局月物化的赛事），消除 future 滞留
        self.advance_month_plan(decision)?;
        self.settle_month(decision)?; // 立即结算新月（快进路径，与 season::advance_month 同序）
        self.advance_month_finish(decision, true)?;
        #[cfg(debug_assertions)]
        crate::nan::assert_no_nan(&self.snapshot());
        Ok(())
    }

    /// R2 ① 修复：**开局即物化当前月（季前月 2026-01）的赛事排程 + fixtures + 锁**。
    ///
    /// 根因（t1）：引擎设计把 2026-01 当作「季前展示月」，首个模拟月是 2026-02
    /// （`advance_month` 先 `next_month_start`）。因此新游戏开局 `scheduled_records`/
    /// `locks`/`live-today` 对 1 月恒空——主页 `season_calendar_plan(2026)` 却承诺了
    /// 01-20 起的一整月赛事，比赛日无声滑过（「卡赛事」）；1 月赛事直到 2 月
    /// `advance_month` 才以同名（m=0）重新登记，造成「错位重排进 2 月」。
    ///
    /// 本方法在**新游戏装配**（主角入队后）调用，对当前时钟所在月做一次
    /// `plan_month`（登记 `Future` + pending fixtures + 队伍锁，**不结算**），
    /// 然后**把时钟恢复到开局日**——1 月赛事因此在开局即可查（`calendar`/
    /// `event_aggregate`）、比赛日 `live/today` 可观测 pending 对阵。
    ///
    /// `month_idx` 为赛事命名序号（开局月 2026-01 = -1，与 `season_calendar_plan`
    /// 的月序平移一致）；`director.month` **不**递增（那是月推进的职责）。
    /// 确定性：plan 消费排程 RNG（邀请/参赛名单），消费序固定、同种子可复现；
    /// 不影响后续 `advance_month` 的首次 plan（2 月用 director.month=0）。
    pub fn plan_opening_month(&mut self, month_idx: i32) {
        let (year, month, day) = self.clock.now();
        let mut ctx = WorldContext {
            world: &mut self.world,
            vrs: &mut self.vrs,
            tournaments: &mut self.tournaments,
            clock: &mut self.clock,
            decision_log: &mut self.decision_log,
            journal: &mut self.journal,
            archive: &mut self.archive,
            rating_profile: self.rating_profile.as_ref(),
            text: &self.text,
        };
        // 直接登记当前时钟月的赛事（plan_month 内部会 advance_to 各赛事 start_day，
        // 以生成正确 event.date 与锁 epoch），不 next_month_start、不结算。
        self.loop_
            .plan_month(&mut ctx, &self.calendar, month_idx, &mut self.director.rng);
        // 恢复到开局日，让后续日级推进逐日经过比赛日（而非停在月末赛事日）。
        self.clock.restore_to(year, month, day);
    }

    /// 结算当前时钟月的已登记（`Future`）赛事（模拟 + 回填 + 荣誉）。日级月出口、
    /// 月推进结算均经此。
    ///
    /// **P2-9**：本方法是中间步骤（advance_month 调 2 次、advance_day 月末出口调 1 次），
    /// 不做全量 NaN 快照扫描——防护上收到调用方出口（advance_month / advance_day 月末）
    /// 各采样一次，避免单月最多 3 次全量深拷贝（debug 下 20 年档内存峰值主要来源之一）。
    pub fn settle_month(
        &mut self,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        let mut ctx = WorldContext {
            world: &mut self.world,
            vrs: &mut self.vrs,
            tournaments: &mut self.tournaments,
            clock: &mut self.clock,
            decision_log: &mut self.decision_log,
            journal: &mut self.journal,
            archive: &mut self.archive,
            rating_profile: self.rating_profile.as_ref(),
            text: &self.text,
        };
        self.loop_
            .settle_month(&mut ctx, decision, &mut self.director.rng);
        Ok(())
    }

    /// 进入新月 + 登记新月赛事为 `Future`（LIVE 闸门阶段 1），**不结算**。
    /// 月推进在 plan 后立即 settle（快进）；日推进在月入口只 plan、月出口才 settle。
    pub fn advance_month_plan(
        &mut self,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        let mut ctx = WorldContext {
            world: &mut self.world,
            vrs: &mut self.vrs,
            tournaments: &mut self.tournaments,
            clock: &mut self.clock,
            decision_log: &mut self.decision_log,
            journal: &mut self.journal,
            archive: &mut self.archive,
            rating_profile: self.rating_profile.as_ref(),
            text: &self.text,
        };
        crate::season::advance_month_plan(
            &mut self.director,
            &mut self.loop_,
            &mut ctx,
            &self.calendar,
            decision,
        )
    }

    /// 月推进收尾（可选月终收束 + VRS + 世界批次）。日推进月出口、月推进均经此。    ///
    /// @param restore_to_end 是否把时钟收束到本月最后一天（月推进= true；
    ///        日级跨月= false，保留新月 1 号供后续逐日推进，见 t16）。
    pub fn advance_month_finish(
        &mut self,
        decision: &mut dyn DecisionSource,
        restore_to_end: bool,
    ) -> Result<(), csc_util::SimError> {
        let mut ctx = WorldContext {
            world: &mut self.world,
            vrs: &mut self.vrs,
            tournaments: &mut self.tournaments,
            clock: &mut self.clock,
            decision_log: &mut self.decision_log,
            journal: &mut self.journal,
            archive: &mut self.archive,
            rating_profile: self.rating_profile.as_ref(),
            text: &self.text,
        };
        crate::season::advance_month_finish(
            &mut self.director,
            &mut self.loop_,
            &mut ctx,
            decision,
            restore_to_end,
        )
    }

    /// 连续推进 `months` 个月。
    pub fn run_season(
        &mut self,
        months: i32,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        let mut ctx = WorldContext {
            world: &mut self.world,
            vrs: &mut self.vrs,
            tournaments: &mut self.tournaments,
            clock: &mut self.clock,
            decision_log: &mut self.decision_log,
            journal: &mut self.journal,
            archive: &mut self.archive,
            rating_profile: self.rating_profile.as_ref(),
            text: &self.text,
        };
        crate::season::run_season(
            &mut self.director,
            &mut self.loop_,
            &mut ctx,
            &self.calendar,
            decision,
            months,
        )
    }

    // —— 存档 / 读档（GameState 完整聚合根）——

    /// 采集世界状态快照：推进状态 + 时钟 + RNG + 决策日志 + 事件流 + 档案 +
    /// **实体 arena + VRS 数据库 + 赛事结果 + 年度累计器**（完整存档）。
    pub fn snapshot(&self) -> GameState {
        let (sim_year, sim_month, sim_day) = self.clock.now();
        // 档案映射一次性克隆（`CareerArchive::snapshot_map`，比逐选手 seasons_of
        // 重组更短；20 年存档口径下档案是大头之一，保持单拷贝路径）
        let archive = self.archive.snapshot_map();
        GameState {
            version: GameState::CURRENT_VERSION,
            month: self.director.month,
            last_contract_year: self.director.last_contract_year,
            locks: self.loop_.lock_records(),
            sim_year,
            sim_month,
            sim_day,
            rng_state: self.director.rng_snapshot(),
            decisions: self.decision_log.entries(),
            journal: self.journal.all(),
            archive,
            world: self.world.clone(),
            vrs: self.vrs.database().clone(),
            events: self.tournaments.results(),
            yearly_rating: self.tournaments.yearly_rating_snapshot(),
            top20_history: self.tournaments.top20_history(),
            scheduled_records: self.tournaments.scheduled_records_snapshot(),
            sim_version: WORLD_SIM_VERSION,
            narrative: self.narrative.progress.clone(),
            execution: self.execution.clone(),
        }
    }

    /// 从快照恢复，并同时恢复新秀实力分布配置（多局 Evict/Resume 路径用）。
    /// `restore` 不覆盖 `rating_profile`；跨进程恢复后若丢配置，未来新秀会
    /// 退化为固定 Tier4，破坏生态校准。
    pub fn restore_with_profile(
        &mut self,
        state: GameState,
        rating_profile: Option<csc_entities::baseline::RatingProfile>,
    ) {
        self.restore(state);
        self.rating_profile = rating_profile;
    }

    /// 从快照恢复（完整读档：全部子系统状态一次性还原）。
    /// `state.sim_version` 被忽略（恢复不校验；指纹门禁比对用）。
    ///
    /// **P1-4 读档原子性**：不在 self 上就地覆盖，而是在临时引擎上完整装配
    /// （构造-交换）——任何一步失败（如锁反解遇到快照与实体不一致）都只丢弃临时
    /// 引擎，self 保持原状。可预期错误经 [`Self::restore_result`] 返回，不 panic。
    pub fn restore(&mut self, state: GameState) {
        self.restore_result(state)
            .expect("restore 失败：读档必须先在临时引擎上完整恢复（restore_result）");
    }

    /// 读档的 Result 版本（构造-交换，全有或全无）：失败时 self 不被改动。
    /// 引擎内部字段全部私有——用 [`Self::empty`] 建临时引擎后逐字段装配，
    /// 装配完成且锁反解通过才整体替换。
    pub fn restore_result(&mut self, state: GameState) -> Result<(), csc_util::SimError> {
        state.validate_execution()?;
        Self::validate_narrative_progress(&state.narrative, &self.narrative.content)?;
        // 先取出叙事/执行状态（内容资产是装配配置，不入存档——恢复时保留 self 已加载的
        // 内容，只替换进度；见 `docs/09` 与 23 §4.3）。
        let narrative_progress = state.narrative.clone();
        let execution = state.execution.clone();
        let mut next = Engine::empty(
            0,
            SimClock::of(state.sim_year, state.sim_month, state.sim_day),
        );
        next.director.restore(&state); // 月份/合同锚点/RNG
        next.clock
            .restore_to(state.sim_year, state.sim_month, state.sim_day);
        next.decision_log.restore(state.decisions);
        next.journal.restore(state.journal);
        next.archive.restore(state.archive);
        // 顺序契约：**先还原实体 arena，再反解锁签名**（锁反解依赖新 world）
        next.world = state.world;
        next.vrs = VrsEngine::from_database(state.vrs);
        next.tournaments = TournamentEngine::restore_from(
            state.yearly_rating,
            state.top20_history,
            state.events,
            state.scheduled_records,
        );
        next.loop_
            .try_restore_locks(&state.locks, &mut next.world)?; // 锁反解失败 → Err，self 未动
        crate::season::sync_vrs_cache(&mut next.world, &next.vrs); // VRS 缓存与数据库对齐
        // 保留已加载的叙事内容资产，只替换进度与执行状态。
        next.narrative.content = self.narrative.content.clone();
        next.narrative.progress = narrative_progress;
        next.execution = execution;
        // 装配配置（不入存档）：文案包与新秀分布模型在恢复时保留 self 的实例——
        // 否则任何 restore（/load、故事步进回滚、LRU 恢复）都会把 text 重置为
        // 编译期内嵌包、rating_profile 丢失（23 §4.3：装配配置与存档状态分开）。
        next.text = self.text.clone();
        next.rating_profile = self.rating_profile.clone();
        *self = next; // 全部成功才整体替换（构造-交换，读档原子性）
        Ok(())
    }

    /// 构造保留全部装配配置的候选引擎，供外层持久化成功后整体交换。
    /// 只复制状态与配置，不推进时间、不消费 RNG、不重放任何模拟阶段。
    pub fn fork_snapshot(&self) -> Result<Self, csc_util::SimError> {
        let mut next = Self::empty(0, self.clock);
        next.text = self.text.clone();
        next.rating_profile = self.rating_profile.clone();
        next.narrative.content = self.narrative.content.clone();
        next.restore_result(self.snapshot())?;
        Ok(next)
    }
    // —— 生涯 / 转会门面 ——

    /// 玩家当前可签约的转会候选（主动转会界面数据源）。
    /// 合同期内为空；买断 / 合同到期成为自由身后，这里列出所有付得起薪资的球队。
    pub fn transfer_offers(
        &self,
        player_id: PlayerId,
    ) -> Result<Vec<TransferOffer>, csc_util::SimError> {
        let date = self.clock.date_label();
        let offers =
            csc_systems::transfer::TransferEngine::generate_offers(&self.world, &self.vrs, &date);
        let mine: Vec<TransferOffer> = offers
            .into_iter()
            .filter(|o| o.player_id == player_id)
            .collect();
        let contract_years = self
            .world
            .player(player_id)
            .and_then(|p| p.career.as_ref())
            .map(|c| c.contract_years)
            .unwrap_or(i32::MAX);
        if contract_years > 0 {
            return Err(csc_util::SimError::protocol(
                "合同期内没有自由转会报价——先买断合同，或等合同到期".to_string(),
            ));
        }
        Ok(mine)
    }

    /// 主动签约目标队（FIFA 生涯模式转会）：玩家在赛季间打开转会界面，
    /// 从 [`Self::transfer_offers`] 候选里选一队立即完成转会，不再等
    /// 「转会窗决策事件」。
    pub fn sign_transfer(
        &mut self,
        player_id: PlayerId,
        team_signature: &str,
    ) -> Result<csc_systems::transfer::TransferEvent, csc_util::SimError> {
        let date = self.clock.date_label();
        let offers =
            csc_systems::transfer::TransferEngine::generate_offers(&self.world, &self.vrs, &date);
        let offer = offers
            .iter()
            .find(|o| o.player_id == player_id)
            .ok_or_else(|| {
                csc_util::SimError::protocol(
                    "合同期内无法自由转会（先买断或等合同到期）".to_string(),
                )
            })?;
        let candidate = offer
            .candidates
            .iter()
            .find(|c| c.team_signature == team_signature)
            .ok_or_else(|| {
                csc_util::SimError::protocol(format!(
                    "目标队不在候选清单：{team_signature}（候选 {} 支）",
                    offer.candidates.len()
                ))
            })?;
        let decisions = [PlayerDecision::new(
            &offer.point_id,
            &candidate.team_signature,
        )];
        let events = csc_systems::transfer::TransferEngine::execute_choices(
            &mut self.world,
            &mut self.vrs,
            &self.clock,
            std::slice::from_ref(offer),
            &decisions,
            &mut self.director.rng,
        )?;
        let event = events.into_iter().next().ok_or_else(|| {
            csc_util::SimError::protocol("转会未生效（目标队可能刚被占用）".to_string())
        })?;
        self.journal
            .record(csc_events::event::WorldEvent::TransferDone {
                date: self.clock.date_label(),
                seq: -1,
                player_name: event.player_name.clone(),
                player_id: event.player_id,
                from_team: event.from_team.clone(),
                from_team_id: event.from_team_id,
                to_team: event.to_team.clone(),
                to_team_id: event.to_team_id,
                fee: event.fee,
            });
        crate::season::sync_vrs_cache(&mut self.world, &self.vrs);
        Ok(event)
    }

    /// 主动公开表态（玩家自己选择发言，不再等媒体事件）。
    /// tone ∈ CONFIDENT / HUMBLE / PROVOKE；确定性结算职业属性与声誉。
    pub fn make_statement(
        &mut self,
        player_id: PlayerId,
        tone: &str,
    ) -> Result<String, csc_util::SimError> {
        let tone = tone.trim().to_uppercase();
        let Some(pc) = self.world.player_mut(player_id) else {
            return Err(csc_util::SimError::protocol(format!(
                "选手不存在：{player_id:?}"
            )));
        };
        let (confidence, morale, spirit, reputation, detail) = match tone.as_str() {
            "CONFIDENT" => (3, 1, 0, 1, "你在采访中自信表态：冠军不是目标，而是计划。"),
            "HUMBLE" => (0, 1, 3, 1, "你谦逊回应外界的赞誉：先把下一场打好。"),
            "PROVOKE" => (2, 1, -2, 3, "你向宿敌放话：赛场上见真章。"),
            _ => {
                return Err(csc_util::SimError::protocol(format!(
                    "非法表态类型：{tone}（CONFIDENT/HUMBLE/PROVOKE）"
                )));
            }
        };
        pc.pro.confidence = (pc.pro.confidence + confidence).clamp(0, 100);
        pc.pro.morale = (pc.pro.morale + morale).clamp(0, 100);
        pc.pro.team_spirit = (pc.pro.team_spirit + spirit).clamp(0, 100);
        let career = pc
            .career_mut()
            .ok_or_else(|| csc_util::SimError::protocol("NPC 没有生涯，无法表态"))?;
        career.reputation = (career.reputation + reputation).clamp(0, 100);
        career.public_stance = Some(tone.clone());
        self.journal
            .record(csc_events::event::WorldEvent::LiveUpdate {
                date: self.clock.date_label(),
                seq: -1,
                headline: "公开表态".to_string(),
                detail: detail.to_string(),
            });
        Ok(detail.to_string())
    }

    /// 主动赛前 BP / 战术预案（玩家自己选择，不再等图间干预事件）。
    /// style ∈ AGGRESSIVE / BALANCED / CONSERVATIVE，作为后续顶级赛事
    /// 图间决策的默认基线（conductor 使用）。
    pub fn set_match_plan(
        &mut self,
        player_id: PlayerId,
        style: &str,
    ) -> Result<(), csc_util::SimError> {
        let style = style.trim().to_uppercase();
        if !matches!(style.as_str(), "AGGRESSIVE" | "BALANCED" | "CONSERVATIVE") {
            return Err(csc_util::SimError::protocol(format!(
                "非法赛前预案：{style}（AGGRESSIVE/BALANCED/CONSERVATIVE）"
            )));
        }
        let pc = self
            .world
            .player_mut(player_id)
            .ok_or_else(|| csc_util::SimError::protocol(format!("选手不存在：{player_id:?}")))?;
        let career = pc
            .career_mut()
            .ok_or_else(|| csc_util::SimError::protocol("NPC 没有生涯，无法设置赛前预案"))?;
        career.match_style = Some(style.clone());
        self.journal
            .record(csc_events::event::WorldEvent::LiveUpdate {
                date: self.clock.date_label(),
                seq: -1,
                headline: "赛前预案".to_string(),
                detail: format!(
                    "你为下一阶段比赛定下 {} 战术基调（主动 BP，不再等待比赛中的暂停事件）。",
                    match style.as_str() {
                        "AGGRESSIVE" => "激进",
                        "CONSERVATIVE" => "保守",
                        _ => "平衡",
                    }
                ),
            });
        Ok(())
    }

    /// 买断自己换队（2026 复审新增）：支付违约金（年薪 × 2，下限 100k）主动
    /// 解除合同 → 自由市场，下个转会窗可自由签约。违约金从个人现金扣除。
    /// 服务端外部操作；需要 world+vrs+rng 三字段协同，故收口在引擎门面。
    pub fn buyout_player(&mut self, player_id: PlayerId) -> Result<i64, csc_util::SimError> {
        csc_systems::transfer::TransferEngine::buyout(
            &mut self.world,
            &mut self.vrs,
            player_id,
            &mut self.director.rng,
        )
        .map_err(csc_util::SimError::protocol)
    }

    /// 投资自己（2026 复审新增）：每年限一次，现金 → 属性成长 + 声誉。
    /// 确定性（哈希派生加成，不消费 RNG）——不破坏可复现性。
    pub fn invest_self(
        &mut self,
        player_id: PlayerId,
        focus: &str,
        year: i32,
    ) -> Result<String, csc_util::SimError> {
        csc_systems::economy::InvestmentEngine::invest_self(&mut self.world, player_id, focus, year)
            .map_err(csc_util::SimError::protocol)
    }

    /// 购买 CS 饰品（2026 复审新增）：现金 → 藏品 + 声誉。确定性选品。
    pub fn buy_skin(
        &mut self,
        player_id: PlayerId,
        rare: bool,
        year: i32,
    ) -> Result<csc_entities::career::SkinOwned, csc_util::SimError> {
        csc_systems::economy::InvestmentEngine::buy_skin(&mut self.world, player_id, rare, year)
            .map_err(csc_util::SimError::protocol)
    }

    /// 玩家合同摘要（队伍/薪资/剩余合同时长/实力，可随时查看）。
    /// `player_id` 是外部输入（UI 查询）——反解失败返回 `Err`（不 panic）。
    pub fn player_contract(&self, player_id: PlayerId) -> Result<String, csc_util::SimError> {
        csc_systems::transfer::TransferEngine::contract_summary(&self.world, player_id)
    }

    /// 排定玩家的**主动训练计划**（网游式：想练才练，不再是月度强制决策）。
    /// 计划写入 `CareerInfo.pending_training`（存档字段，回放可复现），
    /// 在下一月 `advance_month` 开头应用。
    ///
    /// `focus` 必须是 `TrainingFocus.name()`（AIM/UTILITY/CLUTCH/PHYSICAL/
    /// MENTAL/COMMUNICATION/REST），非法值显式报错（不静默回退）。
    pub fn plan_training(
        &mut self,
        player_id: PlayerId,
        focus: &str,
    ) -> Result<(), csc_util::SimError> {
        const ALLOWED: [&str; 7] = [
            "AIM",
            "UTILITY",
            "CLUTCH",
            "PHYSICAL",
            "MENTAL",
            "COMMUNICATION",
            "REST",
        ];
        if !ALLOWED.contains(&focus) {
            return Err(csc_util::SimError::protocol(format!(
                "非法训练计划：{focus}"
            )));
        }
        let pc = self
            .world
            .player_mut(player_id)
            .ok_or_else(|| csc_util::SimError::protocol(format!("选手不存在：{player_id:?}")))?;
        let career = pc
            .career_mut()
            .ok_or_else(|| csc_util::SimError::protocol("NPC 没有生涯，无法排定训练计划"))?;
        career.pending_training = Some(focus.to_string());
        Ok(())
    }

    /// 年度结算：按本年度累计 Rating 为玩家颁发 TOP20 荣誉（转发赛事子系统）。
    pub fn year_end_awards(&mut self, year: i32, top: usize) -> Vec<PlayerId> {
        self.tournaments
            .year_end_awards(&mut self.world, &self.clock, &mut self.journal, year, top)
    }

    /// 用最新一期 standings 建初始队伍实体（同名不同队按阵容签名区分；
    /// baseline 提供真实位置，ratings 提供真实评级先验）。**初始装载必须消费
    /// 确定性 rng**——世界的起点确定，可复现性才成立。
    ///
    /// 注意：本方法在装载前应清空世界（或从空世界调用）。
    pub fn seed_entities_from_standings(
        &mut self,
        baseline: Option<&csc_entities::baseline::RoleBaseline>,
        ratings: Option<&csc_entities::baseline::RatingBaseline>,
    ) {
        let Some(latest) = self.vrs.months().last().cloned() else {
            return;
        };
        let entries = self.vrs.entries_of(&latest);
        // split borrow：vrs 只读查询结束于 entries 拷贝；rng 借自 director；world 可变
        let rng = &mut self.director.rng;
        for entry in &entries {
            let players =
                csc_entities::mapper::VrsMapper::to_players(entry, baseline, ratings, rng);
            let team = self
                .world
                .create_team(&entry.team_name, entry.ranking, entry.points);
            for pc in players {
                self.world
                    .push_character(pc, Some(team))
                    .expect("内部不变量：装载队伍必存在");
            }
            self.world.team_mut(team).expect("队伍不存在").budget =
                csc_entities::mapper::VrsMapper::initial_budget(entry.points);
        }
    }

    /// 便捷：从 (文件名, 内容) standings 装载世界（零 IO；内容由调用方读取）。
    ///
    /// 三个可选校准资产（roles_baseline / player_ratings / rating_profile）打包为
    /// [`CalibrationAssets`]——避免 `load_from_standings` 参数随资产种类持续膨胀
    /// （此前 baseline→ratings→profile 每加一个资产就 +1 个位置参数）。
    /// **外部输入边界（D6）**：资产解析/校验失败返回 `Err(SimError)`——
    /// 坏资产显式报错，不 panic、不静默空世界。
    pub fn load_from_standings(
        files: &[(String, String)],
        calibration: &CalibrationAssets<'_>,
        seed: u64,
        clock: SimClock,
    ) -> Result<Self, csc_util::SimError> {
        let mut eng = Engine::empty(seed, clock);
        let vrs = VrsEngine::from_json_files(files)?;
        eng.vrs = vrs;
        // 日期锚定：数据最新月份（Kotlin create 语义）
        if let Some(last) = eng.vrs.months().last() {
            // M13：from_standings_month 对外部标签返回 Result——坏资产显式报错
            // 而非 panic（D6 契约），包装为 SimError::Asset 随 load_from_standings 上抛。
            eng.clock = SimClock::from_standings_month(last).map_err(|e| {
                csc_util::SimError::asset("standings_*.json", format!("月份标签非法：{e}"))
            })?;
            eng.director.last_contract_year = eng.clock.year();
        }
        let baseline = calibration
            .baseline
            .map(csc_entities::baseline::RoleBaseline::from_json_str)
            .transpose()?;
        let ratings = calibration
            .ratings
            .map(csc_entities::baseline::RatingBaseline::from_json_str)
            .transpose()?;
        eng.rating_profile = calibration
            .rating_profile
            .map(csc_entities::baseline::RatingProfile::from_json_str)
            .transpose()?;
        eng.text = csc_text::TextBundle::parse(calibration.text)?;
        eng.seed_entities_from_standings(baseline.as_ref(), ratings.as_ref());
        Ok(eng)
    }

    /// 推进一天（2026 日级自动推进；LIVE 闸门 t12 两阶段拆解）。
    ///
    /// - **非月末**：只推进时钟 + 每日 tick + 文字直播，**不结算赛事**——当月
    ///   fixtures 全程保持 pending，比赛日当天 `live/today` 可观测（20s 提示条）。
    /// - **月末**：先 `settle_month` 结算刚过完的当月已登记赛事，再经
    ///   `advance_month_plan` 进入新月并**只登记**（不结算）——新月 fixtures 保持
    ///   pending，供下月日级推进时在比赛日触发 LIVE。
    ///
    /// **t16 日级跨月修复**：`advance_month_plan` 登记新月赛事时会把时钟推进到
    /// 新月最后一场赛事的开始日，随后若走月推进的 `advance_month_finish(restore=true)`
    /// 会再把时钟收束到新月末——新月内各比赛日（如 start 20-27 日）从未被逐日经过，
    /// `live/today` 恒空、20s 提示条永不触发。修复：日级跨月把时钟**回拨到新月 1 号**，
    /// 再以 `restore_to_end=false` 跑月出口副作用（VRS reseed/世界批次语义同月推进），
    /// 使新月逐日由后续 `advance_day` 非月末分支自然经过（每个比赛日可观测）。
    pub fn advance_day(
        &mut self,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        let (year, month, day) = self.clock.now();
        let month_end = day == csc_time::civil::days_in_month(year, month);
        if month_end {
            self.settle_month(decision)?; // 结算刚过完的当月（月出口）
            self.advance_month_plan(decision)?; // 进入新月 + 登记（不结算，保持 pending）
            // 日级跨月：回拨时钟到新月 1 号，避免新月赛事被一次性跳过（t16）。
            let (y, m, _) = self.clock.now();
            self.clock.restore_to(y, m, 1);
            self.advance_month_finish(decision, false)?; // 月出口副作用，但不收束到月终
            let (headline, detail) =
                monthly_live_summary(&self.world, &self.clock, self.month(), &self.text);
            self.journal
                .record(csc_events::event::WorldEvent::LiveUpdate {
                    date: self.clock.date_label(),
                    seq: -1,
                    headline,
                    detail,
                });
        } else {
            self.clock.advance_days(1);
            self.daily_career_tick();
            let updates =
                daily_live_updates(&self.world, &self.loop_, &self.text, &mut self.director.rng);
            for (headline, detail) in updates {
                self.journal
                    .record(csc_events::event::WorldEvent::LiveUpdate {
                        date: self.clock.date_label(),
                        seq: -1,
                        headline,
                        detail,
                    });
            }
        }
        #[cfg(debug_assertions)]
        crate::nan::assert_no_nan(&self.snapshot());
        Ok(())
    }

    /// 连续推进 `days` 天（日级自动推进循环）。
    pub fn run_days(
        &mut self,
        days: i32,
        decision: &mut dyn DecisionSource,
    ) -> Result<(), csc_util::SimError> {
        for _ in 0..days {
            self.advance_day(decision)?;
        }
        Ok(())
    }

    /// 每日职业状态 tick：恢复是日常节奏的一部分，赛事结算仍由既有月度管线负责。
    fn daily_career_tick(&mut self) {
        for player in &mut self.world.players {
            if player.retired {
                continue;
            }
            player.fatigue = (player.fatigue - 1.5).max(0.0);
        }
    }

    /// 创建主角（**新游戏流程**，服务端/CLI/WASM 共用）：生成一名玩家角色，
    /// 替换 `team_id` 队伍中最弱的 NPC（roster 保持 5 人不变式），并同步
    /// VRS 阵容迁移（签名变化）。主角属性由传入 rng 确定性生成——同一
    /// (seed, name, team) 三元组得到同一名新秀。
    ///
    /// 单主角语义：世界中已存在主角时返回 `Err(SimError::Protocol)`
    /// （A3 多主角是实体层能力，游戏流暂单主角）。
    ///
    /// @param rng 主角属性生成随机源（独立于引擎主 RNG——人物卡生成与
    ///        世界推进解耦，同一世界可换主角卡）
    /// @param role 主角角色定位（None = 随机）
    pub fn create_protagonist(
        &mut self,
        name: &str,
        tier: csc_domain::tier::Tier,
        team_id: csc_util::id::TeamId,
        rng: &mut Xoshiro256StarStar,
        role: Option<csc_entities::role::Role>,
    ) -> Result<csc_util::id::PlayerId, csc_util::SimError> {
        if !self.world.all_players_only().is_empty() {
            return Err(csc_util::SimError::protocol(format!(
                "世界已存在主角（单主角游戏流）——拒绝创建 {name}"
            )));
        }
        // 目标队伍最弱 NPC（替换对象）
        let weakest = self
            .world
            .team(team_id)
            .ok_or_else(|| csc_util::SimError::protocol(format!("目标队伍 {:?} 不存在", team_id)))?
            .roster_ids
            .iter()
            .filter_map(|pid| self.world.player(*pid))
            .filter(|p| p.is_npc())
            .min_by(|a, b| {
                csc_entities::power::PowerCalculator::player_power(a)
                    .total_cmp(&csc_entities::power::PowerCalculator::player_power(b))
            })
            .map(|p| p.id)
            .ok_or_else(|| {
                csc_util::SimError::protocol(format!(
                    "队伍「{}」没有可替换的 NPC",
                    self.world
                        .team(team_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_default()
                ))
            })?;

        let old_sig = self.world.signature_of(team_id);
        self.world
            .release_player(weakest)
            .expect("内部不变量：被换 NPC 必在 roster");
        self.world
            .add_free_agent(weakest)
            .expect("内部不变量：被换 NPC 可进自由市场");
        // 主角固定 16 岁起步（青训新秀叙事）；角色可指定或随机
        let player_id = self
            .world
            .create_player(tier, Some(name), rng, Some(16), role);
        self.world
            .assign_player_to_team(player_id, team_id)
            .expect("内部不变量：主角/目标队必存在");
        // 青训合同（2026 试玩修复）：开局即签 1 年低薪约（市场价 30%）——
        // 此前主角 0 年薪 0 合同打满 4 个月白工，直到首个转会窗才签约。
        {
            let power = csc_entities::power::PowerCalculator::player_power(
                self.world.player(player_id).expect("主角存在"),
            );
            let market = csc_simulation::TransferRules::salary_of(power, 50);
            if let Some(career) = self
                .world
                .player_mut(player_id)
                .expect("主角存在")
                .career_mut()
            {
                career.salary = ((market as f64 * 0.3).round() as i64).max(10_000);
                career.contract_years = 1;
            }
        }

        // VRS 阵容迁移（签名变化；出走 ≥3 人积分清零语义由 VRS 侧处理）
        let new_names: Vec<String> = self
            .world
            .roster(team_id)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        self.vrs.apply_roster_change_verified(&old_sig, &new_names);
        Ok(player_id)
    }
}

/// 便捷：全自动决策源（世界推进的默认决策方）。
pub fn auto_decision() -> AutoDecisionSource {
    AutoDecisionSource
}

/// 日级文字直播消息池（确定性：由引擎 RNG 逐条抽取，不破坏可复现性）。
fn daily_live_updates(
    world: &World,
    loop_: &crate::loop_::SeasonLoop,
    text: &csc_text::TextBundle,
    rng: &mut Xoshiro256StarStar,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(player) = world.players.iter().find(|p| p.is_player() && !p.retired) else {
        return vec![(
            text.get("live.retired.title").to_string(),
            text.get("live.retired.body").to_string(),
        )];
    };
    let team = player.team.and_then(|tid| world.team(tid));
    let team_name = team
        .map(|t| t.name.as_str())
        .unwrap_or_else(|| text.get("live.free_agent"));
    let lock = team.and_then(|_| {
        loop_
            .lock_records()
            .into_iter()
            .find(|l| l.team_ids.iter().any(|id| Some(*id) == player.team))
    });

    if let Some(l) = lock {
        out.push((
            text.get("live.tournament.title").to_string(),
            text.format("live.tournament.body", &[team_name, &l.event_name]),
        ));
        out.push((
            text.get("live.practice.title").to_string(),
            text.get("live.practice.body").to_string(),
        ));
        return out;
    }
    let contacts = player
        .career
        .as_ref()
        .map(|c| c.transfer_contacts.len())
        .unwrap_or(0);
    if contacts > 0 {
        let pool = [
            "转会动态：经纪人收到多支队伍的私下问价，市场正在升温。",
            "转会流言：你的名字出现在不止一份引援名单上。",
            "媒体观察：有俱乐部官员在采访中称赞你近期的表现。",
        ];
        let text = pool[rng.next_i32_bound(pool.len() as i32) as usize];
        out.push(("转会动态".into(), text.to_string()));
    }
    if player
        .career
        .as_ref()
        .and_then(|c| c.pending_training.as_ref())
        .is_some()
    {
        out.push((
            text.get("live.training_plan.title").to_string(),
            text.get("live.training_plan.body").to_string(),
        ));
    }
    if player
        .career
        .as_ref()
        .map(|c| c.contract_years == 1)
        .unwrap_or(false)
    {
        out.push((
            text.get("live.contract.title").to_string(),
            text.get("live.contract.body").to_string(),
        ));
    }
    if let Some(team) = team {
        let teammate = world
            .roster(team.id)
            .into_iter()
            .find(|p| p.id != player.id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| text.get("live.teammate_fallback").to_string());
        // 文案池（titles/details 平行列表，长度 3）——RNG 消耗与硬编码时代一致
        let pick = rng.next_i32_bound(3) as usize;
        let title = text.pick_format("live.teammate.titles", pick, &[&teammate]);
        let detail = text.pick_format("live.teammate.details", pick, &[team_name]);
        out.push((title, detail));
    }
    out
}

/// 月结直播摘要（月末完整月步之后补一条）。
fn monthly_live_summary(
    world: &World,
    clock: &SimClock,
    month: i32,
    text: &csc_text::TextBundle,
) -> (String, String) {
    let player = world.players.iter().find(|p| p.is_player() && !p.retired);
    let name = player
        .map(|p| p.name.as_str())
        .unwrap_or_else(|| text.get("live.monthly.self"));
    let (year, mon, _) = clock.now();
    (
        text.format(
            "live.monthly.headline",
            &[&year.to_string(), &format!("{mon:02}"), &month.to_string()],
        ),
        text.format("live.monthly.body", &[name]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;

    fn minimal_story() -> crate::narrative::NarrativeContent {
        crate::narrative::NarrativeContent::from_json_str(
            r#"{
          "content_version":2,"arcs":[{"id":"a","title":"A"}],
          "scenes":[{"id":"a.first","arc":"a","title":"Hello","actor_role":"teammate",
          "trigger":{"kind":"arc_entry","arc":"a"},"choices":[{"id":"OK","label":"ok"}]}]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn story_content_mismatch_and_missing_scene_leave_state_untouched() {
        let mut engine = Engine::empty(7, SimClock::of(2026, 1, 1));
        let content = minimal_story();
        engine.set_narrative_content(content.clone()).unwrap();
        assert_eq!(engine.narrative.progress.content_version, 2);
        let facts = engine.narrative_facts();
        engine.begin_scene(&content.scenes[0], &facts);
        let before = engine.snapshot();
        let mut changed = content;
        changed.content_version = 3;
        assert!(engine.set_narrative_content(changed).is_err());
        assert_eq!(before, engine.snapshot());
        let mut bad = before.clone();
        bad.narrative.active_scene.as_mut().unwrap().scene_id = "missing".into();
        assert!(engine.restore_result(bad).is_err());
        assert_eq!(before, engine.snapshot());
    }

    #[test]
    fn fork_snapshot_preserves_content_and_does_not_consume_rng() {
        let mut engine = Engine::empty(7, SimClock::of(2026, 1, 1));
        let content = minimal_story();
        engine.set_narrative_content(content.clone()).unwrap();
        engine.begin_scene(&content.scenes[0], &engine.narrative_facts());
        let before = engine.snapshot();
        let fork = engine.fork_snapshot().unwrap();
        assert_eq!(before, engine.snapshot());
        assert_eq!(before, fork.snapshot());
        assert_eq!(fork.narrative.active_scene_def().unwrap().id, "a.first");
        assert_eq!(
            engine.text.get("season.vrs.headline"),
            fork.text.get("season.vrs.headline")
        );
    }

    #[test]
    fn only_teammate_role_binds_an_actual_teammate() {
        let mut engine = Engine::empty(7, SimClock::of(2026, 1, 1));
        let tid = engine.world.create_team("Test", 1, 1000);
        let pid = engine.world.create_player(
            Tier::Tier4,
            Some("P"),
            &mut Xoshiro256StarStar::seed(7),
            None,
            None,
        );
        engine.world.assign_player_to_team(pid, tid).unwrap();
        let mate = engine
            .world
            .create_npc(
                Tier::Tier4,
                Some(tid),
                None,
                Some("Mate"),
                &mut Xoshiro256StarStar::seed(8),
                None,
            )
            .unwrap();
        let mut scene = minimal_story().scenes.remove(0);
        for role in ["coach", "captain", "manager", "media"] {
            scene.actor_role = role.into();
            engine.begin_scene(&scene, &engine.narrative_facts());
            assert!(
                engine
                    .narrative
                    .progress
                    .active_scene
                    .as_ref()
                    .unwrap()
                    .actor_id
                    .is_none()
            );
        }
        scene.actor_role = "teammate".into();
        engine.begin_scene(&scene, &engine.narrative_facts());
        assert_eq!(
            engine
                .narrative
                .progress
                .active_scene
                .as_ref()
                .unwrap()
                .actor_id,
            Some(mate)
        );
    }

    #[test]
    fn create_protagonist_replaces_weakest_npc_and_migrates_vrs() {
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        let t = eng.world.create_team("Vitality", 1, 2000);
        for i in 0..5 {
            eng.world
                .create_npc(
                    Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("N{i}")),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                )
                .unwrap();
        }
        // VRS 登记该队（阵容迁移验证）
        let old_sig = eng.world.signature_of(t);
        eng.vrs.apply_roster_change_verified(
            &old_sig,
            &[
                "N0".to_string(),
                "N1".to_string(),
                "N2".to_string(),
                "N3".to_string(),
                "N4".to_string(),
            ],
        );

        let pid = eng
            .create_protagonist(
                "MyPlayer",
                Tier::Tier3,
                t,
                &mut Xoshiro256StarStar::seed(7),
                None,
            )
            .expect("主角创建成功");
        assert_eq!(eng.player(), Some(pid), "主角是唯一玩家实体");
        assert_eq!(
            eng.world.team(t).unwrap().roster_ids.len(),
            5,
            "roster 保持 5 人"
        );
        assert!(
            eng.world.team(t).unwrap().roster_ids.contains(&pid),
            "主角已入队"
        );
        // 被换 NPC 进自由市场
        assert_eq!(eng.world.all_free_agents().len(), 1);
        // VRS 签名已迁移
        assert_ne!(eng.world.signature_of(t), old_sig, "阵容变化 → 签名变化");
        // 单主角语义：二次创建被拒
        let again = eng.create_protagonist(
            "Second",
            Tier::Tier3,
            t,
            &mut Xoshiro256StarStar::seed(8),
            None,
        );
        assert!(again.is_err(), "已存在主角时拒绝二次创建");
    }

    #[test]
    fn plan_training_applies_next_month_and_persists() {
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        // 日历推进需要满编 40 队（与 soak 同构：T1/T2 池都要够人）
        for ti in 0..40 {
            let team = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                eng.world
                    .create_npc(
                        Tier::Tier1,
                        Some(team),
                        None,
                        Some(&format!("T{ti}P{i}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
        }
        let t = eng.world.teams[0].id;
        let pid = eng
            .create_protagonist(
                "MyPlayer",
                Tier::Tier3,
                t,
                &mut Xoshiro256StarStar::seed(7),
                None,
            )
            .expect("主角创建成功");

        // 非法计划显式报错（不静默回退）
        assert!(eng.plan_training(pid, "HACK").is_err());
        eng.plan_training(pid, "AIM").expect("合法训练计划");
        assert_eq!(
            eng.world
                .player(pid)
                .unwrap()
                .career
                .as_ref()
                .unwrap()
                .pending_training
                .as_deref(),
            Some("AIM")
        );

        eng.advance_month(&mut AutoDecisionSource)
            .expect("auto 推进必成功");
        assert_eq!(
            eng.world
                .player(pid)
                .unwrap()
                .career
                .as_ref()
                .unwrap()
                .pending_training,
            None,
            "计划应用后清空"
        );
        assert!(
            eng.journal.all().iter().any(|e| matches!(e, csc_events::event::WorldEvent::TrainingDone { player_id, focus, .. } if *player_id == pid && focus == "AIM")),
            "应产出 TrainingDone 事件"
        );

        // 存档 v3 含 pending_training（再排一次后快照往返）
        eng.plan_training(pid, "REST").expect("排定休整");
        let state = eng.snapshot();
        assert_eq!(state.version, GameState::CURRENT_VERSION);
        let json = serde_json::to_string(&state).unwrap();
        let mut restored = Engine::empty(0, SimClock::of(2000, 1, 1));
        restored.restore(serde_json::from_str(&json).unwrap());
        assert_eq!(
            restored
                .world
                .player(pid)
                .unwrap()
                .career
                .as_ref()
                .unwrap()
                .pending_training
                .as_deref(),
            Some("REST"),
            "主动训练计划随存档恢复"
        );
    }

    #[test]
    fn snapshot_restore_full_roundtrip() {
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        // 建满编世界：40 队 × 5 人（T1 瑞士轮 ≥8 队门槛 + T2 池 13~32 名需要队伍）
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                let name = if ti == 0 && i == 0 {
                    "MyPlayer".to_string()
                } else {
                    format!("T{ti}P{i}")
                };
                if ti == 0 && i == 0 {
                    let p = eng.world.create_player(
                        Tier::Tier1,
                        Some(&name),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                        None,
                    );
                    eng.world.assign_player_to_team(p, t).unwrap();
                } else {
                    eng.world
                        .create_npc(
                            Tier::Tier1,
                            Some(t),
                            None,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                        )
                        .unwrap();
                }
            }
        }
        // 推进 2 个月
        eng.run_season(2, &mut AutoDecisionSource)
            .expect("auto 推进必成功");
        assert!(eng.director.month >= 2);
        assert!(!eng.journal.is_empty());

        let state = eng.snapshot();
        let json = serde_json::to_string(&state).unwrap();

        // 全新引擎恢复
        let mut eng2 = Engine::empty(0, SimClock::of(2000, 1, 1));
        eng2.restore(serde_json::from_str(&json).unwrap());
        assert_eq!(eng2.director.month, eng.director.month);
        assert_eq!(eng2.clock.date_label(), eng.clock.date_label());
        assert_eq!(eng2.world, eng.world, "实体 arena 完整恢复");
        assert_eq!(eng2.journal.len(), eng.journal.len(), "事件流完整恢复");
        assert_eq!(
            eng2.decision_log.count(),
            eng.decision_log.count(),
            "决策日志完整恢复"
        );
        assert_eq!(
            eng2.tournaments.results().len(),
            eng.tournaments.results().len(),
            "赛事结果完整恢复"
        );
        // 恢复后继续推进，两边世界一致（可复现性）
        let state_a = eng.snapshot();
        let mut b = eng2;
        let mut a = Engine::empty(0, SimClock::of(2000, 1, 1));
        a.restore(state_a);
        a.advance_month(&mut AutoDecisionSource)
            .expect("auto 推进必成功");
        b.advance_month(&mut AutoDecisionSource)
            .expect("auto 推进必成功");
        assert_eq!(b.clock.date_label(), a.clock.date_label());
        assert_eq!(
            b.director.rng_snapshot(),
            a.director.rng_snapshot(),
            "RNG 轨迹一致"
        );
        assert_eq!(b.vrs.database(), a.vrs.database(), "VRS 状态一致");
    }

    /// P1-4：读档原子性——坏档（锁引用不存在的队伍 ID）restore_result 返回 Err，
    /// 且原引擎状态与之前完全一致（构造-交换，全有或全无；不做半恢复）。
    #[test]
    fn restore_bad_save_leaves_engine_untouched() {
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                eng.world
                    .create_npc(
                        Tier::Tier1,
                        Some(t),
                        None,
                        Some(&format!("T{ti}P{i}")),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                    )
                    .unwrap();
            }
        }
        // 推进 2 个月（产生赛事锁与 journal，使坏档篡改有真实影响面）
        eng.run_season(2, &mut AutoDecisionSource)
            .expect("auto 推进必成功");

        // 取快照：篡改 locks 为「引用不存在队伍」的坏档（锁反解必失败）
        let mut bad = eng.snapshot();
        bad.version = GameState::CURRENT_VERSION;
        bad.locks = vec![crate::state::LockRecord {
            event_name: "T1 Bad".into(),
            end_day: 6,
            start_epoch_day: 10,
            end_epoch_day: 17,
            team_ids: vec![csc_util::id::TeamId(999_999)], // 不存在
        }];

        // 恢复前快照原状（比较用）
        let before = eng.snapshot();
        let res = eng.restore_result(bad);
        assert!(res.is_err(), "坏档必须返回 Err（不再 panic）");
        let after = eng.snapshot();
        assert_eq!(
            after, before,
            "坏档 restore 失败后原引擎必须与之前完全一致（读档原子性）"
        );
    }

    /// LIVE 闸门两阶段拆解验收：日级推进进入新月后，主角 pending 对阵在**登记后、结算前**
    /// 跨调用可观测（`player_live_fixtures`/`live/today` 在比赛日非空）；`settle_month`
    /// 后记录转为 Completed（跳过/结算路径），Auto 月推进不受闸门影响。
    #[test]
    fn live_gate_pending_visible_on_match_day_then_settles() {
        // 建满编世界（复用 roundtrip 的构造）。
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                let name = if ti == 0 && i == 0 {
                    "MyPlayer".to_string()
                } else {
                    format!("T{ti}P{i}")
                };
                if ti == 0 && i == 0 {
                    let p = eng.world.create_player(
                        Tier::Tier1,
                        Some(&name),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                        None,
                    );
                    eng.world.assign_player_to_team(p, t).unwrap();
                } else {
                    eng.world
                        .create_npc(
                            Tier::Tier1,
                            Some(t),
                            None,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                        )
                        .unwrap();
                }
            }
        }

        // 阶段 1：登记新月赛事为 Future（不结算）。主角队应被排程进某场赛事。
        eng.advance_month_plan(&mut AutoDecisionSource)
            .expect("plan 必成功");
        let snap = eng.snapshot();
        // 存在主角 pending 登记记录（Future，fixtures 有待开赛对阵）。
        let has_pending = snap.scheduled_records.iter().any(|r| {
            r.player_team_id.is_some()
                && matches!(
                    r.status,
                    csc_tournaments::scheduled::ScheduledStatus::Future
                )
                && r.fixtures
                    .iter()
                    .any(|f| f.status == csc_tournaments::scheduled::FixtureStatus::Pending)
        });
        assert!(has_pending, "plan 后主角应有 pending 登记赛事");

        // 结算前，主角比赛日当天 live/today 可观测（非空）。
        // 取主角下一场 pending 赛事的日期，把快照时钟对齐到该日再投影。
        let rec = snap
            .scheduled_records
            .iter()
            .find(|r| r.player_team_id.is_some())
            .expect("应有主角赛事");
        // 把 sim 日期设为该赛事开赛日（模拟「日级推进到比赛日」的观察时点）。
        let day: u32 = rec.date.rsplit('-').next().unwrap().parse().unwrap();
        let mut state = snap.clone();
        state.sim_day = day.max(1);
        let live = crate::client::player_live_fixtures(&state);
        assert!(!live.is_empty(), "比赛日当天 live/today 应非空");
        assert_eq!(live[0].event_name, rec.event_name);

        // 阶段 2：结算（跳过/月出口路径）→ 该记录转为 Completed。
        eng.settle_month(&mut AutoDecisionSource)
            .expect("settle 必成功");
        let after = eng.snapshot();
        assert!(
            after.scheduled_records.iter().any(|r| {
                r.player_team_id.is_some()
                    && r.status == csc_tournaments::scheduled::ScheduledStatus::Completed
            }),
            "settle 后主角赛事应 Completed"
        );
        // 结算后 live/today 回到空（比赛已打完，不再是 pending）。
        assert!(
            crate::client::player_live_fixtures(&after).is_empty(),
            "结算后 live/today 应为空"
        );
    }

    /// R2 ① 回归：开局物化当前月（季前月 2026-01）赛事排程 + fixtures + 锁，
    /// 消除「卡赛事」（主页承诺 1 月有比赛但引擎此前不排程、比赛日静默滑过）。
    #[test]
    fn plan_opening_month_materializes_opening_month() {
        // 建满编世界（复用 live_gate 测试构造）：40 队 × 5 人，主角 MyPlayer 在 Team0。
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                let name = if ti == 0 && i == 0 {
                    "MyPlayer".to_string()
                } else {
                    format!("T{ti}P{i}")
                };
                if ti == 0 && i == 0 {
                    let p = eng.world.create_player(
                        Tier::Tier1,
                        Some(&name),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                        None,
                    );
                    eng.world.assign_player_to_team(p, t).unwrap();
                } else {
                    eng.world
                        .create_npc(
                            Tier::Tier1,
                            Some(t),
                            None,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                        )
                        .unwrap();
                }
            }
        }
        let protagonist_team = eng
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .and_then(|p| p.team)
            .expect("主角有队伍");

        // 开局物化 1 月（month_idx = -1，与 season_calendar_plan 平移一致）。
        eng.plan_opening_month(-1);

        let snap = eng.snapshot();
        // 1 月赛事已排程登记（date 以 2026-01- 开头）。
        assert!(
            !snap.scheduled_records.is_empty(),
            "开局物化后 1 月赛事应已登记"
        );
        assert!(
            snap.scheduled_records
                .iter()
                .all(|r| r.date.starts_with("2026-01-")),
            "物化的应是当前月（1 月）赛事，而非 2 月错位"
        );
        // 主角队被排程进至少一场赛事（Future + pending fixtures）。
        assert!(
            snap.scheduled_records.iter().any(|r| {
                r.player_team_id == Some(protagonist_team)
                    && r.status == csc_tournaments::scheduled::ScheduledStatus::Future
                    && !r.fixtures.is_empty()
            }),
            "主角队应被排程进 1 月赛事（含 pending fixtures）"
        );
        // 队伍锁已建立（开局月赛事锁不空）。
        assert!(!snap.locks.is_empty(), "开局物化后应建立队伍锁");
        // 时钟恢复到开局日（01-01），后续日级推进逐日经过比赛日。
        assert_eq!(
            (snap.sim_year, snap.sim_month, snap.sim_day),
            (2026, 1, 1),
            "物化后时钟应恢复到开局日，而非停在月末赛事日"
        );
        // 1 月赛事名与 2 月（m=0）不冲突：本物化用 m=-1 → 名字 #0-*（R2.1 起
        // slot 按月内场次归一化，如 `#0-1..#0-8`，杜绝 `#0--7` 负号畸形）。
        assert!(
            snap.scheduled_records
                .iter()
                .all(|r| !r.event_name.contains("#1-")),
            "开局物化（m=-1）的赛事名应为 #0-*，不与 2 月 #1-* 冲突"
        );
        // R2.1：负 pool_offset 不得泄漏进赛事名（t5 `#0--7` 畸形证据）。
        assert!(
            snap.scheduled_records
                .iter()
                .all(|r| !r.event_name.contains("--")),
            "开局月赛事名不得含 `--` 负号畸形（R2.1 slot 归一化）"
        );
    }

    /// B（开局月结算闭环）+ A（跨月去重唯一实例）：开局物化 1 月赛事后首次月推进
    /// 必须先结算 1 月（2026-01 记录 → Completed，不再滞留 Future），且
    /// player_events 中不得出现同名 future/active 交错并存（t5 `#0--7` 证据）。
    #[test]
    fn opening_month_settles_on_first_month_advance_no_duplicates() {
        // 建满编世界（与 plan_opening_month 测试同构）：40 队 × 5 人。
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                let name = if ti == 0 && i == 0 {
                    "MyPlayer".to_string()
                } else {
                    format!("T{ti}P{i}")
                };
                if ti == 0 && i == 0 {
                    let p = eng.world.create_player(
                        Tier::Tier1,
                        Some(&name),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                        None,
                    );
                    eng.world.assign_player_to_team(p, t).unwrap();
                } else {
                    eng.world
                        .create_npc(
                            Tier::Tier1,
                            Some(t),
                            None,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                        )
                        .unwrap();
                }
            }
        }
        let protagonist_team = eng
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .and_then(|p| p.team)
            .expect("主角有队伍");

        // 开局物化 1 月（month_idx = -1）。
        eng.plan_opening_month(-1);
        let snap_before = eng.snapshot();
        let jan_future_before = snap_before
            .scheduled_records
            .iter()
            .filter(|r| {
                r.date.starts_with("2026-01-")
                    && r.status == csc_tournaments::scheduled::ScheduledStatus::Future
            })
            .count();
        assert!(jan_future_before > 0, "物化后 1 月应有 Future 记录");

        // 首次月推进：必须结算 1 月（B 契约）。
        eng.advance_month(&mut AutoDecisionSource)
            .expect("首次月推进必成功");
        let snap_after = eng.snapshot();

        // 1 月记录全部 Completed（开局月可结算，不再滞留 future）。
        let jan_records: Vec<_> = snap_after
            .scheduled_records
            .iter()
            .filter(|r| r.date.starts_with("2026-01-"))
            .collect();
        assert!(!jan_records.is_empty(), "1 月记录应保留在排程档案中");
        assert!(
            jan_records
                .iter()
                .all(|r| r.status == csc_tournaments::scheduled::ScheduledStatus::Completed),
            "1 月全部记录应为 Completed（开局月闭环）——未来态不得滞留"
        );
        // 主角 1 月赛事已完成（若主角参赛）。
        let jan_player: Vec<_> = jan_records
            .iter()
            .filter(|r| r.player_team_id == Some(protagonist_team))
            .collect();
        if !jan_player.is_empty() {
            assert!(
                jan_player
                    .iter()
                    .all(|r| r.status == csc_tournaments::scheduled::ScheduledStatus::Completed),
                "主角 1 月赛事应已结算（active→completed 正常流转）"
            );
        }
        // 2 月赛事已登记（月推进进入 2 月并结算——快进路径）。
        let feb_records: Vec<_> = snap_after
            .scheduled_records
            .iter()
            .filter(|r| r.date.starts_with("2026-02-"))
            .collect();
        assert!(!feb_records.is_empty(), "首次月推进后 2 月赛事应已排程");

        // A 契约：同一 label 不得有重复/交错实例——按事件名分组必须唯一。
        let mut names = std::collections::HashMap::new();
        for r in &snap_after.scheduled_records {
            let count = names.entry(r.event_name.clone()).or_insert(0usize);
            *count += 1;
        }
        assert!(
            names.values().all(|&c| c == 1),
            "同名赛事必须唯一实例（A 跨月去重）：{names:?}"
        );
        // t5 畸形名精确回归：`#0--7` 不再存在。
        assert!(
            snap_after
                .scheduled_records
                .iter()
                .all(|r| !r.event_name.contains("--")),
            "畸形负号名（#0--7）不得存在"
        );
        // 开局月名（#0-*）与 2 月名（#1-*）分属不同实例、不同日期。
        let jan_names: std::collections::HashSet<&str> =
            jan_records.iter().map(|r| r.event_name.as_str()).collect();
        let feb_names: std::collections::HashSet<&str> =
            feb_records.iter().map(|r| r.event_name.as_str()).collect();
        assert!(
            jan_names.is_disjoint(&feb_names),
            "开局月与 2 月赛事名必须互斥（跨月同名畸形消除）"
        );
        // 结果归档：1 月赛事进入完整结果（calendar results 可见）。
        let results = eng.tournaments().results();
        let jan_results = results
            .iter()
            .filter(|r| r.event.date.starts_with("2026-01"))
            .count();
        assert_eq!(
            jan_results, 15,
            "1 月 15 场赛事全部完赛归档（2 T1 + 5 T2 + 8 T3）"
        );
    }

    /// D（确定性不变）：同种子同决策序列，R2.1 修复后两次独立运行的世界
    /// 语义相等（世界演化不依赖 HashMap 迭代序；JSON 字节序差异来自
    /// `VrsDatabase.teams` HashMap 键序的既有序列化噪声，不属本任务范围）。
    #[test]
    fn r21_determinism_same_seed_same_world() {
        let build_and_run = |seed: u64| {
            let mut eng = Engine::empty(seed, SimClock::of(2026, 1, 1));
            for ti in 0..40 {
                let t = eng
                    .world
                    .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
                for i in 0..5 {
                    let name = if ti == 0 && i == 0 {
                        "MyPlayer".to_string()
                    } else {
                        format!("T{ti}P{i}")
                    };
                    if ti == 0 && i == 0 {
                        let p = eng.world.create_player(
                            Tier::Tier1,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                            None,
                        );
                        eng.world.assign_player_to_team(p, t).unwrap();
                    } else {
                        eng.world
                            .create_npc(
                                Tier::Tier1,
                                Some(t),
                                None,
                                Some(&name),
                                &mut Xoshiro256StarStar::seed(1),
                                None,
                            )
                            .unwrap();
                    }
                }
            }
            eng.plan_opening_month(-1);
            eng.advance_month(&mut AutoDecisionSource).expect("推进");
            eng.snapshot()
        };
        let a = build_and_run(42);
        let b = build_and_run(42);
        assert_eq!(a, b, "同种子同决策 = 同一世界（D 确定性不变，语义相等）");
        // RNG 轨迹一致（快照内含 rng_state）。
        assert_eq!(a.rng_state, b.rng_state, "RNG 状态必须一致");
        // 事件流一致（journal 有序）。
        assert_eq!(a.journal, b.journal, "事件流必须一致");
        let c = build_and_run(43);
        assert_ne!(a, c, "不同种子应产生不同世界（sanity）");
    }

    /// t16 日级跨月回归：`advance_day` 跨月后必须**逐日经过新月每一天**
    /// （1/31→2/1→2/2...），不得一次性跳到新月末——否则新月赛事从未被逐日经过，
    /// `live/today` 恒空、前端 20s LIVE 提示条永不触发（用户「日期跳太快 LIVE 消失」根因）。
    ///
    /// 验收：
    /// 1. 从 1/1 起逐日 `advance_day`，每一步时钟**恰好前进 1 天**（跨 1→2 月不跳日）；
    /// 2. 新月主角队伍赛事 start 日当天 `player_live_fixtures` 返回该 pending 对阵，
    ///    前一天为空、比赛日窗口跨多次日推持续可观测；
    /// 3. 走到新月末下一天触发 settle，新月赛事回填 Completed、live/today 归空。
    #[test]
    fn advance_day_crosses_month_without_skipping_dates() {
        // 建满编世界（复用 live_gate 测试构造）：40 队 × 5 人，主角 MyPlayer 在 Team0。
        let mut eng = Engine::empty(42, SimClock::of(2026, 1, 1));
        for ti in 0..40 {
            let t = eng
                .world
                .create_team(format!("Team{ti}"), ti + 1, 2000 - ti * 50);
            for i in 0..5 {
                let name = if ti == 0 && i == 0 {
                    "MyPlayer".to_string()
                } else {
                    format!("T{ti}P{i}")
                };
                if ti == 0 && i == 0 {
                    let p = eng.world.create_player(
                        Tier::Tier1,
                        Some(&name),
                        &mut Xoshiro256StarStar::seed(1),
                        None,
                        None,
                    );
                    eng.world.assign_player_to_team(p, t).unwrap();
                } else {
                    eng.world
                        .create_npc(
                            Tier::Tier1,
                            Some(t),
                            None,
                            Some(&name),
                            &mut Xoshiro256StarStar::seed(1),
                            None,
                        )
                        .unwrap();
                }
            }
        }

        let mut prev = eng.clock.now();
        let mut crossed_feb = false;
        let mut feb_live_days: Vec<(i32, u32, u32)> = Vec::new();
        // 从 1/1 逐日推进 65 天：覆盖 1/31→2 月全部比赛日→2 月末 settle。
        for _ in 0..65 {
            eng.advance_day(&mut AutoDecisionSource)
                .expect("日推必成功");
            let cur = eng.clock.now();
            // 关键验收 1：每步恰好 +1 天（逐日经过，跨月不跳日）。
            let cur_epoch = csc_time::civil::days_from_civil(cur.0, cur.1, cur.2);
            let prev_epoch = csc_time::civil::days_from_civil(prev.0, prev.1, prev.2);
            assert_eq!(
                cur_epoch - prev_epoch,
                1,
                "advance_day 必须逐日前进，不得跳月/跳日：{:04}-{:02}-{:02}",
                cur.0,
                cur.1,
                cur.2
            );
            prev = cur;
            if cur.1 == 2 {
                crossed_feb = true;
                let snap = eng.snapshot();
                let live = crate::client::player_live_fixtures(&snap);
                if !live.is_empty() {
                    // 窗口跨多次读持续：同一比赛日快照多次投影恒非空且对阵一致
                    //（LIVE 闸门只读旁路，不因重复读取而消失）。
                    let live2 = crate::client::player_live_fixtures(&snap);
                    assert!(
                        !live2.is_empty() && live[0].event_name == live2[0].event_name,
                        "比赛日 live/today 应跨多次读稳定非空"
                    );
                    feb_live_days.push(cur);
                }
            }
        }

        // 关键验收 2：已跨入 2 月，且 2 月内有至少一个比赛日被逐日观察到 LIVE。
        assert!(crossed_feb, "逐日推进应已跨入 2 月");
        assert!(
            !feb_live_days.is_empty(),
            "2 月内应至少有一个比赛日 live/today 非空（LIVE 比赛日可达）"
        );

        // 关键验收 3：走完 2 月末，2 月赛事已结算 Completed、live/today 归空。
        let snap = eng.snapshot();
        assert!(
            snap.scheduled_records
                .iter()
                .filter(|r| r.player_team_id.is_some())
                .any(|r| r.status == csc_tournaments::scheduled::ScheduledStatus::Completed),
            "跨月后（3 月初）2 月主角赛事应已 Completed"
        );
        assert!(
            crate::client::player_live_fixtures(&snap).is_empty(),
            "结算后 live/today 应为空"
        );
    }
}
