use std::collections::HashSet;

use csc_domain::city::{City, Region};
use csc_domain::match_result::MatchVenue;
use csc_domain::tournament::{InvitePolicy, Organizer, Tournament};
use csc_domain::tourney_tier::TourneyTier;
use csc_time::clock::SimClock;
use csc_tournaments::calendar::SeasonCalendar;

use super::state::{
    ClientBracketSlot, ClientTournamentResult, MajorPresentation, series_has_player,
};
use crate::state::GameState;
///
/// 用途：主页「下一场/年度赛历」与赛程页的计划视图——引擎逐月动态排程，
/// 前端若只看「当前锁」，月底锁过期后就永远显示「等待赛历排期」。
pub fn season_calendar_plan(year: i32) -> Vec<Tournament> {
    let calendar = SeasonCalendar;
    let mut plan = Vec::new();
    for month in 1..=12 {
        // 引擎首个模拟月是 2026-02（初始时钟 2026-01-01，advance_month 先
        // next_month_start），且 director.month 从 0 起计数。因此 T2/T3 的
        // 「月份序号 m」按自然月平移：2026-02 → m=0、2027-01 → m=11。
        // 2026-01 是展示用季前计划（引擎不模拟），同样回退 m=0。
        let month_idx = ((year - 2026) * 12 + (month - 2)).max(0);
        for event in calendar.events_of(&SimClock::of(year, month as u32, 1)) {
            let date = format!("{year}-{month:02}-{:02}", event.start_day);
            plan.push((event.build)(month_idx, &date));
        }
    }
    plan.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.name.cmp(&b.name)));
    plan
}

/// 某一自然年的**全部完赛赛事结果**（含 NPC 队伍之间的胜负；series 不克隆，
/// 只下发与 `matches` 下标对齐的 bracket 元数据）。
///
/// 与 [`client_state_from`] 的主角过滤不同：赛程页需要展示整个世界在打什么，
/// 否则「赛程中完全没有数据」——即使其它 127 支队伍每月都在比赛。
pub fn calendar_results_for_year(state: &GameState, year: i32) -> Vec<ClientTournamentResult> {
    let protagonist_name = state
        .world
        .players
        .iter()
        .find(|p| p.is_player() && !p.retired)
        .map(|p| p.name.as_str());
    let mut events = Vec::new();
    for ev in state
        .events
        .iter()
        .filter(|ev| ev.event.date.starts_with(&format!("{year}-")))
    {
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
    events
}

/// 玩家主角“我的赛程”条目：`GET /games/{id}/calendar` 的 `player_events` 数据源。
///
/// P0-A：前端不再用 `isMyTournament(tier, rank)` 把全年同档赛事全塞进“我的赛程”，
/// 而是由后端按**真实参赛事实**生成：
/// - 进行中 = 本月 `locks` 中含主角队伍的赛事（`status = "scheduled"`）；
/// - 已完赛/待开赛 = 主角 `career.attended_tournaments` 中非进行中的赛事
///   （引擎在赛事排程/开赛时即 `record_participation`，含完整 `Tournament`）。
///
/// 主角识别（项目统一约定）：`world.players` 中 `is_player() && !retired` 的唯一角色。
/// `plan` 为年度静态赛历（`season_calendar_plan`），用于进行中赛事在
/// `attended_tournaments` 尚无记录时按名回退补完整对象。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlayerCalendarEvent {
    pub event: Tournament,
    /// 展示状态：`"future"`（已排定待开赛）| `"active"`（进行中）|
    /// `"completed"`（已完赛）| `"cancelled"`（参赛不足取消）。
    /// 旧存档回退路径保留 `"scheduled"`（进行中）语义。
    pub status: &'static str,
    /// 是否已确认参赛（本列表仅含玩家实际参与的赛事，恒为 `true`）。
    pub participation: bool,
    /// 赛前物化的首轮对阵（T3；旧存档/回退路径为空数组）。
    #[serde(default)]
    pub fixtures: Vec<ClientFixture>,
}

/// 前端对阵投影：队伍 ID 对 + 状态 + 完赛结果（pending 时 result=None）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClientFixture {
    /// 赛事内稳定对阵 ID（同 seed 排程确定不变）。
    #[serde(default)]
    pub fixture_id: u32,
    /// 轮次（1 基首轮）。
    #[serde(default)]
    pub round: u32,
    /// 主队（home）。
    pub team_a: u32,
    /// 客队（away；TeamId::NONE.0 = 轮空）。
    pub team_b: u32,
    /// 阶段（group/playoff/quarterfinal/semifinal/final/unknown）。
    pub stage: &'static str,
    /// 本场赛制（BO1/BO3/BO5；轮空为 0）。
    pub best_of: i32,
    /// 对阵状态（pending/completed/bye）。
    #[serde(default)]
    pub status: &'static str,
    #[serde(default)]
    pub result: Option<ClientFixtureResult>,
    /// R2：已看完标记（`fixtures/watched` 持久语义，随存档；前端 types.ts:513）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watched: Option<bool>,
}

/// 对阵结果（胜者队伍 ID + 比分；与 MatchResult 同源投影）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClientFixtureResult {
    pub winner: u32,
    pub loser: u32,
    /// 决定性图回合比分（BO1，如 13:12）或系列胜场数（BO3/BO5，如 2:0）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ClientFixtureScore>,
}

/// 对阵比分（前端 types.ts ClientFixtureScore；纯展示，不影响结算）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ClientFixtureScore {
    pub team_a_score: i32,
    pub team_b_score: i32,
}

pub(crate) fn project_fixtures(
    fixtures: &[csc_tournaments::scheduled::ScheduledFixture],
    world: &csc_entities::world::World,
) -> Vec<ClientFixture> {
    let id_of_sig = |sig: &str| -> Option<u32> {
        world
            .all_teams()
            .iter()
            .find(|t| world.signature_of(t.id) == sig)
            .map(|t| t.id.0)
    };
    fixtures
        .iter()
        .map(|f| ClientFixture {
            fixture_id: f.fixture_id,
            round: f.round,
            team_a: f.team_a.0,
            team_b: f.team_b.0,
            stage: match f.stage {
                csc_simulation::series::SeriesStage::Group => "group",
                csc_simulation::series::SeriesStage::Playoff => "playoff",
                csc_simulation::series::SeriesStage::Quarterfinal => "quarterfinal",
                csc_simulation::series::SeriesStage::Semifinal => "semifinal",
                csc_simulation::series::SeriesStage::Final => "final",
                csc_simulation::series::SeriesStage::Unknown => "unknown",
            },
            best_of: f.best_of,
            status: match f.status {
                csc_tournaments::scheduled::FixtureStatus::Pending => "pending",
                csc_tournaments::scheduled::FixtureStatus::Completed => "completed",
                csc_tournaments::scheduled::FixtureStatus::Bye => "bye",
            },
            result: f.result.as_ref().and_then(|m| {
                Some(ClientFixtureResult {
                    winner: id_of_sig(&m.winner_sig)?,
                    loser: id_of_sig(&m.loser_sig)?,
                    score: f.score.as_ref().map(|s| ClientFixtureScore {
                        team_a_score: s.team_a_score,
                        team_b_score: s.team_b_score,
                    }),
                })
            }),
            watched: f.watched.then_some(true),
        })
        .collect()
}
/// map ScheduledStatus to display status string.
pub fn scheduled_status_label(status: csc_tournaments::scheduled::ScheduledStatus) -> &'static str {
    match status {
        csc_tournaments::scheduled::ScheduledStatus::Future => "future",
        csc_tournaments::scheduled::ScheduledStatus::Active => "active",
        csc_tournaments::scheduled::ScheduledStatus::Completed => "completed",
        csc_tournaments::scheduled::ScheduledStatus::Cancelled => "cancelled",
    }
}

/// Build a lightweight Tournament from a scheduled record (P1-1 future authoritative projection).
///
/// Scheduling confirmation happens before the event starts, so Future records are not yet
/// in `attended_tournaments`; construct the display object directly from the record's own
/// event_name/tier/date rather than matching by name against attended/plan.
fn lightweight_tournament(
    rec: &csc_tournaments::scheduled::ScheduledTournamentRecord,
) -> Tournament {
    Tournament::new(
        rec.event_name.clone(),
        rec.tier,
        Organizer::Other,
        City::new("TBD", "TBD", Region::Other),
        1,
        match rec.tier {
            TourneyTier::Major | TourneyTier::SuperElite | TourneyTier::Elite | TourneyTier::T1 => {
                InvitePolicy::VrsGlobal
            }
            TourneyTier::T2 => InvitePolicy::Qualifier,
            TourneyTier::Qualify => InvitePolicy::Open,
        },
    )
    .with_date(rec.date.clone())
    .with_venue(match rec.tier {
        TourneyTier::Major | TourneyTier::SuperElite | TourneyTier::Elite | TourneyTier::T1 => {
            MatchVenue::Lan
        }
        _ => MatchVenue::Online,
    })
}

pub fn player_calendar_events(
    state: &GameState,
    year: i32,
    plan: &[Tournament],
) -> Vec<PlayerCalendarEvent> {
    let protagonist = state
        .world
        .players
        .iter()
        .find(|p| p.is_player() && !p.retired);
    let protagonist_team = protagonist.and_then(|p| p.team);
    let year_prefix = format!("{year}-");

    // ① 首选权威路径：投影主角实际参赛的 `scheduled_records`（P1-1）。
    if !state.scheduled_records.is_empty() {
        // 进行中窗口集合：locks 中含主角队（队伍级锁，只记 event_name/end_day）。
        // 赛事在锁定期（窗口未结束）内即使已模拟完也视为 active——保证
        // `player_calendar_events` 与 `locks`（LIVE 卡）指向同一状态机。
        let locked_names: HashSet<&str> = state
            .locks
            .iter()
            .filter(|l| protagonist_team.is_some_and(|tid| l.team_ids.contains(&tid)))
            .map(|l| l.event_name.as_str())
            .collect();
        let mut events: Vec<PlayerCalendarEvent> = Vec::new();
        for rec in &state.scheduled_records {
            // 只投影主角实际 participants 中的记录；转队/自由身后不再命中主角队。
            let is_mine = rec
                .player_team_id
                .is_some_and(|tid| protagonist_team == Some(tid));
            if !is_mine || !rec.date.starts_with(&year_prefix) {
                continue;
            }
            // 直接从排程记录构造轻量 Tournament（排程确认时点早于赛事开始，
            // future 记录尚未进入 attended_tournaments，因此不能靠 attended/plan 按名匹配）。
            let ev = lightweight_tournament(rec);
            // 状态：locks 中仍在窗口 = active（进行中），即使记录已 Completed——
            // 引擎在赛事模拟完成时即标 Completed，但锁持续到月末（LIVE 语义）。
            // 仅 Completed 提升（模拟完但仍锁）；Future 保持 future（未开赛，锁为预占）。
            let status = if locked_names.contains(rec.event_name.as_str())
                && rec.status == csc_tournaments::scheduled::ScheduledStatus::Completed
            {
                "active"
            } else {
                scheduled_status_label(rec.status)
            };
            events.push(PlayerCalendarEvent {
                event: ev,
                status,
                participation: true,
                fixtures: project_fixtures(&rec.fixtures, &state.world),
            });
        }
        // 排序：future 优先于 active，且 future/active 在前（组内按日期升序待开赛）；
        // completed/cancelled 在后（日期倒序）。
        events.sort_by(|a, b| {
            let a_pending = a.status == "future" || a.status == "active";
            let b_pending = b.status == "future" || b.status == "active";
            match (a_pending, b_pending) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                // 待开赛/进行中：future 优先于 active，组内按日期升序。
                (true, true) => {
                    let a_future = a.status == "future";
                    let b_future = b.status == "future";
                    b_future
                        .cmp(&a_future)
                        .then_with(|| a.event.date.cmp(&b.event.date))
                }
                (false, false) => b.event.date.cmp(&a.event.date),
            }
        });
        return events;
    }

    // ② 回退路径（scheduled_records 为空：旧存档/旧测试直构）：
    // 进行中赛事名集合：locks 中含主角队（队伍级锁，只记 event_name/end_day）。
    let locked_names: HashSet<&str> = state
        .locks
        .iter()
        .filter(|l| protagonist_team.is_some_and(|tid| l.team_ids.contains(&tid)))
        .map(|l| l.event_name.as_str())
        .collect();
    // 玩家已排程/已开赛的完整赛事（career.attended_tournaments）。
    let attended: Vec<Tournament> = protagonist
        .and_then(|p| p.career.as_ref())
        .map(|c| c.attended_tournaments.clone())
        .unwrap_or_default();
    let mut events: Vec<PlayerCalendarEvent> = Vec::new();
    // ① 进行中：优先用 attended 中的完整对象；仍锁但无 attended（同月赛事刚锁）则
    // 回退到年度静态赛历 plan 按名匹配。
    for name in &locked_names {
        let ev = attended
            .iter()
            .find(|t| t.name == *name)
            .cloned()
            .or_else(|| plan.iter().find(|t| t.name == *name).cloned());
        if let Some(ev) = ev {
            events.push(PlayerCalendarEvent {
                event: ev,
                status: "scheduled",
                participation: true,
                fixtures: Vec::new(),
            });
        }
    }
    // ② 已完赛：attended 中非进行中的。
    for ev in &attended {
        if !locked_names.contains(ev.name.as_str()) {
            events.push(PlayerCalendarEvent {
                event: ev.clone(),
                status: "completed",
                participation: true,
                fixtures: Vec::new(),
            });
        }
    }
    // 排序：进行中在前，已完赛按日期倒序。
    events.sort_by(|a, b| {
        let a_sched = a.status == "scheduled";
        let b_sched = b.status == "scheduled";
        b_sched
            .cmp(&a_sched)
            .then_with(|| b.event.date.cmp(&a.event.date))
    });
    // 仅保留本年度赛事（跨年存档时 attended 可能含往季）。
    events.retain(|e| e.event.date.starts_with(&year_prefix));
    events
}
