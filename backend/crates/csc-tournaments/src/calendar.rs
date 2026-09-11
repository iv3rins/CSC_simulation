//! 赛季赛事日历（Kotlin `SeasonCalendar.kt` 转写）：**什么时候办什么赛事**。
//!
//! 2026 修订：赛事密度对齐 HLTV 真实赛历——
//! - **T1 桶（MAJOR / SUPERELITE / ELITE）**：2026 全年约 25 场，直接按
//!   HLTV 2026 Top Tier Calendar 的真实月份与赛事品牌排期（2 Major +
//!   18 场 S 级 + 5 场 A 级），不再是「每自然月一场」。
//! - **T2**：每月 5 场（约 60 场/年，HLTV B/C 级池的模拟缩影）。
//! - **T3（QUALIFY）**：每月 8 场（约 96 场/年；真实 HLTV 低级别与资格赛
//!   密度更高，此处按 128 支模拟队伍的规模做了 1:2~1:3 缩放，保证长生涯
//!   存档体积与运行时间可控）。
//!
//! 同期互斥是**队伍级**而非日历级（`Team.current_tournament_id` 锁）。

use csc_domain::city::{City, Region};
use csc_domain::event_importance::EventImportance;
use csc_domain::match_result::MatchVenue;
use csc_domain::tournament::{InvitePolicy, Organizer, Tournament};
use csc_domain::tournament_format::TournamentFormat;
use csc_domain::tourney_tier::TourneyTier;
use csc_time::clock::SimClock;

/// 一次排期：开始日 + 等级（排序仲裁用）+ 赛事构建器。
///
/// 转写差异：Kotlin 闭包捕获 clock 引用 → Rust 闭包只捕获**构建参数**（year 值，
/// 无引用），`date` 由 `events_of` 在调用时注入（`Fn(i32, &str) -> Tournament`）。
/// 赛事构建器：(monthIdx, dateLabel) -> Tournament
pub type EventBuilder = Box<dyn Fn(i32, &str) -> Tournament>;

/// 一次排期：开始日 + 等级（排序仲裁用）+ 赛事构建器。
pub struct ScheduledEvent {
    pub start_day: i32,
    pub tier: TourneyTier,
    pub build: EventBuilder,
}

impl std::fmt::Debug for ScheduledEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduledEvent")
            .field("start_day", &self.start_day)
            .field("tier", &self.tier)
            .finish()
    }
}

/// 一场顶级赛事的静态描述（真实品牌/城市/承办方/规模）。
#[derive(Debug, Clone)]
struct TopSpec {
    start_day: i32,
    tier: TourneyTier,
    name: String,
    nickname: String,
    city: City,
    organizer: Organizer,
    team_slots: i32,
    duration_days: i32,
}

/// 赛季赛事日历。
pub struct SeasonCalendar;

impl SeasonCalendar {
    /// 本月全部赛事排程：T1 桶（按真实赛历 0~3 场）+ T2×5 + T3×8。
    /// 按开始日升序，同日按等级降序（高 Tier 先模拟 → 先锁队伍 → 低 Tier 捡漏）。
    pub fn events_of(&self, clock: &SimClock) -> Vec<ScheduledEvent> {
        let month_of_year = clock.now().1 as i32;
        let year = clock.year();
        let mut events: Vec<ScheduledEvent> = Vec::new();

        for spec in Self::top_tier_specs(year, month_of_year) {
            events.push(ScheduledEvent {
                start_day: spec.start_day,
                tier: spec.tier,
                build: Box::new(move |_m, date| Self::top_event(&spec, date)),
            });
        }
        for k in 0..Self::T2_EVENTS_PER_MONTH {
            events.push(ScheduledEvent {
                start_day: Self::T2_START_DAY + k * Self::T2_EVENT_GAP,
                tier: TourneyTier::T2,
                // 全局排程序号（2026 复审修复）：月计数 × 每月场次 + 场内序号，
                // 作为邀请池**分片**索引——窗口逐月滚动，13~32 名全池公平轮换
                // （旧实现 offset 0..4 连续取 8 队，21~32 名永不参赛）。
                build: Box::new(move |m, date| {
                    Self::t2_event(m, m * Self::T2_EVENTS_PER_MONTH + k, date)
                }),
            });
        }
        for k in 0..Self::T3_EVENTS_PER_MONTH {
            events.push(ScheduledEvent {
                start_day: Self::T3_START_DAY + k * Self::T3_EVENT_GAP,
                tier: TourneyTier::Qualify,
                // 同上：全局序号 → 分片索引；33~128 名全池每月都有队伍进入轮换
                // （旧实现只覆盖 33~47 名，T4 队伍无赛可打）。
                build: Box::new(move |m, date| {
                    Self::t3_event(m, m * Self::T3_EVENTS_PER_MONTH + k, date)
                }),
            });
        }
        // 开始日升序；同日按等级降序（MAJOR 最先）
        events.sort_by(|a, b| {
            a.start_day
                .cmp(&b.start_day)
                .then_with(|| tier_ordinal(b.tier).cmp(&tier_ordinal(a.tier)))
        });
        events
    }

    /// 2026 真实顶级赛历（HLTV Top Tier Calendar + 各主办方官宣）。
    /// 返回值按开始日升序；12 月没有新开顶级赛事（新加坡 Major 11 月末开始）。
    fn top_tier_specs(year: i32, month: i32) -> Vec<TopSpec> {
        let spec = |start_day: i32,
                    tier: TourneyTier,
                    name: String,
                    nickname: &str,
                    city: (&str, &str, Region),
                    organizer: Organizer,
                    team_slots: i32,
                    duration_days: i32| TopSpec {
            start_day,
            tier,
            name,
            nickname: nickname.to_string(),
            city: City::new(city.0, city.1, city.2),
            organizer,
            team_slots,
            duration_days,
        };

        match month {
            1 => vec![
                spec(
                    7,
                    TourneyTier::SuperElite,
                    format!("BLAST Bounty {year}"),
                    "BLAST Bounty",
                    ("Attard", "Malta", Region::Europe),
                    Organizer::Blast,
                    16,
                    8,
                ),
                spec(
                    16,
                    TourneyTier::SuperElite,
                    format!("IEM Kraków {year}"),
                    "IEM Kraków",
                    ("Kraków", "Poland", Region::Europe),
                    Organizer::Esl,
                    16,
                    10,
                ),
            ],
            2 => vec![
                spec(
                    6,
                    TourneyTier::SuperElite,
                    format!("PGL Cluj-Napoca {year}"),
                    "PGL Cluj-Napoca",
                    ("Cluj-Napoca", "Romania", Region::Europe),
                    Organizer::Pgl,
                    16,
                    9,
                ),
                spec(
                    18,
                    TourneyTier::SuperElite,
                    format!("ESL Pro League S{}", year - 2000),
                    "EPL",
                    ("Stockholm", "Sweden", Region::Europe),
                    Organizer::Esl,
                    16,
                    10,
                ),
            ],
            3 => vec![spec(
                8,
                TourneyTier::SuperElite,
                format!("BLAST Open Rotterdam {year}"),
                "BLAST Open",
                ("Rotterdam", "Netherlands", Region::Europe),
                Organizer::Blast,
                16,
                9,
            )],
            4 => vec![
                spec(
                    3,
                    TourneyTier::SuperElite,
                    format!("PGL Bucharest {year}"),
                    "PGL Bucharest",
                    ("Bucharest", "Romania", Region::Europe),
                    Organizer::Pgl,
                    16,
                    8,
                ),
                spec(
                    13,
                    TourneyTier::SuperElite,
                    format!("IEM Rio {year}"),
                    "IEM Rio",
                    ("Rio de Janeiro", "Brazil", Region::SouthAmerica),
                    Organizer::Esl,
                    16,
                    6,
                ),
                spec(
                    20,
                    TourneyTier::Elite,
                    format!("FISSURE Playground 3 {year}"),
                    "FISSURE Playground",
                    ("Shenzhen", "China", Region::Asia),
                    Organizer::Other,
                    16,
                    5,
                ),
            ],
            5 => vec![
                spec(
                    7,
                    TourneyTier::SuperElite,
                    format!("PGL Astana {year}"),
                    "PGL Astana",
                    ("Astana", "Kazakhstan", Region::Asia),
                    Organizer::Pgl,
                    16,
                    8,
                ),
                spec(
                    12,
                    TourneyTier::SuperElite,
                    format!("IEM Atlanta {year}"),
                    "IEM Atlanta",
                    ("Atlanta", "United States", Region::NorthAmerica),
                    Organizer::Esl,
                    16,
                    6,
                ),
                spec(
                    19,
                    TourneyTier::SuperElite,
                    format!("CS Asia Championships {year}"),
                    "CAC",
                    ("Shanghai", "China", Region::Asia),
                    Organizer::Other,
                    16,
                    5,
                ),
            ],
            6 => vec![spec(
                2,
                TourneyTier::Major,
                format!("IEM Cologne Major {year}"),
                "Cologne Major",
                ("Cologne", "Germany", Region::Europe),
                Organizer::Esl,
                16,
                11,
            )],
            7 => vec![
                spec(
                    6,
                    TourneyTier::Elite,
                    format!("XSE Pro League {year}"),
                    "XSE Pro League",
                    ("Guangzhou", "China", Region::Asia),
                    Organizer::Other,
                    16,
                    8,
                ),
                spec(
                    13,
                    TourneyTier::Elite,
                    format!("FISSURE Playground 4 {year}"),
                    "FISSURE Playground",
                    ("Suzhou", "China", Region::Asia),
                    Organizer::Other,
                    16,
                    5,
                ),
                spec(
                    20,
                    TourneyTier::SuperElite,
                    format!("BLAST Bounty {year} Season 2"),
                    "BLAST Bounty S2",
                    ("Attard", "Malta", Region::Europe),
                    Organizer::Blast,
                    16,
                    8,
                ),
            ],
            8 => vec![
                spec(
                    12,
                    TourneyTier::SuperElite,
                    format!("Esports World Cup {year}"),
                    "Esports World Cup",
                    ("Riyadh", "Saudi Arabia", Region::Asia),
                    Organizer::Other,
                    16,
                    8,
                ),
                spec(
                    24,
                    TourneyTier::SuperElite,
                    format!("BLAST Open Porto {year}"),
                    "BLAST Open",
                    ("Porto", "Portugal", Region::Europe),
                    Organizer::Blast,
                    16,
                    8,
                ),
            ],
            9 => vec![
                spec(
                    7,
                    TourneyTier::Elite,
                    format!("FISSURE Playground 5 {year}"),
                    "FISSURE Playground",
                    ("Suzhou", "China", Region::Asia),
                    Organizer::Other,
                    16,
                    5,
                ),
                spec(
                    17,
                    TourneyTier::Elite,
                    format!("StarLadder StarSeries Fall {year}"),
                    "StarSeries",
                    ("Belgrade", "Serbia", Region::Europe),
                    Organizer::Other,
                    16,
                    5,
                ),
            ],
            10 => vec![
                spec(
                    3,
                    TourneyTier::SuperElite,
                    format!("ESL Pro League S{}", year - 2000),
                    "EPL",
                    ("Katowice", "Poland", Region::Europe),
                    Organizer::Esl,
                    16,
                    8,
                ),
                spec(
                    14,
                    TourneyTier::Elite,
                    format!("Thunderpick World Championship {year}"),
                    "Thunderpick",
                    ("Malta", "Malta", Region::Europe),
                    Organizer::Other,
                    16,
                    5,
                ),
                spec(
                    24,
                    TourneyTier::SuperElite,
                    format!("PGL Masters Bucharest {year}"),
                    "PGL Masters Bucharest",
                    ("Bucharest", "Romania", Region::Europe),
                    Organizer::Pgl,
                    16,
                    7,
                ),
            ],
            11 => vec![
                spec(
                    2,
                    TourneyTier::SuperElite,
                    format!("IEM China {year}"),
                    "IEM China",
                    ("Chengdu", "China", Region::Asia),
                    Organizer::Esl,
                    16,
                    6,
                ),
                spec(
                    9,
                    TourneyTier::SuperElite,
                    format!("BLAST Rivals {year} Season 2"),
                    "BLAST Rivals",
                    ("Hong Kong", "China", Region::Asia),
                    Organizer::Blast,
                    16,
                    6,
                ),
                spec(
                    17,
                    TourneyTier::Major,
                    format!("PGL Major Singapore {year}"),
                    "Singapore Major",
                    ("Singapore", "Singapore", Region::Asia),
                    Organizer::Valve,
                    16,
                    13,
                ),
            ],
            // 12 月：新加坡 Major 跨月收官，不新开顶级赛事。
            _ => vec![],
        }
    }

    /// 顶级赛事构建（T1 桶共用）：线下 LAN + VRS 全球直邀 + 瑞士轮淘汰赛。
    fn top_event(spec: &TopSpec, date: &str) -> Tournament {
        let mut event = Tournament::new(
            spec.name.clone(),
            spec.tier,
            spec.organizer,
            spec.city.clone(),
            if spec.tier == TourneyTier::Major {
                2
            } else {
                1
            },
            InvitePolicy::VrsGlobal,
        )
        .with_nickname(spec.nickname.clone())
        .with_team_slots(spec.team_slots)
        // 2026 复审修复：Major = 16 直邀 + 0 公开预选（只认月首 VRS 前 16），
        // 低排名队伍不得再经公开预选补进 Major；普通 T1 桶仍为 12+4。
        .with_direct_invites(if spec.tier == TourneyTier::Major {
            spec.team_slots
        } else {
            (spec.team_slots - 4).max(0)
        })
        .with_open_qualifier_slots(if spec.tier == TourneyTier::Major {
            0
        } else {
            4
        })
        .with_venue(MatchVenue::Lan)
        .with_duration_days(spec.duration_days)
        .with_date(date);

        event.importance = if spec.tier == TourneyTier::Major {
            EventImportance::Major
        } else {
            EventImportance::Important
        };
        event.format = if spec.tier == TourneyTier::Major {
            // HLTV 2026 Major：瑞士轮小组赛 BO3 → 单败淘汰 BO3 → 决赛 BO5
            TournamentFormat::SwissPlayoff {
                rounds: 3,
                qualifiers: 8,
                swiss_best_of: 3,
                playoff_best_of: 3,
                final_best_of: 5,
            }
        } else {
            TournamentFormat::swiss_playoff()
        };
        event
    }

    // —— 赛事模板（新建的 Tournament 对象，供日历构建）——

    /// T2 赛事：线上快速赛，每月 5 场，从 T2 池按 pool_offset 轮换。
    pub fn t2_event(month: i32, pool_offset: i32, date: &str) -> Tournament {
        let label = Self::T2_LABELS
            [((month + pool_offset).rem_euclid(Self::T2_LABELS.len() as i32)) as usize];
        // R2.1 修复：名内序号按月内场次归一化（slot ∈ 1..=每月场次）。
        // 开局月（month=-1）pool_offset 为负（-5..-1），旧实现直出 `#0--4` 类
        // 畸形名；归一化后开局月名 = `#0-1..5`，与 2 月（m=0）`#1-1..5` 唯一区分。
        // label 轮换（(month+pool_offset) mod len）与 `pool_offset` 分片值保持原样——
        // 只修显示编号，不动邀请池轮换、分片选择与 RNG 域。
        let slot = pool_offset.rem_euclid(Self::T2_EVENTS_PER_MONTH) + 1;
        Tournament::new(
            format!("{label} #{}-{slot}", month + 1),
            TourneyTier::T2,
            Organizer::Cct,
            City::new("Online", "EU", Region::Europe),
            1,
            InvitePolicy::Qualifier,
        )
        .with_nickname(label)
        .with_team_slots(Self::T2_TEAMS_PER_EVENT)
        .with_direct_invites(Self::T2_TEAMS_PER_EVENT)
        .with_open_qualifier_slots(0)
        .with_venue(MatchVenue::Online)
        .with_duration_days(Self::T2_DURATION_DAYS)
        .with_date(date)
        // 树状淘汰赛：瑞士轮 BO1 → 半决赛/淘汰赛 BO3 → 决赛 BO5
        .with_format(TournamentFormat::SwissPlayoff {
            rounds: 3,
            qualifiers: 4,
            swiss_best_of: 1,
            playoff_best_of: 3,
            final_best_of: 5,
        })
        .with_teams_per_event(Some(Self::T2_TEAMS_PER_EVENT))
        .with_pool_offset(pool_offset)
    }

    /// T3 赛事：全年无休线上快速赛，统一双败（2 组 × 4 队全 BO1）。
    pub fn t3_event(month: i32, pool_offset: i32, date: &str) -> Tournament {
        let label = Self::T3_LABELS
            [((month + pool_offset).rem_euclid(Self::T3_LABELS.len() as i32)) as usize];
        // R2.1 修复：名内序号按月内场次归一化（slot ∈ 1..=8）。
        // 开局月（month=-1）pool_offset 为负（-8..-1），旧实现直出 `#0--7` 类
        // 畸形名，且与 2 月（m=0）`#1-8` label 交错并存（t5 证据）。归一化后
        // 开局月名 = `#0-1..8`，与 2 月 `#1-1..8` 唯一区分。
        // label 轮换与 `pool_offset` 分片值保持原样（只修显示编号）。
        let slot = pool_offset.rem_euclid(Self::T3_EVENTS_PER_MONTH) + 1;
        Tournament::new(
            format!("{label} #{}-{slot}", month + 1),
            TourneyTier::Qualify,
            Organizer::Other,
            City::new("Online", "WW", Region::Other),
            1,
            InvitePolicy::Open,
        )
        .with_nickname(label)
        .with_team_slots(Self::T3_TEAMS_PER_EVENT)
        .with_direct_invites(Self::T3_TEAMS_PER_EVENT)
        .with_open_qualifier_slots(0)
        .with_venue(MatchVenue::LanCafe)
        .with_duration_days(Self::T3_DURATION_DAYS)
        .with_date(date)
        .with_format(TournamentFormat::DoubleElimGroups {
            groups: 2,
            group_best_of: 1,
            playoff_best_of: 3,
            final_best_of: 5,
        })
        .with_teams_per_event(Some(Self::T3_TEAMS_PER_EVENT))
        .with_pool_offset(pool_offset)
    }

    // —— 常量 ——

    /// Major 持续天数（HLTV 2026：Cologne Major 6/11–6/21 ≈ 11 天）。
    pub const MAJOR_DURATION_DAYS: i32 = 11;
    /// T2：每月 5 场线上赛，首场 3 号起（真实 B/C 级池的缩放密度）。
    pub const T2_EVENTS_PER_MONTH: i32 = 5;
    pub const T2_TEAMS_PER_EVENT: i32 = 8;
    pub const T2_START_DAY: i32 = 3;
    pub const T2_DURATION_DAYS: i32 = 2;
    pub const T2_EVENT_GAP: i32 = 4;
    /// T3：全年无休线上快速赛，每月 8 场（20–27 日滚动开赛）。
    pub const T3_EVENTS_PER_MONTH: i32 = 8;
    pub const T3_TEAMS_PER_EVENT: i32 = 8;
    pub const T3_START_DAY: i32 = 20;
    pub const T3_DURATION_DAYS: i32 = 1;
    pub const T3_EVENT_GAP: i32 = 1;
    /// T2 赛事昵称轮换池（2026 依据 HLTV events 页扩充：ESL Challenger/CCT/
    /// FISSURE/StarLadder/Stake/1win/Logitech/iBUYPOWER）。
    pub const T2_LABELS: [&str; 10] = [
        "CCT Global Finals",
        "ESL Challenger League",
        "CCT Europe Series",
        "RES Regional Series",
        "FISSURE Playground",
        "StarLadder StarSeries",
        "Stake Pulse Beat",
        "1win Private Club",
        "Logitech G Play Connect",
        "iBUYPOWER Masters",
    ];
    /// T3 赛事昵称轮换池（2026 依据 HLTV events 页扩充）。
    pub const T3_LABELS: [&str; 8] = [
        "CCT Open Cup",
        "Regional Cup",
        "Online Masters",
        "Rising Cup",
        "CCT Contenders",
        "NODWIN Clutch Series",
        "UKIC Masters",
        "Exort Fiesta Series",
    ];
}

/// 等级排序权重（同日按等级降序：MAJOR 最先）。
fn tier_ordinal(tier: TourneyTier) -> i32 {
    match tier {
        TourneyTier::Major => 0,
        TourneyTier::SuperElite => 1,
        TourneyTier::Elite => 2,
        TourneyTier::T1 => 3,
        TourneyTier::T2 => 4,
        TourneyTier::Qualify => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_2026_top_tier_density() {
        // HLTV 2026 顶级赛历：2 Major + 18 S + 5 A = 25 场。
        let total: usize = (1..=12)
            .map(|m| SeasonCalendar::top_tier_specs(2026, m).len())
            .sum();
        assert_eq!(total, 25);
        let specs = SeasonCalendar::top_tier_specs(2026, 6);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].tier, TourneyTier::Major);
        assert_eq!(specs[0].name, "IEM Cologne Major 2026");
        let nov = SeasonCalendar::top_tier_specs(2026, 11);
        assert!(nov.iter().any(|s| s.name == "PGL Major Singapore 2026"));
    }

    #[test]
    fn lower_tier_density_and_unique_days() {
        let cal = SeasonCalendar;
        let clock = SimClock::of(2026, 1, 1);
        let events = cal.events_of(&clock);
        assert_eq!(events.len(), 2 + 5 + 8, "1 月 = 2 场 T1 桶 + T2×5 + T3×8");
        assert!(
            !events.iter().any(|e| e.tier == TourneyTier::Major),
            "1 月无 Major"
        );
        let t3_days: Vec<i32> = events
            .iter()
            .filter(|e| e.tier == TourneyTier::Qualify)
            .map(|e| e.start_day)
            .collect();
        assert_eq!(t3_days, vec![20, 21, 22, 23, 24, 25, 26, 27]);
        // 开始日升序
        for w in events.windows(2) {
            assert!(w[0].start_day <= w[1].start_day);
        }
    }

    #[test]
    fn june_has_cologne_major_plus_t2_t3() {
        let cal = SeasonCalendar;
        let clock = SimClock::of(2026, 6, 1);
        let events = cal.events_of(&clock);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.tier == TourneyTier::Major)
                .count(),
            1,
            "6 月有科隆 Major"
        );
        assert_eq!(events.len(), 1 + 5 + 8, "Major + T2×5 + T3×8");
        let major = (events[0].build)(5, "2026-06-02");
        assert_eq!(major.name, "IEM Cologne Major 2026");
        assert_eq!(major.nickname, "Cologne Major");
        assert_eq!(major.tier, TourneyTier::Major);
        assert_eq!(major.importance, EventImportance::Major);
        assert!(major.importance.is_live());
        assert_eq!(major.duration_days, 11);
        // 2026 复审修复：Major 只认 VRS 前 16，不再开公开预选让低排名混入。
        assert_eq!(major.direct_invites, 16);
        assert_eq!(major.open_qualifier_slots, 0);
    }

    #[test]
    fn november_has_singapore_major() {
        let cal = SeasonCalendar;
        let clock = SimClock::of(2026, 11, 1);
        let events = cal.events_of(&clock);
        assert!(
            events.iter().any(|e| e.tier == TourneyTier::Major),
            "11 月有新加坡 Major"
        );
        let spec = SeasonCalendar::top_tier_specs(2026, 11)
            .into_iter()
            .find(|s| s.tier == TourneyTier::Major)
            .expect("11 月赛历含 Major");
        let major = SeasonCalendar::top_event(&spec, "2026-11-17");
        assert_eq!(major.name, "PGL Major Singapore 2026");
        assert_eq!(major.nickname, "Singapore Major");
        assert_eq!(major.importance, EventImportance::Major);
    }

    #[test]
    fn lower_tier_events_are_background() {
        let t2 = SeasonCalendar::t2_event(0, 0, "2026-01-03");
        let t3 = SeasonCalendar::t3_event(0, 0, "2026-01-20");
        assert_eq!(t2.importance, EventImportance::Background);
        assert_eq!(t3.importance, EventImportance::Background);
        assert!(!t2.importance.is_live());
        assert!(!t3.importance.is_live());
    }

    #[test]
    fn t1_template_uses_first_real_fixture() {
        let spec = SeasonCalendar::top_tier_specs(2026, 1)
            .into_iter()
            .next()
            .expect("1 月有顶级赛事");
        let t = SeasonCalendar::top_event(&spec, "2026-01-07");
        assert_eq!(t.name, "BLAST Bounty 2026");
        assert_eq!(t.nickname, "BLAST Bounty");
        assert_eq!(t.team_slots, 16);
        assert_eq!(t.format, TournamentFormat::swiss_playoff());
    }

    #[test]
    fn t1_identities_match_hlvt_2026() {
        let first_nick = |month: i32| {
            SeasonCalendar::top_tier_specs(2026, month)
                .first()
                .map(|s| s.nickname.clone())
                .unwrap_or_default()
        };
        assert_eq!(first_nick(1), "BLAST Bounty");
        assert_eq!(first_nick(2), "PGL Cluj-Napoca");
        assert_eq!(first_nick(4), "PGL Bucharest");
        assert_eq!(first_nick(11), "IEM China");
    }

    #[test]
    fn t2_t3_labels_do_not_collide_within_month() {
        // 同月 5 场 T2 / 8 场 T3 昵称互不相同（轮换池长度 ≥ 场次）
        for month in 0..24 {
            let mut seen = std::collections::HashSet::new();
            for k in 0..SeasonCalendar::T2_EVENTS_PER_MONTH {
                let t = SeasonCalendar::t2_event(month, k, "2026-01-03");
                assert!(
                    seen.insert(t.nickname.clone()),
                    "T2 月 {month} 场 {k} 昵称重复: {}",
                    t.nickname
                );
            }
            let mut seen3 = std::collections::HashSet::new();
            for k in 0..SeasonCalendar::T3_EVENTS_PER_MONTH {
                let t = SeasonCalendar::t3_event(month, k, "2026-01-20");
                assert!(
                    seen3.insert(t.nickname.clone()),
                    "T3 月 {month} 场 {k} 昵称重复: {}",
                    t.nickname
                );
            }
        }
    }

    /// A（跨月去重 · 命名归一化）：开局月（month_idx=-1）的 T2/T3 赛事名
    /// slot 必须按月内场次归一化（`#0-1..#0-8`），不得出现 t5 证据里的
    /// `#0--7` 负号畸形；且开局月与 2 月（m=0）的同 label 赛事名唯一区分。
    #[test]
    fn opening_month_t2_t3_names_are_normalized_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for k in 0..SeasonCalendar::T2_EVENTS_PER_MONTH {
            let t = SeasonCalendar::t2_event(
                -1,
                -SeasonCalendar::T2_EVENTS_PER_MONTH + k,
                "2026-01-03",
            );
            assert!(
                !t.name.contains("--"),
                "开局月 T2 名不得含负号畸形：{}",
                t.name
            );
            assert!(t.name.contains("#0-"), "开局月 T2 名应为 #0-*：{}", t.name);
            seen.insert(t.name);
        }
        for k in 0..SeasonCalendar::T3_EVENTS_PER_MONTH {
            let t = SeasonCalendar::t3_event(
                -1,
                -SeasonCalendar::T3_EVENTS_PER_MONTH + k,
                "2026-01-20",
            );
            assert!(
                !t.name.contains("--"),
                "开局月 T3 名不得含负号畸形：{}",
                t.name
            );
            assert!(t.name.contains("#0-"), "开局月 T3 名应为 #0-*：{}", t.name);
            seen.insert(t.name);
        }
        // 2 月（m=0）对应场次与开局月同名? 不允许——跨月去重唯一实例。
        for k in 0..SeasonCalendar::T2_EVENTS_PER_MONTH {
            let t = SeasonCalendar::t2_event(0, k, "2026-02-03");
            assert!(
                !seen.contains(&t.name),
                "2 月 T2 名不得与开局月重名：{}",
                t.name
            );
        }
        for k in 0..SeasonCalendar::T3_EVENTS_PER_MONTH {
            let t = SeasonCalendar::t3_event(0, k, "2026-02-20");
            assert!(
                !seen.contains(&t.name),
                "2 月 T3 名不得与开局月重名：{}",
                t.name
            );
        }
        // t5 证据的精确回归：`Exort Fiesta Series #0--7`（1 月 20 日首场 T3，
        // month=-1、pool_offset=-8、k=0）不再出现——归一化为 `#0-1`。
        let exort = SeasonCalendar::t3_event(-1, -8, "2026-01-20");
        assert_eq!(
            exort.name, "Exort Fiesta Series #0-1",
            "t5 畸形名 #0--7 已归一化为 #0-1"
        );
        // label 轮换不受 slot 归一化影响（pool_offset 分片值原样保留）。
        let feb_exort = SeasonCalendar::t3_event(0, 7, "2026-02-27");
        assert_eq!(
            feb_exort.name, "Exort Fiesta Series #1-8",
            "2 月 Exort 仍为 #1-8"
        );
        assert_ne!(exort.name, feb_exort.name, "跨月同名实例必须唯一区分");
    }

    #[test]
    fn top_tier_density_by_month() {
        // 1 月与 7 月都有顶级赛事（BLAST Bounty / IEM Kraków 等），无整月休赛。
        assert!(!SeasonCalendar::top_tier_specs(2026, 1).is_empty());
        assert!(!SeasonCalendar::top_tier_specs(2026, 7).is_empty());
        // Major 开赛月：6 月科隆、11 月新加坡；12 月是收官月，无新开 Major。
        let has_major = |month: i32| {
            SeasonCalendar::top_tier_specs(2026, month)
                .iter()
                .any(|s| s.tier == TourneyTier::Major)
        };
        assert!(has_major(6));
        assert!(has_major(11));
        assert!(!has_major(12), "12 月是 Major 收官月，不是开赛月");
    }
}
