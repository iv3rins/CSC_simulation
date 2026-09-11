//! 世界查询层（Kotlin `WorldQuery.kt` 转写）——表现层/服务器协议的**只读边界**。
//!
//! 设计原则：
//! - 只读：全部方法不修改任何状态（防御性拷贝/纯值返回）；
//! - 聚合：组合实体仓库/VRS/赛事结果/事件日志/生涯档案/决策日志，UI 不再直接摸活对象；
//! - **窄依赖**：只注入各子系统的只读引用（不依赖 Engine 装配根——查询层与
//!   编排层解耦，UI/服务端可独立组装查询视图）；
//! - 纯值：返回 profile/record/entry 等值对象（serde 直译）。

use std::collections::HashMap;

use csc_career::archive::{CareerArchive, SeasonRecord, SeasonTotals};
use csc_decision::log::DecisionLog;
use csc_decision::point::PlayerDecision;
use csc_entities::mark::CareerMark;
use csc_entities::power::PowerCalculator;
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_time::clock::SimClock;
use csc_tournaments::engine::TournamentEngine;
use csc_tournaments::scheduled::TournamentResult;
use csc_tournaments::top20::{Top20Commentary, Top20Evaluator, Top20Score, Top20YearBoard};
use csc_vrs::engine::VrsEngine;
use csc_vrs::entry::VrsEntry;

/// 世界查询层——全部只读。
pub struct WorldQuery<'a> {
    pub world: &'a World,
    pub vrs: &'a VrsEngine,
    pub tournaments: &'a TournamentEngine,
    pub journal: &'a WorldJournal,
    pub archive: &'a CareerArchive,
    pub decision_log: &'a DecisionLog,
    pub clock: &'a SimClock,
    /// 叙事文案包（查询层派生文本用）
    pub text: &'a csc_text::TextBundle,
}

/// 从任意 (world, archive) 组合计算主角生涯结局——活体查询与服务端快照路径
/// 共用的**唯一实现**（历史上 routes.get_ending 曾自行重建档案再评估，与
/// `WorldQuery::career_ending` 漂移的风险已消除）。
pub fn career_ending_of(
    world: &World,
    archive: &CareerArchive,
    text: &csc_text::TextBundle,
) -> Option<(csc_util::id::PlayerId, String, csc_career::CareerEnding)> {
    let pid = world.players.iter().find(|p| p.is_player()).map(|p| p.id)?;
    let name = world.player(pid)?.name.clone();
    let totals = archive.totals_of(pid)?;
    let career = world.player(pid)?.career.as_ref()?;
    let ending = csc_career::CareerEndingEvaluator::evaluate(pid, &name, &totals, career, text);
    Some((pid, name, ending))
}

/// 世界概览（表现层首页）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorldSummary {
    pub date: String,
    pub month: i32,
    pub team_count: usize,
    pub npc_count: usize,
    pub player_count: usize,
    pub event_count: usize,
    pub journal_cursor: i32,
    pub decisions_logged: usize,
}

/// 玩家档案（纯值，表现层只读视图）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayerProfile {
    pub name: String,
    pub age: i32,
    pub role: csc_entities::role::Role,
    /// 所在队伍名（None = 自由身）
    pub team: Option<String>,
    pub power: f64,
    pub fatigue: f64,
    /// 伤病摘要（None = 健康）
    pub injury: Option<String>,
    pub reputation: i32,
    pub salary: i64,
    pub contract_years: i32,
    pub cash: i64,
    pub career_earnings: i64,
    /// 代言摘要（"品牌（年付/年，剩 N 年）"）
    pub sponsors: Vec<String>,
    pub marks: Vec<CareerMark>,
    /// 印记强度摘要（"TYPE×N"）
    pub marks_summary: Vec<String>,
    pub career_wins: i32,
    pub career_kills: i32,
    pub career_deaths: i32,
}

impl<'a> WorldQuery<'a> {
    /// 世界概览（表现层首页）。
    pub fn world_summary(&self) -> WorldSummary {
        WorldSummary {
            date: self.clock.date_label(),
            month: self.clock.now().1 as i32,
            team_count: self.world.all_teams().len(),
            npc_count: self.world.all_npcs().len(),
            player_count: self.world.all_players_only().len(),
            // 只读计数（2026 性能修订：不 clone 全部赛事结果）
            event_count: self.tournaments.event_count(),
            journal_cursor: self.journal.cursor(),
            decisions_logged: self.decision_log.count(),
        }
    }

    /// 实时 VRS 排名（按排名升序）。
    pub fn rankings(&self) -> Vec<VrsEntry> {
        self.vrs.ranking_entries()
    }

    /// 玩家档案（表现层"我的生涯"页数据源）。
    pub fn player_profile(&self, name: &str) -> Option<PlayerProfile> {
        let pid = self.world.player_by_name(name)?;
        let pc = self.world.player(pid)?;
        let career = pc.career.as_ref()?;
        Some(PlayerProfile {
            name: pc.name.clone(),
            age: pc.age,
            role: pc.role,
            team: pc.team.map(|tid| {
                self.world
                    .team(tid)
                    .map(|t| t.name.clone())
                    .unwrap_or_default()
            }),
            power: PowerCalculator::player_power(pc),
            fatigue: pc.fatigue,
            injury: pc.injury.as_ref().map(|i| {
                format!(
                    "{}（{}，剩 {} 月）",
                    i.kind.label(),
                    severity_name(i.severity),
                    i.days_left
                )
            }),
            reputation: career.reputation,
            salary: career.salary,
            contract_years: career.contract_years,
            cash: career.finance.cash,
            career_earnings: career.finance.career_earnings,
            sponsors: career
                .finance
                .sponsors
                .iter()
                .map(|s| {
                    format!(
                        "{}（{}/年，剩 {} 年）",
                        s.brand, s.annual_value, s.years_left
                    )
                })
                .collect(),
            marks: career.marks.clone(),
            marks_summary: {
                let mut map: HashMap<csc_entities::mark::CareerMarkType, i32> = HashMap::new();
                for m in &career.marks {
                    *map.entry(m.r#type).or_insert(0) += m.strength;
                }
                let mut v: Vec<String> = map.iter().map(|(t, s)| format!("{:?}×{s}", t)).collect();
                v.sort();
                v
            },
            career_wins: career.career_wins,
            career_kills: career.career_kills,
            career_deaths: career.career_deaths,
        })
    }

    /// 某玩家的生涯档案（逐赛季，年份升序；按名字反解稳定 ID，UI 友好入口）。
    pub fn career(&self, name: &str) -> Vec<SeasonRecord> {
        self.world
            .player_by_name(name)
            .map(|pid| self.archive.seasons_of(pid))
            .unwrap_or_default()
    }

    /// 某玩家的生涯汇总。
    pub fn career_totals(&self, name: &str) -> Option<SeasonTotals> {
        let pid = self.world.player_by_name(name)?;
        self.archive.totals_of(pid)
    }

    /// 主角的生涯结局/复盘（退役页/通关页数据源）。
    /// `CareerEndingEvaluator` 综合生涯汇总 + 结构化荣誉 + 印记产出定论。
    ///
    /// 注意：从 arena 按 `is_player` 查找（**含退役保留**）——主角 36 岁退役后
    /// `all_players_only()` 为空，此前退役后结局永远查不到（试玩捕获）。
    pub fn career_ending(&self) -> Option<(String, csc_career::CareerEnding)> {
        career_ending_of(self.world, self.archive, self.text)
            .map(|(_, name, ending)| (name, ending))
    }

    /// 某玩家的生涯档案（按稳定 ID——谱系可靠，服务端/重名场景用）。
    pub fn career_of(&self, player_id: csc_util::id::PlayerId) -> Vec<SeasonRecord> {
        self.archive.seasons_of(player_id)
    }

    /// 全部事件（按 seq 升序）。
    pub fn journal_all(&self) -> Vec<WorldEvent> {
        self.journal.all()
    }

    /// 增量拉取：seq 之后的事件（UI/服务端同步游标）。
    pub fn journal_since(&self, seq: i32) -> Vec<WorldEvent> {
        self.journal.since(seq)
    }

    /// 某玩家的相关事件（叙事/复盘；按名字，展示用）。
    pub fn journal_of(&self, player_name: &str) -> Vec<WorldEvent> {
        self.journal.of(player_name)
    }

    /// 某玩家的相关事件（按稳定 ID，谱系可靠）。
    pub fn journal_of_player(&self, player_id: csc_util::id::PlayerId) -> Vec<WorldEvent> {
        self.journal.of_player(player_id)
    }

    /// 已完赛的全部赛事结果。
    pub fn event_results(&self) -> Vec<TournamentResult> {
        self.tournaments.results()
    }

    /// 玩家决策历史（决策日志，可复现性的对外视图）。
    pub fn decision_history(&self) -> Vec<PlayerDecision> {
        self.decision_log.entries()
    }

    /// 当前年度 HLTV TOP20 实时榜单（纯值 DTO，服务端协议边界）。
    /// 委托 [`top20_of`]。
    pub fn top20(&self) -> Vec<Top20Entry> {
        top20_of(
            self.world,
            &self.tournaments.yearly_rating_snapshot(),
            self.clock.year(),
            self.text,
        )
    }
}

/// 年度 TOP20 榜单条目（纯值 DTO，serde 直译给服务端/前端）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Top20Entry {
    pub rank: usize,
    pub player_id: csc_util::id::PlayerId,
    pub player_name: String,
    /// 2026 复审新增：所属队伍名（HLTV 风格单榜的队伍列；自由身/无队伍 = 空串）
    pub team_name: String,
    pub score: f64,
    pub weighted_rating: f64,
    pub honor_points: f64,
    pub playoff_rating: Option<f64>,
    pub maps: i32,
    pub mvp_count: i32,
    pub evp_count: i32,
    pub comment: String,
    /// 天才少年通配入围（低阶赛事 Rating 极优、无 T1+ 样本的年轻选手）
    pub wildcard: bool,
}

/// 从年度累计器 + 实体仓库实时计算当前年度 TOP20 榜单（纯值 DTO）。
///
/// 这是 TOP20 查询的唯一业务门面：server 侧持有 GameState 快照，直接调用本函数；
/// `WorldQuery::top20()` 亦委托本函数。含**天才少年通配**（Top20Evaluator 内）。
pub fn top20_of(
    world: &World,
    yearly_rating: &csc_tournaments::yearly_rating::YearlyRatingTracker,
    year: i32,
    text: &csc_text::TextBundle,
) -> Vec<Top20Entry> {
    let ranked: Vec<Top20Score> =
        Top20Evaluator::evaluate_with_wildcards(world, yearly_rating, year)
            .into_iter()
            .take(20)
            .collect();
    ranked
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let player_name = world
                .player(s.player_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            Top20Entry {
                rank: i + 1,
                player_id: s.player_id,
                player_name,
                team_name: world
                    .player(s.player_id)
                    .and_then(|p| p.team)
                    .and_then(|tid| world.team(tid))
                    .map(|t| t.name.clone())
                    .unwrap_or_default(),
                score: s.score,
                weighted_rating: s.weighted_rating,
                honor_points: s.honor_points,
                playoff_rating: s.playoff_rating,
                maps: s.maps,
                mvp_count: s.mvp_count,
                evp_count: s.evp_count,
                comment: Top20Commentary::comment(&ranked, i, text),
                wildcard: s.wildcard,
            }
        })
        .collect()
}

/// 从已收官年度的榜单快照重建 TOP20 展示条目（含评语）。
///
/// 跨年后年度累计器已清空、新赛季尚无 T1+ 样本——实时榜会只剩通配/空榜。
/// 此时服务端回退展示最近一届完整榜单，避免玩家误以为「世界赛事没有模拟」。
pub fn top20_of_board(board: &Top20YearBoard, text: &csc_text::TextBundle) -> Vec<Top20Entry> {
    let ranked: Vec<Top20Score> = board
        .entries
        .iter()
        .map(|b| Top20Score {
            player_id: b.player_id,
            score: b.score,
            weighted_rating: b.weighted_rating,
            honor_points: b.honor_points,
            playoff_rating: b.playoff_rating,
            maps: b.maps,
            mvp_count: b.mvp_count,
            evp_count: b.evp_count,
            wildcard: b.wildcard,
        })
        .collect();
    ranked
        .iter()
        .enumerate()
        .map(|(i, s)| Top20Entry {
            rank: i + 1,
            player_id: s.player_id,
            player_name: board.entries[i].player_name.clone(),
            team_name: String::new(),
            score: s.score,
            weighted_rating: s.weighted_rating,
            honor_points: s.honor_points,
            playoff_rating: s.playoff_rating,
            maps: s.maps,
            mvp_count: s.mvp_count,
            evp_count: s.evp_count,
            comment: Top20Commentary::comment(&ranked, i, text),
            wildcard: s.wildcard,
        })
        .collect()
}

/// 伤病严重度名（= Kotlin `severity.name`）。
pub fn severity_name(s: csc_entities::injury::InjurySeverity) -> &'static str {
    match s {
        csc_entities::injury::InjurySeverity::Minor => "MINOR",
        csc_entities::injury::InjurySeverity::Moderate => "MODERATE",
        csc_entities::injury::InjurySeverity::Severe => "SEVERE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;
    use csc_util::rng::Xoshiro256StarStar;

    fn fixture() -> (
        World,
        VrsEngine,
        TournamentEngine,
        WorldJournal,
        CareerArchive,
        DecisionLog,
        SimClock,
    ) {
        let mut world = World::new();
        let t = world.create_team("Vitality", 1, 2000);
        for i in 0..5 {
            let name = if i == 0 {
                "MyPlayer".to_string()
            } else {
                format!("N{i}")
            };
            if i == 0 {
                let p = world.create_player(
                    Tier::Tier1,
                    Some(&name),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                    None,
                );
                world.assign_player_to_team(p, t).unwrap();
            } else {
                world
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
        let vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        (
            world,
            vrs,
            TournamentEngine::new(),
            WorldJournal::default(),
            CareerArchive::default(),
            DecisionLog::default(),
            SimClock::of(2026, 6, 1),
        )
    }

    #[test]
    fn summary_and_profile() {
        let (world, vrs, tournaments, journal, archive, decision_log, clock) = fixture();
        let q = WorldQuery {
            world: &world,
            vrs: &vrs,
            tournaments: &tournaments,
            journal: &journal,
            archive: &archive,
            decision_log: &decision_log,
            clock: &clock,
            text: &csc_text::TextBundle::default(),
        };
        let s = q.world_summary();
        assert_eq!(s.team_count, 1);
        assert_eq!(s.player_count, 1);
        assert_eq!(s.npc_count, 4);
        assert_eq!(s.date, "2026-06-01");

        let p = q.player_profile("MyPlayer").unwrap();
        assert_eq!(p.name, "MyPlayer");
        assert_eq!(p.team.as_deref(), Some("Vitality"));
        assert!(p.power > 0.0);
        assert_eq!(p.injury, None);
        assert!(q.player_profile("Ghost").is_none());
    }

    #[test]
    fn rankings_and_decision_history() {
        let (world, vrs, tournaments, journal, archive, decision_log, clock) = fixture();
        let q = WorldQuery {
            world: &world,
            vrs: &vrs,
            tournaments: &tournaments,
            journal: &journal,
            archive: &archive,
            decision_log: &decision_log,
            clock: &clock,
            text: &csc_text::TextBundle::default(),
        };
        assert!(q.rankings().is_empty(), "空 VRS 库无排名");
        assert!(q.decision_history().is_empty());
        assert!(q.event_results().is_empty());
    }

    /// event_count 只读计数与 results().len() 一致（性能修订后语义不漂移）。
    #[test]
    fn event_count_matches_results_len() {
        let (_world, _vrs, tournaments, _journal, _archive, _decision_log, _clock) = fixture();
        assert_eq!(tournaments.event_count(), 0);
        assert_eq!(tournaments.event_count(), tournaments.results().len());
    }
}
