//! 世界叙事推导（2026「世界可感知性」）——从 [`csc_core::state::GameState`] 纯只读地
//! 派生「队伍风云 + 宿敌」，让 NPC 世界从「随机比分生成器」变成「活的竞争者」。
//!
//! 设计原则：
//! - **纯只读、确定性**：不修改任何状态、不消费 RNG——同存档同输出（表现层派生，
//!   不进决策/状态变更，不破坏可复现性）；
//! - **数据源全部来自既有 GameState**：`world`（阵容/年龄/潜力）、`events`
//!   （赛事结果/冠军）、`journal`（转会流）、`top20_history`（人才产出）——
//!   不新增状态字段，存档格式不变（`GameState::CURRENT_VERSION` 不动）；
//! - **不做角色/队伍配额**：纯由既有数据推导故事骨架，Narrator 负责措辞。

use csc_events::event::WorldEvent;
use csc_tournaments::scheduled::TournamentResult;

use crate::state::GameState;

/// 单支队伍的叙事骨架（serde 直译给前端）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TeamStory {
    pub team_id: u32,
    pub team_name: String,
    pub vrs_ranking: i32,
    /// 故事原型（"DYNASTY" / "RISING" / "REBUILDING" / "SLUMP" / "CONTENDER" / "STABLE"）
    pub archetype: &'static str,
    /// 一句话标题
    pub headline: String,
    /// 叙事正文（多句，\n 分段）
    pub body: String,
}

/// 一对宿敌队伍。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Rivalry {
    pub team_a: String,
    pub team_b: String,
    /// 近期交手次数（window 内）
    pub meetings: i32,
    /// 一句话标题
    pub headline: String,
}

/// 世界叙事聚合（`GET /games/{id}/world` 的数据源）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WorldStories {
    pub stories: Vec<TeamStory>,
    pub rivalries: Vec<Rivalry>,
}

/// 最近赛事窗口（用于胜率/交手统计）。
const RECENT_EVENTS: usize = 40;
/// 转会流窗口（用于「重建」判定）。
const TURNOVER_WINDOW: i32 = 24; // 最近 24 个月
/// 青年军年龄阈值（≤ 此年龄算「小将」）。
const YOUNG_AGE: i32 = 22;
/// 「重建」的入队转会数门槛。
const TURNOVER_THRESHOLD: i32 = 2;

/// 从 GameState 推导世界叙事（纯只读；文案来自 `text` 包）。
pub fn world_stories(state: &GameState, text: &csc_text::TextBundle) -> WorldStories {
    let world = &state.world;
    // 最近窗口：`events` 是 append-only 时间序 Vec，取**最后** RECENT_EVENTS 条
    let recent: Vec<&TournamentResult> = state.events.iter().rev().take(RECENT_EVENTS).collect();
    // 每队：近期胜率 + 交手计数（rivalries 用）
    let mut meetings: std::collections::HashMap<String, std::collections::HashMap<String, i32>> =
        std::collections::HashMap::new();
    let mut team_stats: std::collections::HashMap<String, (i32, i32)> =
        std::collections::HashMap::new(); // (wins, matches)
    for ev in &recent {
        // 冠军
        if let Some(team) = world.team(ev.champion) {
            let e = team_stats.entry(team.name.clone()).or_insert((0, 0));
            e.0 += 1;
            e.1 += 1;
        }
        for m in &ev.matches {
            let a = sig_team(&m.winner_sig);
            let b = sig_team(&m.loser_sig);
            {
                let e = team_stats.entry(a.to_string()).or_insert((0, 0));
                e.0 += 1;
                e.1 += 1;
            }
            {
                let e = team_stats.entry(b.to_string()).or_insert((0, 0));
                e.1 += 1;
            }
            let (a2, b2) = (a.to_string(), b.to_string());
            let e = meetings.entry(a2.clone()).or_default();
            *e.entry(b2.clone()).or_insert(0) += 1;
            let e = meetings.entry(b2).or_default();
            *e.entry(a2).or_insert(0) += 1;
        }
    }

    // 转会流（重建判定）
    let turnover: std::collections::HashMap<String, i32> = state
        .journal
        .iter()
        .filter(|e| recent_months(state, e) < TURNOVER_WINDOW)
        .filter_map(|e| match e {
            WorldEvent::TransferDone { to_team, .. } => Some(to_team.clone()),
            _ => None,
        })
        .fold(std::collections::HashMap::new(), |mut acc, team| {
            *acc.entry(team).or_insert(0) += 1;
            acc
        });

    // 逐队推导故事
    let mut stories: Vec<TeamStory> = world
        .all_teams()
        .iter()
        .map(|team| {
            let roster = world.roster(team.id);
            let active: Vec<_> = roster.iter().filter(|p| !p.retired).collect();
            let avg_age = if active.is_empty() {
                24.0
            } else {
                active.iter().map(|p| p.age as f64).sum::<f64>() / active.len() as f64
            };
            let young = active.iter().filter(|p| p.age <= YOUNG_AGE).count() as i32;
            let (wins, matches) = team_stats.get(&team.name).copied().unwrap_or((0, 0));
            let win_rate = if matches > 0 {
                wins as f64 / matches as f64
            } else {
                0.0
            };
            let in_turnovers = turnover.get(&team.name).copied().unwrap_or(0);
            let rank = team.vrs_ranking;

            let (archetype, headline, body) = classify(
                &team.name,
                rank,
                avg_age,
                young,
                win_rate,
                matches,
                in_turnovers,
                text,
            );
            TeamStory {
                team_id: team.id.0,
                team_name: team.name.clone(),
                vrs_ranking: rank,
                archetype,
                headline,
                body,
            }
        })
        .collect();
    stories.sort_by_key(|s| s.vrs_ranking);

    // 宿敌：交手最频繁的队对（窗口内 ≥2 次才算「恩怨」）
    let mut pairs: Vec<(String, String, i32)> = meetings
        .iter()
        .flat_map(|(a, map)| {
            map.iter()
                .filter(move |(b, _)| b.as_str() > a.as_str())
                .map(move |(b, n)| (a.clone(), b.clone(), *n))
        })
        .collect();
    pairs.sort_by(|x, y| y.2.cmp(&x.2).then_with(|| x.0.cmp(&y.0)));
    let rivalries: Vec<Rivalry> = pairs
        .into_iter()
        .filter(|(_, _, n)| *n >= 2)
        .take(6)
        .map(|(a, b, n)| Rivalry {
            headline: text.format("story.rivalry", &[&a, &b, &n.to_string()]),
            team_a: a,
            team_b: b,
            meetings: n,
        })
        .collect();

    WorldStories { stories, rivalries }
}

/// 队伍故事分类（纯规则，不做配额；参数多为并列判定量，保持平铺以贴合规则表）。
#[allow(clippy::too_many_arguments)]
fn classify(
    name: &str,
    rank: i32,
    avg_age: f64,
    young: i32,
    win_rate: f64,
    matches: i32,
    in_turnovers: i32,
    text: &csc_text::TextBundle,
) -> (&'static str, String, String) {
    let low_sample = matches < 3;
    // 王朝：前 5 + 高胜率（近期冠军多）
    if !low_sample && rank <= 5 && win_rate >= 0.6 {
        return (
            "DYNASTY",
            text.format("story.dynasty.headline", &[name]),
            text.format(
                "story.dynasty.body",
                &[name, &rank.to_string(), &format!("{:.0}", win_rate * 100.0)],
            ),
        );
    }
    // 青年军崛起：小将 ≥3 且胜率不错
    if !low_sample && young >= 3 && win_rate >= 0.5 {
        return (
            "RISING",
            text.format("story.rising.headline", &[name]),
            text.format(
                "story.rising.body",
                &[
                    &format!("{avg_age:.0}"),
                    &young.to_string(),
                    &matches.to_string(),
                    &format!("{:.0}", win_rate * 100.0),
                ],
            ),
        );
    }
    // 重建：近期大量换血
    if in_turnovers >= TURNOVER_THRESHOLD && win_rate < 0.5 {
        return (
            "REBUILDING",
            text.format("story.rebuilding.headline", &[name]),
            text.format("story.rebuilding.body", &[name, &in_turnovers.to_string()]),
        );
    }
    // 低迷：近期胜率惨淡且样本足
    if !low_sample && win_rate < 0.35 {
        return (
            "SLUMP",
            text.format("story.slump.headline", &[name]),
            text.format(
                "story.slump.body",
                &[
                    &matches.to_string(),
                    &format!("{:.0}", win_rate * 100.0),
                    name,
                ],
            ),
        );
    }
    // 争冠：前 10 + 中等胜率
    if rank <= 10 && win_rate >= 0.5 {
        return (
            "CONTENDER",
            text.format("story.contender.headline", &[name]),
            text.format(
                "story.contender.body",
                &[&rank.to_string(), name, &format!("{:.0}", win_rate * 100.0)],
            ),
        );
    }
    // 稳定
    (
        "STABLE",
        text.format("story.stable.headline", &[name]),
        text.format("story.stable.body", &[name, &rank.to_string()]),
    )
}

/// 事件距今的月数（粗略按事件日期年份差 ×12 估算；无日期字段用 0）。
fn recent_months(state: &GameState, e: &WorldEvent) -> i32 {
    let y = match e
        .date()
        .split('-')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
    {
        Some(y) => y,
        None => return 0,
    };
    let now = state.sim_year;
    if y > now { 0 } else { (now - y) * 12 }
}

/// 从签名剥离队名（`队名|选手` → 队名）。
fn sig_team(sig: &str) -> &str {
    sig.split('|').next().unwrap_or(sig)
}

#[cfg(test)]
use csc_entities::world::World;

/// 便捷：构造一条队伍故事（测试用）。
#[cfg(test)]
fn story_of(
    world: &World,
    team_name: &str,
    avg_age: f64,
    young: i32,
    win_rate: f64,
    matches: i32,
    turnovers: i32,
) -> Option<TeamStory> {
    let tid = world.team_by_name(team_name)?;
    let team = world.team(tid)?;
    let (archetype, headline, body) = classify(
        team_name,
        team.vrs_ranking,
        avg_age,
        young,
        win_rate,
        matches,
        turnovers,
        &csc_text::TextBundle::default(),
    );
    Some(TeamStory {
        team_id: tid.0,
        team_name: team.name.clone(),
        vrs_ranking: team.vrs_ranking,
        archetype,
        headline,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_dynasty_and_slump() {
        // 王朝
        let (a, h, _) = classify(
            "NaVi",
            1,
            25.0,
            0,
            0.8,
            20,
            0,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(a, "DYNASTY");
        assert!(h.contains("统治"));
        // 低迷
        let (a2, h2, _) = classify(
            "BIG",
            30,
            24.0,
            0,
            0.2,
            15,
            0,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(a2, "SLUMP");
        assert!(h2.contains("低谷"));
    }

    #[test]
    fn classify_rising_and_rebuild() {
        let (a, h, _) = classify(
            "Spirit",
            12,
            20.0,
            4,
            0.6,
            18,
            1,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(a, "RISING");
        assert!(h.contains("崛起"));
        let (a2, _, _) = classify(
            "Fnatic",
            25,
            24.0,
            0,
            0.3,
            12,
            3,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(a2, "REBUILDING");
    }

    #[test]
    fn sig_team_strips_signature() {
        assert_eq!(sig_team("Vitality|a,b,c"), "Vitality");
        assert_eq!(sig_team("Vitality"), "Vitality");
    }

    #[test]
    fn stories_reference_by_name() {
        let mut world = World::new();
        world.create_team("NaVi", 1, 2000);
        let s = story_of(&world, "NaVi", 25.0, 0, 0.8, 20, 0).unwrap();
        assert_eq!(s.archetype, "DYNASTY");
    }

    /// M6 回归：统计窗口 = 最近 40 场（不是最早 40 场）。
    #[test]
    fn recent_window_takes_latest_events() {
        use csc_domain::tournament::Tournament;
        use csc_domain::tourney_tier::TourneyTier;
        use csc_util::id::TeamId;

        fn event_result(champion: TeamId, date: &str) -> TournamentResult {
            TournamentResult {
                event: Tournament::new(
                    format!("T1 #{date}"),
                    TourneyTier::T1,
                    csc_domain::tournament::Organizer::Other,
                    csc_domain::city::City::new("TBD", "TBD", csc_domain::city::Region::Other),
                    1,
                    csc_domain::tournament::InvitePolicy::VrsGlobal,
                )
                .with_date(date.to_string()),
                champion,
                matches: vec![],
                series: vec![],
            }
        }

        let mut world = World::new();
        let team_a = world.create_team("TeamA", 1, 2000);
        let team_b = world.create_team("TeamB", 2, 2000);
        let mut state = crate::state::GameState {
            version: crate::state::GameState::CURRENT_VERSION,
            month: 0,
            last_contract_year: 2026,
            locks: vec![],
            sim_year: 2026,
            sim_month: 1,
            sim_day: 1,
            rng_state: [0, 0, 0, 0],
            decisions: vec![],
            journal: vec![],
            archive: std::collections::HashMap::new(),
            world,
            vrs: csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: csc_tournaments::yearly_rating::YearlyRatingTracker::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: crate::state::WORLD_SIM_VERSION,
            narrative: Default::default(),
            execution: crate::state::ExecutionState::Idle,
        };
        // 45 条事件：前 5 条 TeamA 冠军（最早），后 40 条 TeamB 冠军（最近）
        for i in 0..45 {
            let (champion, day) = if i < 5 {
                (team_a, i + 1)
            } else {
                (team_b, i + 1)
            };
            state
                .events
                .push(event_result(champion, &format!("2026-01-{day:02}")));
        }
        let stories = world_stories(&state, &csc_text::TextBundle::default());
        // 最近 40 场窗口：TeamB 冠军计数 = 40，TeamA 不在窗口内（0）
        let team_b_story = stories
            .stories
            .iter()
            .find(|s| s.team_name == "TeamB")
            .unwrap();
        assert_eq!(
            team_b_story.archetype, "DYNASTY",
            "TeamB 最近 40 场全冠军 → 高胜率王朝"
        );
        // 直接验证窗口语义：TeamA 冠军只在最早 5 条（窗口外）→ 计数 0
        let stats_a = team_stats_of(&state, "TeamA");
        assert_eq!(stats_a, 0, "TeamA 冠军只在最早 5 条（窗口外）→ 计数 0");
        let stats_b = team_stats_of(&state, "TeamB");
        assert_eq!(stats_b, 40, "TeamB 冠军在最近 40 条（窗口内）→ 计数 40");
    }

    /// 测试辅助：复刻 world_stories 的 team_stats 冠军计数逻辑（窗口内）。
    fn team_stats_of(state: &crate::state::GameState, team_name: &str) -> i32 {
        let world = &state.world;
        let recent: Vec<&TournamentResult> =
            state.events.iter().rev().take(RECENT_EVENTS).collect();
        recent
            .iter()
            .filter(|ev| {
                world
                    .team(ev.champion)
                    .map(|t| t.name == team_name)
                    .unwrap_or(false)
            })
            .count() as i32
    }
}
