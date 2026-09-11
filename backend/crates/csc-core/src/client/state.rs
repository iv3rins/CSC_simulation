//! 客户端视图（Client State）——服务端协议为「网页端可玩」而设的**轻量投影**。
//!
//! 设计动机（试玩审查 P0）：
//! [`crate::state::GameState`] 是**完整存档聚合根**：20 年生涯 JSON ≈68MB，
//! 其中绝大部分是 `events`（全部赛事结果 + 全部 series 逐图 KDA）与
//! `journal`（全量事件流）。前端每次 `GET /state` 全量替换显然不可扩展。
//!
//! 本视图只携带**表现层轮询所需的最小一致状态**：
//! - 主角 + 当前队伍 roster（而非全部 640 选手）；
//! - 全队伍的名字/排名引用（赛程页锁展示用）；
//! - 主角个人的年度 Rating 标量（而非全量 YearlyRatingTracker）；
//! - 主角生涯档案（而非按 PlayerId 键控的全档案 map）；
//! - 只保留主角参赛过的赛事结果；**不下发任何 SeriesResult**——只下发与
//!   `matches` 下标对齐的 `replayable` 布尔向量，点击回放时由
//!   `GET /games/{id}/replay` 懒加载单场 series；
//! - 事件流（journal）走既有 `GET /journal?since=` 增量协议，不进本视图。
//!
//! 完整存档仍由 `/state`（兼容/调试）与 `/save`（下载）提供；网页端高频刷新

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use csc_career::archive::{CareerArchive, SeasonRecord};
use csc_decision::log::DecisionLog;
use csc_domain::event_importance::EventImportance;
use csc_domain::tourney_tier::TourneyTier;
use csc_entities::character::PlayerCharacter;
use csc_entities::team::Team;
use csc_entities::world::World;
use csc_simulation::series::{SeriesResult, SeriesStage};
use csc_tournaments::engine::TournamentEngine;
use csc_util::id::TeamId;

use crate::state::LockRecord;

/// 客户端视图协议版本（与存档 `GameState.version` 相互独立——这是传输投影格式）。
pub const CLIENT_VIEW_VERSION: u32 = 1;

/// 首页需要关注的下一个赛事节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CareerMilestone {
    pub event_name: String,
    pub date: String,
    pub days_until: i64,
    pub importance: EventImportance,
    pub live: bool,
}

/// 队伍轻量引用（赛程页锁 / 冠军名反解）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamRef {
    pub id: TeamId,
    pub name: String,
    pub vrs_ranking: i32,
}

/// 主角年度 Rating 标量（原 `YearlyRatingTracker` 只对主角有意义的子集）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClientSeason {
    /// 加权场均 Rating（T1+ 口径；无样本 = None）
    pub rating: Option<f64>,
    /// T1+ 参赛图数
    pub maps: i32,
    /// 全赛事场均 Rating（含 T2/预选——数据中心兜底显示，不再长期挂 "—"）
    pub all_rating: f64,
    /// 全赛事参赛图数
    pub maps_all: i32,
    /// 年度击杀 / 死亡 / 胜场（全赛事口径，生涯档案一致）
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub rounds: i32,
    pub wins: i32,
}

/// 客户端世界投影：主角 + 队友 + 当前队伍 + 全队名字引用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientWorld {
    /// 主角 + 当前 roster（含主角；自由身时只有主角）。
    /// 主角 = `career != null && !retired` 的那一条（与旧全量 arena 语义一致）。
    pub players: Vec<PlayerCharacter>,
    /// 主角当前队伍完整实体（自由身/已退役 = None）。
    /// Team 页面需要 chemistry / budget / roster_ids / vrs 缓存。
    pub team: Option<Team>,
    /// 全部队伍的名字/排名引用（约 300 条 × 30B，远小于完整 Team.chemistry）。
    pub teams: Vec<TeamRef>,
}

/// 赛事结果客户端投影：只保留主角参赛的赛事。
/// **series 不下发**——只发与 `matches` 下标对齐的 bracket 元数据
/// （阶段/赛制/是否可回放），点开直播时由 `/replay` 端点懒加载单场 series。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientTournamentResult {
    pub event: csc_domain::tournament::Tournament,
    pub champion: TeamId,
    pub matches: Vec<csc_domain::match_result::MatchResult>,
    pub replayable: Vec<bool>,
    pub bracket: Vec<ClientBracketSlot>,
}

/// 单场 bracket 元数据（赛历树状图数据源，不含逐图 KDA）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientBracketSlot {
    pub stage: SeriesStage,
    pub best_of: i32,
    pub replayable: bool,
    /// 是否是可进入 LIVE 的系列赛（由赛事 importance 与比赛阶段共同决定）。
    #[serde(default)]
    pub live: bool,
    /// Major 入场仪式所需的最小展示数据；动画保留在前端。
    #[serde(default)]
    pub presentation: Option<MajorPresentation>,
}

/// Major/Championship 的前端入场仪式数据，不携带动画逻辑。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MajorPresentation {
    pub event_name: String,
    pub stage: SeriesStage,
    pub team_a: TeamId,
    pub team_b: TeamId,
    pub best_of: i32,
}

/// 网页端高频轮询的轻量状态视图（`GET /games/{id}/view`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientState {
    /// 下一个职业生涯重要节点（用于首页 NEXT / LIVE 倒计时）。
    pub next_milestone: Option<CareerMilestone>,
    /// 客户端视图格式版本
    pub view_version: u32,
    /// 模拟月份计数（0 起）
    pub month: i32,
    /// 显示日期（"YYYY-MM-DD"）
    pub date: String,
    pub sim_year: i32,
    pub sim_month: u32,
    pub sim_day: u32,
    /// 本月进行中赛事的队伍级锁
    pub locks: Vec<LockRecord>,
    /// 世界投影（见 [`ClientWorld`]）
    pub world: ClientWorld,
    /// 主角年度 Rating 标量（无主角 = None）
    pub season: Option<ClientSeason>,
    /// 主角生涯档案（逐赛季，年份升序）
    pub archive: Vec<SeasonRecord>,
    /// 主角参赛过的赛事结果（series 已裁剪，见 [`ClientTournamentResult`]）
    pub events: Vec<ClientTournamentResult>,
    /// 世界概览计数（原 `/summary` 的数据源；随视图一次下发，省第二把锁）
    pub team_count: usize,
    pub player_count: usize,
    pub npc_count: usize,
    pub event_count: usize,
    pub journal_len: usize,
    pub decisions_logged: usize,
}

/// 判断一条 series 是否有该选手出场（按战绩行昵称匹配）。
pub(crate) fn series_has_player(series: &SeriesResult, player_name: &str) -> bool {
    series
        .maps
        .iter()
        .any(|m| m.lines.iter().any(|line| line.player_name == player_name))
}

/// 从引擎只读组件构造客户端视图（不 clone 全量赛事结果/全量事件流）。
///
/// `tournaments.results_ref()` 提供零 clone 的历史切片；本函数只 clone
/// 「主角参赛过」的子集与裁剪后的 series。
#[allow(clippy::too_many_arguments)]
pub fn client_state_from(
    month: i32,
    sim_year: i32,
    sim_month: u32,
    sim_day: u32,
    locks: Vec<LockRecord>,
    world: &World,
    tournaments: &TournamentEngine,
    archive: &CareerArchive,
    journal_len: usize,
    decision_log: &DecisionLog,
    scheduled_records: &[csc_tournaments::scheduled::ScheduledTournamentRecord],
) -> ClientState {
    // 主角（活跃；退役后视图退化为「无主角」——结局页走 /ending，不依赖本字段）
    let protagonist = world.players.iter().find(|p| p.is_player() && !p.retired);

    // roster 投影：主角 + 当前队友（按 team.roster_ids 顺序）
    let mut players: Vec<PlayerCharacter> = Vec::with_capacity(6);
    let team = protagonist
        .and_then(|p| p.team)
        .and_then(|tid| world.team(tid))
        .cloned();
    if let Some(team) = &team {
        for id in &team.roster_ids {
            if let Some(pc) = world.player(*id) {
                players.push(pc.clone());
            }
        }
    }
    if let Some(pc) = protagonist
        && !players.iter().any(|p| p.id == pc.id)
    {
        players.push(pc.clone());
    }

    let teams: Vec<TeamRef> = world
        .all_teams()
        .iter()
        .map(|t| TeamRef {
            id: t.id,
            name: t.name.clone(),
            vrs_ranking: t.vrs_ranking,
        })
        .collect();

    // 主角年度 Rating：只取一个标量（旧实现把全量 tracker 发给前端）
    let season = protagonist.and_then(|pc| {
        let stats = tournaments.yearly_rating_snapshot().stats();
        let stat = stats.get(&pc.id)?;
        Some(ClientSeason {
            rating: (stat.maps > 0).then_some(stat.rating),
            maps: stat.maps,
            all_rating: stat.all_rating,
            maps_all: stat.maps_all,
            kills: stat.kills,
            deaths: stat.deaths,
            assists: stat.assists,
            rounds: stat.rounds,
            wins: stat.wins,
        })
    });

    // 主角生涯档案（原前端按 PlayerId 键控的 map → 只发主角一列）
    let archive_seasons = protagonist
        .map(|pc| archive.seasons_of(pc.id))
        .unwrap_or_default();

    // 只保留主角参赛赛事：优先按 career.attended_tournaments 的 (name,date) 匹配；
    // 旧存档/边界情况回退为「series 战绩行里出现过主角名」。
    let attended: Option<HashSet<(String, String)>> = protagonist.map(|pc| {
        pc.career
            .as_ref()
            .map(|career| {
                career
                    .attended_tournaments
                    .iter()
                    .map(|t| (t.name.clone(), t.date.clone()))
                    .collect()
            })
            .unwrap_or_default()
    });
    let protagonist_name = protagonist.map(|p| p.name.as_str());
    let mut events: Vec<ClientTournamentResult> = Vec::new();
    for ev in tournaments.results_ref() {
        let key = (ev.event.name.clone(), ev.event.date.clone());
        let attended_hit = attended.as_ref().is_some_and(|set| set.contains(&key));
        let player_hit = protagonist_name
            .is_some_and(|name| ev.series.iter().any(|s| series_has_player(s, name)));
        if !attended_hit && !player_hit {
            continue;
        }
        let replayable: Vec<bool> = ev
            .series
            .iter()
            .map(|s| protagonist_name.is_some_and(|name| series_has_player(s, name)))
            .collect();
        let bracket: Vec<ClientBracketSlot> = ev
            .series
            .iter()
            .map(|s| ClientBracketSlot {
                stage: s.stage,
                best_of: s.best_of,
                replayable: s.replayable,
                live: ev.event.importance.is_live() && s.replayable,
                presentation: (ev.event.importance.is_live() && s.stage.is_playoff()).then(|| {
                    MajorPresentation {
                        event_name: ev.event.name.clone(),
                        stage: s.stage,
                        team_a: s.team_a_id,
                        team_b: s.team_b_id,
                        best_of: s.best_of,
                    }
                }),
            })
            .collect();
        events.push(ClientTournamentResult {
            event: ev.event.clone(),
            champion: ev.champion,
            matches: ev.matches.clone(),
            replayable,
            bracket,
        });
    }

    let today_epoch = csc_time::civil::days_from_civil(sim_year, sim_month, sim_day);
    let team_rank = protagonist
        .and_then(|p| p.team)
        .and_then(|tid| world.team(tid))
        .map(|t| t.vrs_ranking);
    // T9/T19 统一事实源：优先取「已确认参赛」的下一场赛事（scheduled_records
    // 权威投影，与 player_events/CalendarPage 同源），不再用排名资格推断——
    // 排名波动的“潜在赛事”不再与“已确认赛事”卡面冲突。
    let confirmed_next = scheduled_records
        .iter()
        .filter(|rec| {
            rec.player_team_id
                .is_some_and(|tid| protagonist.and_then(|p| p.team) == Some(tid))
                && matches!(
                    rec.status,
                    csc_tournaments::scheduled::ScheduledStatus::Future
                        | csc_tournaments::scheduled::ScheduledStatus::Active
                )
        })
        .filter_map(|rec| {
            let mut parts = rec.date.split('-');
            let year = parts.next()?.parse::<i32>().ok()?;
            let month = parts.next()?.parse::<u32>().ok()?;
            let day = parts.next()?.parse::<u32>().ok()?;
            let days_until = csc_time::civil::days_from_civil(year, month, day) - today_epoch;
            (days_until >= 0).then_some(CareerMilestone {
                event_name: rec.event_name.clone(),
                date: rec.date.clone(),
                days_until,
                // tier → importance：与 scheduled.rs 的 with_importance 同一映射。
                importance: match rec.tier {
                    TourneyTier::Major => {
                        csc_domain::event_importance::EventImportance::Championship
                    }
                    TourneyTier::SuperElite | TourneyTier::Elite => {
                        csc_domain::event_importance::EventImportance::Major
                    }
                    TourneyTier::T1 => csc_domain::event_importance::EventImportance::Important,
                    TourneyTier::T2 | TourneyTier::Qualify => {
                        csc_domain::event_importance::EventImportance::Background
                    }
                },
                live: matches!(
                    rec.status,
                    csc_tournaments::scheduled::ScheduledStatus::Active
                ),
            })
        })
        .min_by_key(|milestone| milestone.days_until);
    // 兜底：无已确认参赛时回退静态赛历（按排名资格推断），保持旧行为不退化。
    let next_milestone = confirmed_next.or_else(|| {
        super::calendar::season_calendar_plan(sim_year)
            .into_iter()
            .chain(super::calendar::season_calendar_plan(sim_year + 1))
            .filter_map(|event| {
                let eligible = match team_rank {
                    Some(rank) if rank <= 12 => matches!(
                        event.tier,
                        TourneyTier::Major
                            | TourneyTier::SuperElite
                            | TourneyTier::Elite
                            | TourneyTier::T1
                    ),
                    Some(rank) if rank <= 32 => event.tier == TourneyTier::T2,
                    Some(_) => event.tier == TourneyTier::Qualify,
                    None => event.importance.is_career_milestone(),
                };
                if !eligible {
                    return None;
                }
                let mut parts = event.date.split('-');
                let year = parts.next()?.parse::<i32>().ok()?;
                let month = parts.next()?.parse::<u32>().ok()?;
                let day = parts.next()?.parse::<u32>().ok()?;
                let days_until = csc_time::civil::days_from_civil(year, month, day) - today_epoch;
                (days_until >= 0).then_some(CareerMilestone {
                    event_name: event.name,
                    date: event.date,
                    days_until,
                    importance: event.importance,
                    live: event.importance.is_live(),
                })
            })
            .min_by_key(|milestone| milestone.days_until)
    });

    ClientState {
        next_milestone,
        view_version: CLIENT_VIEW_VERSION,
        month,
        date: format!("{sim_year}-{sim_month:02}-{sim_day:02}"),
        sim_year,
        sim_month,
        sim_day,
        locks,
        world: ClientWorld {
            players,
            team,
            teams,
        },
        season,
        archive: archive_seasons,
        events,
        team_count: world.all_teams().len(),
        player_count: world.all_players_only().len(),
        npc_count: world.all_npcs().len(),
        event_count: tournaments.event_count(),
        journal_len,
        decisions_logged: decision_log.count(),
    }
}
