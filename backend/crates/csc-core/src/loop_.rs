//! 赛季循环编排（Kotlin `SeasonLoop.kt` 转写）：**队伍级互斥锁 + 月度赛事编排**。
//!
//! 职责：
//! - 持有本月进行中赛事的队伍级锁（key = 赛事名，同月唯一）：并行日历下
//!   同一支队伍不能同时打两场；
//! - 按赛事日历排期并模拟本月全部赛事（[`Self::run_monthly_events`]）；
//! - 锁生命周期：模拟后锁定参赛队伍 → 时钟推进到结束日或跨月时解锁；
//! - 存档接口：[`Self::lock_records`] / [`Self::restore_locks`]。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use csc_decision::source::DecisionSource;
use csc_domain::tournament::Tournament;
use csc_entities::world::World;
use csc_time::clock::SimClock;
use csc_time::days_from_civil;
use csc_tournaments::calendar::SeasonCalendar;
use csc_tournaments::engine::TournamentEngine;
use csc_util::id::TeamId;
use csc_util::rng::Xoshiro256StarStar;

use crate::context::WorldContext;
use crate::state::LockRecord;

/// 本月进行中赛事的队伍级锁（key = 赛事名，同月唯一）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TournamentLock {
    /// 锁的结束日（当月日号；时钟推进到该日或跨月时解锁）
    pub end_day: i32,
    /// 锁的开始日（绝对日序表达；= days_from_civil(创建当天)，跨月语义正确）
    pub start_epoch_day: i64,
    /// 锁的结束日（绝对日序表达；= start_epoch + duration - 1，跨月语义正确）
    pub end_epoch_day: i64,
    /// 被锁队伍的稳定 ID 列表（存档值；运行期与 `Team.current_tournament_id` 双端）
    pub team_ids: Vec<TeamId>,
}

/// 赛季循环编排：锁状态可序列化（GameState 的一部分）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SeasonLoop {
    locks: HashMap<String, TournamentLock>,
}

impl SeasonLoop {
    /// 跨月清锁：上月赛事（最晚 27 号结束）全部结束，本月从零开始锁。
    /// **必须同时清 map 与队伍字段**（Kotlin `locks.clear()` 同语义——
    /// 只清字段会导致旧锁签名残留在 map，快照恢复时重复覆盖）。
    pub fn clear_locks(&mut self, world: &mut World) {
        for team in &mut world.teams {
            team.current_tournament_id = None;
        }
        self.locks.clear();
    }

    /// 按赛事日历排期并模拟本月全部赛事（并行生态位：同期多 Tier 共存，队伍级互斥）。
    ///
    /// **世界完整性契约（2026 修复）**：本方法模拟的是「整张日历」，与主角
    /// 是否参赛、是否身处 T1 无关——T1/Major 由 VRS 前 12 名 + 预选照常开打，
    /// NPC 对局走 quick 路径也产出逐人 KDA 并写入年度 Rating 累计器，
    /// TOP20 因此始终有满员的世界级样本池。玩家不在 T1，绝不等于 T1 停摆。
    ///
    /// @param month_idx 模拟月计数（0 起，赛事命名序号，见 `SeasonCalendar::events_of`）
    ///
    /// LIVE 闸门两阶段拆解（t12）：本方法 = `plan_month`（登记新月为 Future，
    /// 不结算）+ `settle_month`（立即结算）。月推进（Auto/月推进）走本方法快进；
    /// 日推进（`advance_day`）在月入口只 `plan_month`、月出口才 `settle_month`，
    /// 从而让比赛日当天 fixtures 保持 pending 可被 `live/today` 观测。
    pub fn run_monthly_events(
        &mut self,
        ctx: &mut WorldContext<'_>,
        calendar: &SeasonCalendar,
        decision: &mut dyn DecisionSource,
        month_idx: i32,
        rng: &mut Xoshiro256StarStar,
    ) {
        self.plan_month(ctx, calendar, month_idx, rng);
        self.settle_month(ctx, decision, rng);
    }

    /// LIVE 闸门阶段 1：登记当前时钟月的全部赛事为 `Future`（排程 + pending fixtures +
    /// 队伍锁），**不结算**。比赛日当天这些记录以 pending 存在，`live/today` 可观测。
    pub fn plan_month(
        &mut self,
        ctx: &mut WorldContext<'_>,
        calendar: &SeasonCalendar,
        month_idx: i32,
        rng: &mut Xoshiro256StarStar,
    ) {
        // 2026 复审修复：月首采集一次资格快照，本月全部赛事共用——
        // 月内 VRS 实时涨跌不再让 T1 队掉进后续 T2/T3 赛事。
        let monthly_ranking = TournamentEngine::ranking_snapshot(ctx.world, ctx.vrs);
        for event in calendar.events_of(ctx.clock) {
            self.advance_to(ctx.clock, ctx.world, event.start_day); // 推进时钟到开始日 + 解锁已结束赛事
            let tournament = (event.build)(month_idx, &ctx.clock.date_label()); // 在开始日创建：date 字段 = 开始日
            let planned = ctx.tournaments.plan_event_with_ranking(
                ctx.world,
                ctx.vrs,
                ctx.clock,
                ctx.journal,
                ctx.decision_log,
                &tournament,
                &monthly_ranking,
                rng,
            ); // 邀请阶段过滤已锁队伍（高 Tier 先锁）
            if planned {
                self.lock_event(ctx.world, ctx.tournaments, ctx.clock, &tournament); // 锁定参赛队伍（时间片内不重复参赛）
            }
            // R2.1 排程去重：同名 Future 记录已存在（如开局月物化后新月重排撞名）
            // 时 plan 返回 false——跳过锁（旧记录仍持有其锁），不覆盖、不重复。
        }
    }

    /// LIVE 闸门阶段 2：结算当前时钟月已登记（`Future`）的全部赛事（模拟 + 回填 + 荣誉）。
    /// 日推进在月出口调用；月推进由 [`Self::run_monthly_events`] 在 plan 后立即调用。
    pub fn settle_month(
        &mut self,
        ctx: &mut WorldContext<'_>,
        decision: &mut dyn DecisionSource,
        rng: &mut Xoshiro256StarStar,
    ) {
        let (y, m, _) = ctx.clock.now();
        let month_prefix = format!("{:04}-{:02}", y, m);
        ctx.tournaments.settle_pending_events(
            ctx.world,
            ctx.vrs,
            ctx.clock,
            ctx.journal,
            decision,
            ctx.decision_log,
            &month_prefix,
            rng,
        );
    }

    /// 当前锁的值记录（存档用：对象引用降维为稳定 ID 列表）。
    ///
    /// 确定性护栏（2026 结构拆分修复）：`locks` 是 HashMap，迭代序跨进程随机；
    /// 返回值进入 `GameState.locks`（数组，不被指纹 canonicalize 的键排序覆盖），
    /// 先按 `event_name` 排序可让存档字节序与指纹跨进程稳定。世界演化本身
    /// 不依赖该序（restore/删除均按 ID 反解），此排序零副作用。
    pub fn lock_records(&self) -> Vec<LockRecord> {
        let mut records: Vec<LockRecord> = self
            .locks
            .iter()
            .map(|(name, lock)| LockRecord {
                event_name: name.clone(),
                end_day: lock.end_day,
                start_epoch_day: lock.start_epoch_day,
                end_epoch_day: lock.end_epoch_day,
                team_ids: lock.team_ids.clone(),
            })
            .collect();
        records.sort_by(|a, b| a.event_name.cmp(&b.event_name));
        records
    }

    /// 从快照恢复锁（读档用）：按稳定 ID 直接反解并重设 `Team.current_tournament_id`。
    ///
    /// 反解失败（快照与实体仓库不一致）→ [`Err(SimError::Save)`]，不静默丢锁（丢锁会
    /// 让队伍重复参赛、生态错乱）。Engine::restore 在临时引擎上调用本方法——失败时
    /// 原引擎保持原状（P1-4 读档原子性，构造-交换）。
    pub fn try_restore_locks(
        &mut self,
        records: &[LockRecord],
        world: &mut World,
    ) -> Result<(), csc_util::SimError> {
        self.clear_locks(world); // 先解除旧锁队伍的占用标记
        for rec in records {
            for tid in &rec.team_ids {
                let Some(team) = world.team_mut(*tid) else {
                    return Err(csc_util::SimError::save(format!(
                        "GameState 恢复失败：锁「{}」的队伍 ID「{:?}」在实体仓库中不存在——快照与实体状态不一致，拒绝静默丢锁",
                        rec.event_name, tid
                    )));
                };
                team.current_tournament_id = Some(rec.event_name.clone());
            }
            self.locks.insert(
                rec.event_name.clone(),
                TournamentLock {
                    end_day: rec.end_day,
                    start_epoch_day: rec.start_epoch_day,
                    end_epoch_day: rec.end_epoch_day,
                    team_ids: rec.team_ids.clone(),
                },
            );
        }
        Ok(())
    }

    /// 从快照恢复锁（panic 版）：失败即 panic（内部测试/直接调用语义）。
    /// 外部读档一律走 [`Self::try_restore_locks`]（Engine::restore 构造-交换路径）。
    pub fn restore_locks(&mut self, records: &[LockRecord], world: &mut World) {
        self.try_restore_locks(records, world)
            .expect("restore_locks 失败：快照与实体状态不一致");
    }

    /// 推进模拟时钟到 `start_day`（只进不退），并解锁所有**已结束**赛事的队伍锁
    /// （结束日 <= 当前时钟，含结束日当天——赛事当天结束即可参加次日赛事）。
    fn advance_to(&mut self, clock: &mut SimClock, world: &mut World, start_day: i32) {
        let today = clock.now().2 as i32;
        if start_day > today {
            clock.advance_days((start_day - today) as i64);
        }
        let (y, m, d) = clock.now();
        let today_epoch = days_from_civil(y, m, d);
        let now_day = d as i32;
        let expired: Vec<String> = self
            .locks
            .iter()
            .filter(|(_, l)| {
                if l.end_epoch_day > 0 {
                    // 新逻辑：绝对日序解锁（跨月正确）
                    l.end_epoch_day <= today_epoch
                } else {
                    // 兼容旧存档：锁内没有绝对日序时退回月内日判断
                    l.end_day <= now_day
                }
            })
            .map(|(k, _)| k.clone())
            .collect();
        for name in expired {
            let lock = self.locks.remove(&name).expect("锁不存在");
            for tid in &lock.team_ids {
                if let Some(team) = world.team_mut(*tid) {
                    team.current_tournament_id = None;
                }
            }
        }
    }

    /// 赛事模拟结束后锁定全部参赛队伍（持续到该赛事结束日，由 `advance_to` 解锁）。
    fn lock_event(
        &mut self,
        world: &mut World,
        tournaments: &TournamentEngine,
        clock: &SimClock,
        event: &Tournament,
    ) {
        let team_ids = tournaments.last_event_teams();
        let (y, m, d) = clock.now();
        let start_epoch_day = days_from_civil(y, m, d);
        // R2.1 修整：`end_day` 与绝对日序 `end_epoch_day` 语义对齐——
        // 结束日 = 开始日 + duration - 1（含开始日当天；duration==1 的 T3 单日赛
        // start == end 合法：当天打完当天解锁）。旧实现 `d + duration` 把 T3 单日赛
        // 记为次日解锁，且与 `advance_to` 的 epoch 判断（<=）不一致。
        // 跨月赛事（如 1 月底开赛的顶级赛）`end_day` 可能溢出月内日号，
        // 但 `end_epoch_day > 0` 时 `advance_to` 恒走绝对日序判断（旧字段仅兼容用）。
        let end_epoch_day = start_epoch_day + event.duration_days.max(1) as i64 - 1;
        let end_day = d as i32 + event.duration_days.max(1) - 1;
        for tid in &team_ids {
            world
                .team_mut(*tid)
                .expect("队伍不存在")
                .current_tournament_id = Some(event.name.clone());
        }
        self.locks.insert(
            event.name.clone(),
            TournamentLock {
                end_day,
                start_epoch_day,
                end_epoch_day,
                team_ids,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;

    #[test]
    fn lock_roundtrip_via_records() {
        let mut world = World::new();
        let t = world.create_team("Vitality", 1, 2000);
        for i in 0..5 {
            world
                .create_npc(
                    Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("P{i}")),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                )
                .unwrap();
        }
        let mut loop_ = SeasonLoop::default();
        loop_.locks.insert(
            "EV".into(),
            TournamentLock {
                end_day: 6,
                start_epoch_day: 10,
                end_epoch_day: 17,
                team_ids: vec![t],
            },
        );
        world.team_mut(t).unwrap().current_tournament_id = Some("EV".into());

        // 存档 → 恢复
        let records = loop_.lock_records();
        let mut loop2 = SeasonLoop::default();
        loop2.restore_locks(&records, &mut world);
        assert_eq!(loop2.locks.len(), 1);
        assert_eq!(
            world.team(t).unwrap().current_tournament_id.as_deref(),
            Some("EV")
        );
    }

    #[test]
    fn restore_missing_team_id_panics() {
        let mut world = World::new();
        world.create_team("Vitality", 1, 2000);
        let mut loop_ = SeasonLoop::default();
        let records = vec![LockRecord {
            event_name: "EV".into(),
            end_day: 6,
            start_epoch_day: 10,
            end_epoch_day: 17,
            team_ids: vec![TeamId(999)], // 不存在的队伍 ID
        }];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loop_.restore_locks(&records, &mut world);
        }));
        assert!(result.is_err(), "ID 反解失败必须显式抛错");
    }

    #[test]
    fn clear_locks_releases_teams_and_map() {
        let mut world = World::new();
        let t = world.create_team("Vitality", 1, 2000);
        world.team_mut(t).unwrap().current_tournament_id = Some("EV".into());
        let mut loop_ = SeasonLoop::default();
        loop_.locks.insert(
            "EV".into(),
            TournamentLock {
                end_day: 6,
                start_epoch_day: 10,
                end_epoch_day: 17,
                team_ids: vec![],
            },
        );
        loop_.clear_locks(&mut world);
        assert_eq!(world.team(t).unwrap().current_tournament_id, None);
        assert!(
            loop_.locks.is_empty(),
            "map 必须一并清空（Kotlin locks.clear()）"
        );
    }

    #[test]
    fn advance_to_unlocks_cross_month_lock_by_epoch() {
        use csc_time::clock::SimClock;
        let mut world = World::new();
        let t = world.create_team("Vitality", 1, 2000);
        world.team_mut(t).unwrap().current_tournament_id = Some("EV".into());

        // 锁：2026-01-28 开始，持续 8 天 → end_epoch_day = 2026-02-04。
        let start_epoch = days_from_civil(2026, 1, 28);
        let end_epoch = start_epoch + 8 - 1;
        let mut loop_ = SeasonLoop::default();
        loop_.locks.insert(
            "EV".into(),
            TournamentLock {
                end_day: 28 + 8, // 旧字段：36，月内日无法表达跨月（而不是合法日期）
                start_epoch_day: start_epoch,
                end_epoch_day: end_epoch,
                team_ids: vec![t],
            },
        );

        let mut clock = SimClock::of(2026, 1, 28);
        loop_.advance_to(&mut clock, &mut world, 0);
        assert!(loop_.locks.contains_key("EV"), "1-28 锁应仍在");

        clock.advance_days(3); // → 2026-01-31，未到结束日
        loop_.advance_to(&mut clock, &mut world, 0);
        assert!(loop_.locks.contains_key("EV"), "1-31(跨月前) 锁应仍在");

        clock.advance_days(4); // → 2026-02-04，达到结束日
        loop_.advance_to(&mut clock, &mut world, 0);
        assert!(!loop_.locks.contains_key("EV"), "2-04 达到结束日，锁应解除");
        assert_eq!(world.team(t).unwrap().current_tournament_id, None);
    }

    /// C（锁 epoch 合法性）：`lock_event` 写入的锁必须满足
    /// `start_epoch_day <= end_epoch_day`，且与赛事开始/结束日一致
    /// （跨月赛事用绝对日序）。duration==1 的单日赛（T3）start==end 合法：
    /// 当天开赛当天解锁——这正是 t5 观察到的 `20511==20511`（2026-02-27 单日
    /// T3 赛），是合法记录而非异常；t5 以 duration>1 为前提误判。
    #[test]
    fn lock_event_epoch_is_legal_and_consistent_with_event_dates() {
        // 构造 8 支 T3 队伍（T3 固定 8 队契约，全空闲 → 参赛名单非空）。
        let mut world = World::new();
        let mut teams = Vec::new();
        for i in 0..8 {
            let t = world.create_team(format!("Team{i}"), 33 + i, 800 - i * 10);
            for j in 0..5 {
                world
                    .create_npc(
                        Tier::Tier4,
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
        let mut tournaments = csc_tournaments::engine::TournamentEngine::new();
        {
            // 用一次真实排程填充 last_scheduled（T3 排名段 33+ 的 8 队全进名单）。
            let ranking: Vec<(i32, i32, TeamId)> = teams
                .iter()
                .enumerate()
                .map(|(i, &t)| (33 + i as i32, 800 - i as i32 * 10, t))
                .collect();
            let mut rng = Xoshiro256StarStar::seed(42);
            let event = csc_domain::tournament::Tournament::new(
                "Exort Fiesta Series #1-8",
                csc_domain::tourney_tier::TourneyTier::Qualify,
                csc_domain::tournament::Organizer::Other,
                csc_domain::city::City::new("Online", "WW", csc_domain::city::Region::Other),
                1,
                csc_domain::tournament::InvitePolicy::Open,
            )
            .with_team_slots(8)
            .with_direct_invites(8)
            .with_open_qualifier_slots(0)
            .with_date("2026-02-27")
            .with_duration_days(1)
            .with_teams_per_event(Some(8))
            .with_pool_offset(0);
            let mut journal = csc_events::journal::WorldJournal::default();
            let mut log = csc_decision::log::DecisionLog::default();
            let planned = tournaments.plan_event_with_ranking(
                &mut world,
                &mut csc_vrs::engine::VrsEngine::from_database(
                    csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产"),
                ),
                &SimClock::of(2026, 2, 27),
                &mut journal,
                &mut log,
                &event,
                &ranking,
                &mut rng,
            );
            assert!(planned, "8 队 T3 排程应登记成功");
        }

        let mut loop_ = SeasonLoop::default();
        let clock = SimClock::of(2026, 2, 27);
        let event = csc_domain::tournament::Tournament::new(
            "Exort Fiesta Series #1-8",
            csc_domain::tourney_tier::TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            csc_domain::city::City::new("Online", "WW", csc_domain::city::Region::Other),
            1,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_duration_days(1)
        .with_date("2026-02-27");
        loop_.lock_event(&mut world, &tournaments, &clock, &event);

        let lock = loop_
            .locks
            .get("Exort Fiesta Series #1-8")
            .expect("锁已写入");
        assert!(
            lock.start_epoch_day <= lock.end_epoch_day,
            "锁 epoch 必须满足 start <= end（实际 {}..={}）",
            lock.start_epoch_day,
            lock.end_epoch_day
        );
        let start = days_from_civil(2026, 2, 27);
        assert_eq!(
            lock.start_epoch_day, start,
            "开始日 = 赛事开赛日（绝对日序）"
        );
        assert_eq!(
            lock.end_epoch_day, start,
            "duration==1 单日赛 start == end（当天开赛当天解锁，合法）"
        );
        assert_eq!(
            lock.end_day, 27,
            "end_day 与 epoch 对齐（d + duration - 1）"
        );
        // 被锁队伍与参赛名单一致。
        assert_eq!(lock.team_ids.len(), 8, "8 支参赛队全部锁定");
        for tid in &lock.team_ids {
            assert_eq!(
                world.team(*tid).unwrap().current_tournament_id.as_deref(),
                Some("Exort Fiesta Series #1-8")
            );
        }
        // 存档/恢复往返后 epoch 保持一致。
        let records = loop_.lock_records();
        let rec = records
            .iter()
            .find(|r| r.event_name == "Exort Fiesta Series #1-8")
            .expect("锁记录存在");
        assert!(rec.start_epoch_day <= rec.end_epoch_day);
    }

    /// C（锁 epoch 合法性 · 跨月）：跨月长赛事（duration>1 且跨月界）锁的
    /// start/end 必须用绝对日序正确表达，且 `advance_to` 按 epoch 解锁——
    /// `end_day` 月内日号溢出时不影响解锁判定。
    #[test]
    fn lock_event_epoch_cross_month_duration() {
        // 16 支 T1 队伍（12 直邀 + 4 预选 → 参赛名单恒 16）。
        let mut world = World::new();
        let mut teams = Vec::new();
        for i in 0..16 {
            let t = world.create_team(format!("Team{i}"), i + 1, 2000 - i * 50);
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
        let mut tournaments = csc_tournaments::engine::TournamentEngine::new();
        {
            let ranking: Vec<(i32, i32, TeamId)> = teams
                .iter()
                .enumerate()
                .map(|(i, &t)| (i as i32 + 1, 2000 - i as i32 * 50, t))
                .collect();
            let mut rng = Xoshiro256StarStar::seed(42);
            let event = csc_domain::tournament::Tournament::new(
                "IEM Kraków 2026",
                csc_domain::tourney_tier::TourneyTier::SuperElite,
                csc_domain::tournament::Organizer::Esl,
                csc_domain::city::City::new("Kraków", "Poland", csc_domain::city::Region::Europe),
                1,
                csc_domain::tournament::InvitePolicy::VrsGlobal,
            )
            .with_team_slots(16)
            .with_direct_invites(12)
            .with_open_qualifier_slots(4)
            .with_date("2026-01-16")
            .with_duration_days(10);
            let mut journal = csc_events::journal::WorldJournal::default();
            let mut log = csc_decision::log::DecisionLog::default();
            let planned = tournaments.plan_event_with_ranking(
                &mut world,
                &mut csc_vrs::engine::VrsEngine::from_database(
                    csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产"),
                ),
                &SimClock::of(2026, 1, 16),
                &mut journal,
                &mut log,
                &event,
                &ranking,
                &mut rng,
            );
            assert!(planned, "16 队 T1 排程应登记成功");
        }
        let mut loop_ = SeasonLoop::default();
        let clock = SimClock::of(2026, 1, 16);
        let event = csc_domain::tournament::Tournament::new(
            "IEM Kraków 2026",
            csc_domain::tourney_tier::TourneyTier::SuperElite,
            csc_domain::tournament::Organizer::Esl,
            csc_domain::city::City::new("Kraków", "Poland", csc_domain::city::Region::Europe),
            1,
            csc_domain::tournament::InvitePolicy::VrsGlobal,
        )
        .with_duration_days(10)
        .with_date("2026-01-16");
        loop_.lock_event(&mut world, &tournaments, &clock, &event);

        let lock = loop_.locks.get("IEM Kraków 2026").expect("锁已写入");
        let start = days_from_civil(2026, 1, 16);
        assert_eq!(lock.start_epoch_day, start);
        assert_eq!(
            lock.end_epoch_day,
            start + 10 - 1,
            "结束日 = 开始日 + duration - 1"
        );
        assert!(
            lock.start_epoch_day < lock.end_epoch_day,
            "跨月长赛事 start < end"
        );
        assert_eq!(lock.end_day, 25, "end_day 与 epoch 对齐（16 + 10 - 1）");
        // 结束日（01-25）解锁；01-24 仍锁。
        let mut clock2 = SimClock::of(2026, 1, 24);
        loop_.advance_to(&mut clock2, &mut world, 0);
        assert!(
            loop_.locks.contains_key("IEM Kraków 2026"),
            "01-24 锁应仍在"
        );
        clock2.advance_days(1); // → 01-25
        loop_.advance_to(&mut clock2, &mut world, 0);
        assert!(
            !loop_.locks.contains_key("IEM Kraków 2026"),
            "01-25 达到结束日，锁应解除（绝对日序）"
        );
        assert_eq!(world.team(teams[0]).unwrap().current_tournament_id, None);
    }
}
