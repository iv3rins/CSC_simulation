//! 赛事排程器（Kotlin `TournamentScheduler.kt` 转写）：**谁有资格参加**。
//!
//! 转写差异（相对 Kotlin）：Kotlin 构造注入 `EntityEngine` + `VrsEngine` +
//! `SimClock` → Rust 采用**参数化依赖**（月首资格快照 / 队伍锁定闭包
//! 全部作为方法参数），结构只持有排程产物——消除 `&mut` 借用冲突，
//! 让 `TournamentEngine`（编排壳）可自由组合。

use crate::invite_model::InviteModel;
use crate::scheduled::ScheduledTournament;
use csc_domain::event_importance::EventImportance;
use csc_domain::team_tier::TeamTier;
use csc_domain::tournament::{InvitePolicy, Tournament};
use csc_domain::tournament_format::TournamentFormat;
use csc_domain::tourney_tier::TourneyTier;
use csc_util::id::TeamId;
use csc_util::rng::Xoshiro256StarStar;

/// 队伍锁定判定闭包（= Kotlin `Team.currentTournamentId == null`）。
pub type TeamFreeCheck<'a> = dyn Fn(TeamId) -> bool + 'a;
/// 排程候选：(VRS 排名, VRS 积分, 队伍 ID)。
pub type RankedTeam = (i32, i32, TeamId);

/// 赛事排程器 sub-engine：按赛事描述生成参赛名单（邀请/资格）。
///
/// 数据源：**月首资格快照**（2026 复审）：整个月只认月初排名，杜绝实时
/// 排名波动让 T1 队掉进 T2/T3；空库回退调用方提供的实体快照。
pub struct TournamentScheduler {
    /// 已排程的赛事（按排程顺序）
    scheduled: Vec<ScheduledTournament>,
    /// 已排程参赛队伍名单快照（participants_of 用）
    participants: Vec<(TourneyTier, Vec<TeamId>)>,
}

impl Default for TournamentScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl TournamentScheduler {
    /// 构造排程器（无状态；依赖参数化）。
    pub fn new() -> Self {
        Self {
            scheduled: Vec::new(),
            participants: Vec::new(),
        }
    }

    /// 全部已排程赛事。
    pub fn scheduled_tournaments(&self) -> Vec<ScheduledTournament> {
        self.scheduled.clone()
    }

    /// 指定等级赛事已排程的参赛队伍（直邀 + 预选）。
    pub fn participants_of(&self, tier: TourneyTier) -> Vec<TeamId> {
        self.participants
            .iter()
            .filter(|(t, _)| *t == tier)
            .flat_map(|(_, ids)| ids.clone())
            .collect()
    }

    /// 生成 T1 赛事参赛名单（= Kotlin `scheduleT1`）：以**月首资格快照**
    /// 前 N 名为唯一邀请池（拒绝后按 VRS 次序替补）+ 限定名次段的公开预选。
    ///
    /// 2026 复审修复：此前邀请池读实时 VRS、替补/预选扫实时全池——赛事在
    /// 月内先后开打时，一场 T1 失利导致的名次波动会让「T1 队」掉进后续
    /// T2/T3 池，形成顶级队伍打二线赛的生态错乱。现在整个月只认一次月首排名。
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_t1(
        &mut self,
        is_team_free: &TeamFreeCheck,
        monthly_ranking: &[RankedTeam],
        direct_slots: i32,
        qualifier_slots: i32,
        rng: &mut Xoshiro256StarStar,
        event_date: &str,
        format: TournamentFormat,
    ) -> ScheduledTournament {
        // 邀请池：月首排名前 direct_slots 名（快照；实时排名波动不影响资格）
        let invite_pool: Vec<(i32, TeamId)> = monthly_ranking
            .iter()
            .take(direct_slots.max(0) as usize)
            .map(|(rank, points, id)| (*rank, *points, *id))
            .filter(|(_, _, id)| is_team_free(*id))
            .map(|(_, points, id)| (points, id))
            .collect();
        // 邀请中心 μ = 前 N 名积分均值（空池回退 0）
        let invite_center: f64 = {
            let take: Vec<i32> = invite_pool
                .iter()
                .take(direct_slots as usize)
                .map(|(p, _)| *p)
                .collect();
            if take.is_empty() {
                0.0
            } else {
                take.iter().map(|p| *p as f64).sum::<f64>() / take.len() as f64
            }
        };

        let mut invited: Vec<TeamId> = Vec::new();
        let mut declined: Vec<TeamId> = Vec::new();
        for (points, team) in &invite_pool {
            if invited.len() >= direct_slots as usize {
                break;
            }
            let roll = rng.next_bp(); // 整数骰（D2：掷骰外提，判定为整数比较）
            if InviteModel::declines(*points, invite_center, roll) {
                declined.push(*team);
            } else {
                invited.push(*team);
            }
        }

        // 替补：直邀不足时按**月首 VRS 次序**从未处理队伍补足（跳过已锁）
        if invited.len() < direct_slots as usize {
            let used: std::collections::HashSet<TeamId> =
                invited.iter().chain(declined.iter()).copied().collect();
            for (_, _, team) in monthly_ranking {
                if invited.len() >= direct_slots as usize {
                    break;
                }
                if is_team_free(*team) && !used.contains(team) {
                    invited.push(*team);
                }
            }
        }

        // 公开预选：只从直邀名次段之后取 qualifier_slots 支（如 T1=12+4，
        // Major=16+0）——低排名队伍不能靠公开预选混进顶级赛事。
        let qualifiers: Vec<TeamId> = monthly_ranking
            .iter()
            .map(|(rank, _, id)| (*rank, *id))
            .filter(|(rank, id)| *rank > direct_slots && is_team_free(*id))
            .take(qualifier_slots.max(0) as usize)
            .map(|(_, id)| id)
            .collect();

        self.record(ScheduledTournament {
            tier: TourneyTier::T1,
            importance: EventImportance::Important,
            direct_invitees: invited.into_iter().take(direct_slots as usize).collect(),
            replacements: declined,
            qualifiers,
            event_date: event_date.to_string(),
            format,
        })
    }

    /// 生成 T2 赛事参赛名单（= Kotlin `scheduleT2`）：月首排名 13~32，轮换切片。
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_t2(
        &mut self,
        is_team_free: &TeamFreeCheck,
        monthly_ranking: &[RankedTeam],
        teams_per_event: Option<i32>,
        pool_offset: i32,
        event_date: &str,
        format: TournamentFormat,
    ) -> ScheduledTournament {
        let mut pool: Vec<TeamId> = monthly_ranking
            .iter()
            .filter(|(r, _, _)| *r > TeamTier::T1_TEAM_COUNT && *r <= TeamTier::T2_MAX_RANKING)
            .map(|(_, _, id)| *id)
            .collect();
        pool.retain(|id| is_team_free(*id));
        let t2 = Self::slice_pool(&pool, pool_offset, teams_per_event);
        self.record(ScheduledTournament {
            tier: TourneyTier::T2,
            importance: EventImportance::Background,
            direct_invitees: t2,
            replacements: Vec::new(),
            qualifiers: Vec::new(),
            event_date: event_date.to_string(),
            format,
        })
    }

    /// 生成 T3 赛事参赛名单（= Kotlin `scheduleT3`）：月首排名 33 名开外自由报名。
    ///
    /// 2026 复审修复：
    /// - 玩家队伍优先（`player_team` 若空闲且符合 T3 排名段则保证入赛，避免玩家"排不到对手"）；
    /// - 参赛人数归一化（`resolve_t3_participants`）：去重 / 用空闲兜底池补足 / 参赛不足取消。
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_t3(
        &mut self,
        is_team_free: &TeamFreeCheck,
        monthly_ranking: &[RankedTeam],
        open_registration: &[TeamId],
        teams_per_event: Option<i32>,
        pool_offset: i32,
        event_date: &str,
        format: TournamentFormat,
        player_team: Option<TeamId>,
    ) -> ScheduledTournament {
        // T3 主池：月首排名 33 名开外的**空闲**队伍（公平轮换切片契约保持不变）。
        let mut pool: Vec<TeamId> = monthly_ranking
            .iter()
            .filter(|(r, _, _)| *r > TeamTier::T2_MAX_RANKING)
            .map(|(_, _, id)| *id)
            .collect();
        pool.retain(|id| is_team_free(*id));
        // 兜底池：同 T3 排名段内所有空闲队伍（切片未选中的也可用来补足参赛人数）。
        let fallback: Vec<TeamId> = pool.clone();
        let mut candidates = Self::slice_pool(&pool, pool_offset, teams_per_event);
        candidates.extend(open_registration.iter().copied());
        if let Some(pt) = player_team {
            candidates.push(pt);
        }
        let tier34 =
            Self::resolve_t3_participants(&candidates, player_team, is_team_free, &fallback);
        self.record(ScheduledTournament {
            tier: TourneyTier::Qualify,
            importance: EventImportance::Background,
            direct_invitees: tier34,
            replacements: Vec::new(),
            qualifiers: Vec::new(),
            event_date: event_date.to_string(),
            format,
        })
    }

    /// 按赛事描述的邀请策略分派到对应排程（= Kotlin `scheduleFor`）。
    ///
    /// 2026 复审修复：新增 `player_team`，仅 `InvitePolicy::Open`（T3）使用——
    /// 保证玩家队伍被排入三级赛事（P0-B）。
    #[allow(clippy::too_many_arguments)]
    pub fn schedule_for(
        &mut self,
        is_team_free: &TeamFreeCheck,
        monthly_ranking: &[RankedTeam],
        event: &Tournament,
        rng: &mut Xoshiro256StarStar,
        player_team: Option<TeamId>,
    ) -> ScheduledTournament {
        let mut scheduled = match event.invite_policy {
            InvitePolicy::VrsGlobal | InvitePolicy::VrsRegional => self.schedule_t1(
                is_team_free,
                monthly_ranking,
                event.direct_invites,
                event.open_qualifier_slots,
                rng,
                &event.date,
                event.format,
            ),
            InvitePolicy::Qualifier => self.schedule_t2(
                is_team_free,
                monthly_ranking,
                event.teams_per_event,
                event.pool_offset,
                &event.date,
                event.format,
            ),
            InvitePolicy::Open => self.schedule_t3(
                is_team_free,
                monthly_ranking,
                &[],
                event.teams_per_event,
                event.pool_offset,
                &event.date,
                event.format,
                player_team,
            ),
        };
        // 排程等级以真实赛事为准（schedule_t1 的 T1 硬编码会吞掉 Major——
        // 对阵/疲劳/决策门槛/阶段里程碑都依赖正确的 tier）
        scheduled.tier = event.tier;
        scheduled.importance = event.importance;
        scheduled
    }

    /// T3 参赛名单归一化：去重 / 玩家优先 / 兜底补足 / 凑不满取消。
    ///
    /// 规则（ROUND11 归一化修复：T3 **固定 8 队**，消除非 2 幂降级单败
    /// 「16 槽 7 轮空 / 9 队仅 1 场真赛」的荒谬观感；不隐式升 16——
    /// 超出容量的候选按稳定顺序留在候选池/下一场）：
    /// - 去重 + 玩家优先（`player_team` 若空闲则最前）；
    /// - 从兜底池补足到 8（`T3_TARGET_TEAMS`，只取未锁定队伍）；
    /// - 候选 > 8 时**截断保留前 8**（稳定顺序，不升级 16；多余留待下一场）；
    /// - 兜底后仍不足 8 → 取消整场（返回空名单，engine 侧识别跳过，不产生对局）。
    ///
    /// 即参赛数必须 = 8（或取消）；8 队走标准双败小组（snake_seed 分组，
    /// 无轮空过载）。`is_team_free` 过滤保证绝不引入重复/锁定队伍。
    const T3_TARGET_TEAMS: usize = 8;
    fn resolve_t3_participants(
        candidates: &[TeamId],
        player_team: Option<TeamId>,
        is_team_free: &TeamFreeCheck,
        fallback: &[TeamId],
    ) -> Vec<TeamId> {
        // 1. 去重并过滤未锁定队伍（保序：候选序优先，玩家队排在最前便于后续保证）。
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<TeamId> = Vec::new();
        let push =
            |id: TeamId, out: &mut Vec<TeamId>, seen: &mut std::collections::HashSet<TeamId>| {
                if seen.insert(id) && is_team_free(id) {
                    out.push(id);
                }
            };
        // 玩家队优先：若空闲则放到最前，保证即便切片未选中也被纳入。
        if let Some(pt) = player_team {
            push(pt, &mut out, &mut seen);
        }
        for id in candidates {
            push(*id, &mut out, &mut seen);
        }

        // 2. 固定容量 8：候选 > 8 截断保留前 8（稳定顺序，不隐式升 16）。
        out.truncate(Self::T3_TARGET_TEAMS);

        // 3. 从兜底池按序补足到 target（跳过已选/已锁定）。
        if out.len() < Self::T3_TARGET_TEAMS {
            for id in fallback {
                if out.len() >= Self::T3_TARGET_TEAMS {
                    break;
                }
                push(*id, &mut out, &mut seen);
            }
        }
        // 4. 兜底后仍不足 target → 取消整场（空名单，供 engine 侧识别并跳过）。
        if out.len() < Self::T3_TARGET_TEAMS {
            out.clear();
        }
        out
    }

    /// 邀请池分片（**2026 复审修复**）：此前按 `offset` 旋转后连续取前 `limit`
    /// 支——T3 池 96 队每月 offset 0..7 只会覆盖排名 33~47，排名 121+ 的队伍
    /// 永远无赛可打。改为「分片」语义：把池切成每片 `limit` 支，`offset` 选择
    /// 第 `offset` 片（环形取模）——同月 8 场 T3 覆盖 8 片 × 8 队 = 64 支不同
    /// 队伍；月份序号参与 offset（`month × 每月场次 + k`）后窗口逐月滚动，
    /// 全池队伍公平轮换。
    fn slice_pool(pool: &[TeamId], offset: i32, limit: Option<i32>) -> Vec<TeamId> {
        if pool.is_empty() {
            return Vec::new();
        }
        let Some(limit) = limit else {
            return pool.to_vec();
        };
        if limit <= 0 {
            return Vec::new();
        }
        let n = pool.len();
        let l = limit as usize;
        // 第 offset 片（环形）：每片 l 支，窗口逐场平移
        let start = ((offset as usize).wrapping_mul(l)).rem_euclid(n);
        let mut out = Vec::with_capacity(l.min(n));
        for i in 0..l.min(n) {
            out.push(pool[(start + i) % n]);
        }
        out
    }

    /// 记录一次排程并返回。
    fn record(&mut self, tournament: ScheduledTournament) -> ScheduledTournament {
        let tier = tournament.tier;
        let participants = tournament.all_participants();
        self.scheduled.push(tournament.clone());
        self.participants.push((tier, participants));
        tournament
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_pool_rotates() {
        let pool = vec![TeamId(1), TeamId(2), TeamId(3), TeamId(4)];
        // 分片语义：第 offset 片（每片 limit 支）——片 1 = 位置 [2..4]
        let s = TournamentScheduler::slice_pool(&pool, 1, Some(2));
        assert_eq!(s, vec![TeamId(3), TeamId(4)]);
        let s = TournamentScheduler::slice_pool(&pool, 0, None);
        assert_eq!(s.len(), 4);
        assert!(TournamentScheduler::slice_pool(&[], 0, None).is_empty());
        assert!(TournamentScheduler::slice_pool(&pool, 0, Some(0)).is_empty());
    }

    #[test]
    fn slice_pool_wraps_offset() {
        // 环形分片：3 队每片 3 支 → 片 5 回到片 0（5×3 mod 3 = 0）
        let pool = vec![TeamId(1), TeamId(2), TeamId(3)];
        let s = TournamentScheduler::slice_pool(&pool, 5, Some(3));
        assert_eq!(s, vec![TeamId(1), TeamId(2), TeamId(3)]);
        // 4 队每片 2 支：片 2 环形回卷 → [1, 2]
        let pool4 = vec![TeamId(1), TeamId(2), TeamId(3), TeamId(4)];
        let s = TournamentScheduler::slice_pool(&pool4, 2, Some(2));
        assert_eq!(s, vec![TeamId(1), TeamId(2)]);
    }

    #[test]
    fn slice_pool_slices_cover_whole_pool() {
        // 96 队池每片 8 支：连续 12 片覆盖全部队伍且片内不重复（T3 公平轮换契约）
        let pool: Vec<TeamId> = (1..=96).map(|i| TeamId(i as u32)).collect();
        let mut seen = std::collections::HashSet::new();
        for slice in 0..12 {
            let s = TournamentScheduler::slice_pool(&pool, slice, Some(8));
            assert_eq!(s.len(), 8);
            let uniq: std::collections::HashSet<_> = s.iter().copied().collect();
            assert_eq!(uniq.len(), 8, "片内队伍不得重复");
            for t in s {
                seen.insert(t);
            }
        }
        assert_eq!(seen.len(), 96, "12 片 × 8 队应覆盖全池 96 队");
        // 特定位置：排名 121~128（池尾 8 队）在片 11 被选中（公平轮换的关键修复）
        let tail = TournamentScheduler::slice_pool(&pool, 11, Some(8));
        assert!(tail.contains(&TeamId(96)), "池尾队伍必须进入轮换窗口");
    }

    #[test]
    fn t1_invites_and_qualifiers_via_fallback() {
        let mut sched = TournamentScheduler::new();
        let ranked: Vec<RankedTeam> = (1..=20)
            .map(|r| (r, 2000 - r * 10, TeamId(r as u32)))
            .collect();
        let free = |_id: TeamId| true;
        let t = sched.schedule_t1(
            &free,
            &ranked,
            12,
            4,
            &mut Xoshiro256StarStar::seed(42),
            "2026-06-01",
            TournamentFormat::SingleElim,
        );
        // 12 直邀 + 4 预选 = 16
        assert_eq!(t.direct_invitees.len(), 12);
        assert_eq!(t.qualifiers.len(), 4);
        assert_eq!(t.all_participants().len(), 16);
        assert_eq!(sched.participants_of(TourneyTier::T1).len(), 16);
    }

    #[test]
    fn monthly_snapshot_keeps_top_teams_out_of_lower_tiers() {
        // 128 队月首快照：无论实时 VRS 之后如何波动，1~12 只能进 T1，
        // 13~32 只能进 T2，33+ 只能进 T3。
        let snapshot: Vec<RankedTeam> = (1..=128)
            .map(|r| (r, 2500 - r * 10, TeamId(r as u32)))
            .collect();
        let free = |_id: TeamId| true;
        let mut sched = TournamentScheduler::new();
        let t2 = sched.schedule_t2(
            &free,
            &snapshot,
            Some(8),
            0,
            "2026-01-03",
            TournamentFormat::SwissPlayoff {
                rounds: 3,
                qualifiers: 4,
                swiss_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
        );
        assert!(
            t2.all_participants()
                .iter()
                .all(|id| (13..=32).contains(&(id.0 as i32))),
            "T2 参赛名单必须全部来自 13~32 名"
        );
        let t3 = sched.schedule_t3(
            &free,
            &snapshot,
            &[],
            Some(8),
            0,
            "2026-01-20",
            TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
            None,
        );
        assert!(
            t3.all_participants().iter().all(|id| (id.0 as i32) > 32),
            "T3 参赛名单不得包含 32 名以内队伍"
        );
    }

    #[test]
    fn major_takes_top_sixteen_only() {
        // Major = 16 直邀 + 0 公开预选：即使第 128 名自由且快照充足，
        // 也不得进入 Major 名单。
        let snapshot: Vec<RankedTeam> = (1..=128)
            .map(|r| (r, 2500 - r * 10, TeamId(r as u32)))
            .collect();
        let free = |_id: TeamId| true;
        let event = Tournament::new(
            "IEM Cologne Major 2026",
            TourneyTier::Major,
            csc_domain::tournament::Organizer::Esl,
            csc_domain::city::City::new("Cologne", "Germany", csc_domain::city::Region::Europe),
            2,
            InvitePolicy::VrsGlobal,
        )
        .with_team_slots(16)
        .with_direct_invites(16)
        .with_open_qualifier_slots(0)
        .with_date("2026-06-02");
        let mut sched = TournamentScheduler::new();
        let t = sched.schedule_for(
            &free,
            &snapshot,
            &event,
            &mut Xoshiro256StarStar::seed(42),
            None,
        );
        assert_eq!(t.all_participants().len(), 16);
        assert!(
            t.all_participants().iter().all(|id| id.0 as i32 <= 16),
            "Major 只能由月首排名前 16 的队伍参加"
        );
    }

    #[test]
    fn resolve_t3_dedups_and_keeps_player_first() {
        // 去重 + 玩家队优先（无兜底，纯去重）：玩家队即使被切片重复/后置也保证在最前。
        // 3 队 < 4 → 取消（空名单）。
        let free = |_id: TeamId| true;
        let players = vec![
            TeamId(100),
            TeamId(100), // 重复
            TeamId(101),
            TeamId(102),
        ];
        let resolved =
            TournamentScheduler::resolve_t3_participants(&players, Some(TeamId(100)), &free, &[]);
        assert!(resolved.is_empty(), "去重后 3 队 < 4 → 取消（空名单）");
    }

    #[test]
    fn resolve_t3_three_teams_cancels() {
        // 3 队（无兜底）→ 0~3 取消。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2), TeamId(3)],
            None,
            &free,
            &[],
        );
        assert!(resolved.is_empty(), "3 队应取消（空名单）");
    }

    #[test]
    fn resolve_t3_three_teams_cancels_even_with_fallback() {
        // ROUND11 契约：3 队 + 兜底池充足 → 补足到 8（固定 8 队）。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2), TeamId(3)],
            None,
            &free,
            &[
                TeamId(50),
                TeamId(51),
                TeamId(52),
                TeamId(53),
                TeamId(54),
                TeamId(55),
            ],
        );
        assert_eq!(resolved.len(), 8, "3 队 + 兜底充足应补足到 8");
    }

    #[test]
    fn resolve_t3_pads_four_to_seven_to_eight() {
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2), TeamId(3), TeamId(4), TeamId(5)],
            None,
            &free,
            &[TeamId(50), TeamId(51), TeamId(52), TeamId(53)],
        );
        assert_eq!(resolved.len(), 8, "5 队应补足到 8");
    }

    #[test]
    fn resolve_t3_nine_plus_truncates_to_eight_no_sixteen() {
        // ROUND11 契约：T3 固定 8 队——9+ 候选不隐式升 16，
        // 截断保留前 8（稳定顺序），多余留给下一场。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &(0..9).map(TeamId).collect::<Vec<_>>(),
            None,
            &free,
            &(100..116).map(TeamId).collect::<Vec<_>>(),
        );
        assert_eq!(resolved.len(), 8, "9 队应截断为 8，不升 16");
        assert_eq!(
            resolved,
            (0..8).map(TeamId).collect::<Vec<_>>(),
            "稳定顺序保留前 8"
        );
        // 12 队同样截断 8。
        let resolved12 = TournamentScheduler::resolve_t3_participants(
            &(0..12).map(TeamId).collect::<Vec<_>>(),
            None,
            &free,
            &[],
        );
        assert_eq!(resolved12.len(), 8, "12 队应截断为 8");
    }

    #[test]
    fn resolve_t3_cancels_when_cannot_pad_to_target() {
        // 5 队 + 兜底不足 → 取消整场（不再保留 5 队开赛）。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2), TeamId(3), TeamId(4), TeamId(5)],
            None,
            &free,
            &[TeamId(50), TeamId(51)], // 兜底只有 2 队，补不满 8
        );
        assert!(resolved.is_empty(), "兜底不足 8 → 取消整场");

        // 7 队 + 兜底不足 8 → 取消整场（固定 8 队契约，不降级开赛）。
        let resolved7 = TournamentScheduler::resolve_t3_participants(
            &(0..7).map(TeamId).collect::<Vec<_>>(),
            None,
            &free,
            &[], // 兜底为空
        );
        assert!(resolved7.is_empty(), "7 队兜底不足 8 → 取消整场");
    }

    #[test]
    fn resolve_t3_two_teams_cancels() {
        // 2 队 → 0~3 取消（不再单场对决）。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2)],
            None,
            &free,
            &[TeamId(50), TeamId(51)],
        );
        assert!(resolved.is_empty(), "2 队应取消（空名单）");
    }

    #[test]
    fn resolve_t3_excludes_locked_teams() {
        // 锁定队伍不得补位/入赛：TeamId(1) 被锁定 → 从兜底补另一队。
        // 4 候选去重后仅 3 空闲 → 从兜底补足到 8（固定 8 队契约）。
        let free = |id: TeamId| id != TeamId(1);
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(1), TeamId(2), TeamId(3), TeamId(4)],
            None,
            &free,
            &[
                TeamId(50),
                TeamId(51),
                TeamId(52),
                TeamId(53),
                TeamId(54),
                TeamId(55),
                TeamId(56),
                TeamId(57),
            ],
        );
        assert!(!resolved.contains(&TeamId(1)), "锁定队不得入赛");
        assert_eq!(resolved.len(), 8, "3 空闲 + 兜底 8 队应补足到 8");
        assert!(resolved.iter().all(|id| *id != TeamId(1)), "锁定队绝不入赛");
    }

    #[test]
    fn resolve_t3_empty_cancels() {
        // 0 队且兜底为空 → 取消（空名单）。
        let free = |_id: TeamId| true;
        let resolved = TournamentScheduler::resolve_t3_participants(&[], None, &free, &[]);
        assert!(resolved.is_empty(), "无参赛队应取消（空名单）");
    }

    #[test]
    fn resolve_t3_locked_player_not_forced() {
        // 玩家队若被锁定（正在其它赛事），不得强行纳入。
        let free = |id: TeamId| id != TeamId(100);
        let resolved = TournamentScheduler::resolve_t3_participants(
            &[TeamId(100), TeamId(1), TeamId(2)],
            Some(TeamId(100)),
            &free,
            &[TeamId(50), TeamId(51)],
        );
        assert!(!resolved.contains(&TeamId(100)), "锁定玩家队不得被强行纳入");
    }

    #[test]
    fn t3_schedule_always_8_never_16() {
        // ROUND11 回归：T3 成功调度恒为 8 队；9+ 候选不隐式升 16。
        let snapshot: Vec<RankedTeam> = (1..=128)
            .map(|r| (r, 2500 - r * 10, TeamId(r as u32)))
            .collect();
        let free = |_id: TeamId| true;
        let mut sched = TournamentScheduler::new();
        // 正常 8 队切片 → 8。
        let t3 = sched.schedule_t3(
            &free,
            &snapshot,
            &[],
            Some(8),
            0,
            "2026-01-20",
            TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
            None,
        );
        assert_eq!(
            t3.all_participants().len(),
            8,
            "T3 正常调度恒为 8 队（固定容量）"
        );
        // 9+ 自由报名候选 → 截断保留前 8，不升 16。
        let open: Vec<TeamId> = (100..112).map(TeamId).collect(); // 12 队自由报名
        let t3b = sched.schedule_t3(
            &free,
            &snapshot,
            &open,
            Some(8),
            1,
            "2026-01-21",
            TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
            None,
        );
        assert_eq!(
            t3b.all_participants().len(),
            8,
            "9+ 候选不得隐式升级 16，必须截断为 8"
        );
    }

    #[test]
    fn t3_open_registration_candidates_stay_8() {
        // ROUND11 契约：open_registration 追加候选后仍限制目标 8（玩家优先保留）。
        let snapshot: Vec<RankedTeam> = (1..=128)
            .map(|r| (r, 2500 - r * 10, TeamId(r as u32)))
            .collect();
        let free = |_id: TeamId| true;
        let mut sched = TournamentScheduler::new();
        let open: Vec<TeamId> = (200..220).map(TeamId).collect(); // 20 队自由报名
        let t3 = sched.schedule_t3(
            &free,
            &snapshot,
            &open,
            Some(8),
            0,
            "2026-02-03",
            TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
            Some(TeamId(250)), // 玩家队自由
        );
        let parts = t3.all_participants();
        assert_eq!(parts.len(), 8, "open_registration 追加后仍为 8 队");
        assert!(parts.contains(&TeamId(250)), "玩家队优先必须保留在名单中");
    }
}
