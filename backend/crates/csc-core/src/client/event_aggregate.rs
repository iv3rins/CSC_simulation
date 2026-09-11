use super::calendar::{ClientFixture, project_fixtures, scheduled_status_label};
use crate::state::GameState;
/// 赛事对象化：参赛队伍（事件侧栏/对阵布局展示）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EventTeam {
    pub id: u32,
    pub name: String,
    pub vrs_ranking: i32,
    pub vrs_value: i32,
}

/// 赛事对象化：冠军（已完赛才有）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EventChampion {
    pub team_id: u32,
    pub name: String,
}

/// 赛事对象化聚合（R2）：按赛事名返回参赛队伍 + fixtures + 冠军 + 主角参与态。
/// 传输投影只序列化（前端读取）；`PartialEq` 供测试断言（`ClientFixture` 链
/// 不反序列化，故聚合本身不 derive Deserialize）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EventAggregate {
    pub event_name: String,
    pub tier: &'static str,
    pub date: String,
    pub end_date: String,
    /// `future`|`active`|`completed`|`cancelled`
    pub status: &'static str,
    pub teams: Vec<EventTeam>,
    pub fixtures: Vec<ClientFixture>,
    pub champion: Option<EventChampion>,
    pub player_participating: bool,
    /// 主角参赛但无对阵可打时的原因（完满 = null）。
    pub emptiness_reason: Option<String>,
}

/// R2 赛事对象化：按赛事名返回聚合对象（参赛队伍 + VRS + 冠军 + 对阵布局 +
/// 空状态原因）。供 LIVE 赛事页「赛事信息侧栏」与「下一场队伍赛事」承诺兑现。
pub fn event_aggregate(state: &GameState, event_name: &str) -> Option<EventAggregate> {
    let rec = state
        .scheduled_records
        .iter()
        .find(|r| r.event_name == event_name)?;
    // 队伍信息（按排程参赛名单；VRS 排名/积分取世界缓存的队伍值）。
    let teams: Vec<EventTeam> = rec
        .team_ids
        .iter()
        .filter_map(|tid| {
            let team = state.world.team(*tid)?;
            Some(EventTeam {
                id: tid.0,
                name: team.name.clone(),
                vrs_ranking: team.vrs_ranking,
                vrs_value: team.vrs_value,
            })
        })
        .collect();
    // fixtures 投影（与 player_calendar_events 同款）。
    let fixtures = project_fixtures(&rec.fixtures, &state.world);
    // 冠军：从已完赛结果按名关联。幽灵队（旧档已删队伍）兜底 `#id` 名，
    // 避免静默丢冠军（P1-6 ②）。
    let champion = state
        .events
        .iter()
        .find(|e| e.event.name == event_name)
        .map(|e| {
            let name = state
                .world
                .team(e.champion)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| format!("#{}", e.champion.0));
            EventChampion {
                team_id: e.champion.0,
                name,
            }
        });
    let player_participating = rec.player_team_id.is_some();
    // 空状态原因：主角参赛但赛事已完赛/取消（无对阵可打）时说明。
    let emptiness_reason = if player_participating
        && matches!(
            rec.status,
            csc_tournaments::scheduled::ScheduledStatus::Completed
                | csc_tournaments::scheduled::ScheduledStatus::Cancelled
        ) {
        Some(match rec.status {
            csc_tournaments::scheduled::ScheduledStatus::Completed => "赛事已完赛".to_string(),
            csc_tournaments::scheduled::ScheduledStatus::Cancelled => "赛事已取消".to_string(),
            _ => unreachable!(),
        })
    } else if player_participating
        && rec
            .fixtures
            .iter()
            .all(|f| f.status == csc_tournaments::scheduled::FixtureStatus::Bye)
    {
        Some("主角本轮轮空（无对阵）".to_string())
    } else {
        None
    };
    // 锁窗口状态提升（P1-7）：scheduled_records 已 Completed 但 `locks` 仍含
    // 主角队伍（锁窗口未结束）→ 标 active，与 player_calendar_events 语义一致，
    // 消除「LIVE 侧栏 completed / 我的赛程 active」割裂。
    let lock_active = rec.player_team_id.is_some()
        && state.locks.iter().any(|l| {
            l.event_name == event_name && l.team_ids.contains(&rec.player_team_id.unwrap())
        });
    let status: &'static str = if matches!(
        rec.status,
        csc_tournaments::scheduled::ScheduledStatus::Completed
    ) && lock_active
    {
        "active"
    } else {
        scheduled_status_label(rec.status)
    };
    // end_date：优先用 locks.end_epoch_day 反解（跨月赛事结束日准确）；
    // 无锁/0 值兜底 = date（单日赛口径，P1-6 ③）。
    let end_date = state
        .locks
        .iter()
        .find(|l| l.event_name == event_name && l.end_epoch_day > 0)
        .map(|l| {
            let (y, m, d) = csc_time::civil::civil_from_days(l.end_epoch_day);
            format!("{y:04}-{m:02}-{d:02}")
        })
        .unwrap_or_else(|| rec.date.clone());
    Some(EventAggregate {
        event_name: rec.event_name.clone(),
        tier: rec.tier.name(),
        date: rec.date.clone(),
        end_date,
        status,
        teams,
        fixtures,
        champion,
        player_participating,
        emptiness_reason,
    })
}
