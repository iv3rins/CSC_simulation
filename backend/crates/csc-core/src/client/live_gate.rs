use crate::state::GameState;
// —— R2 赛事对象化 + LIVE 比赛日闸门（t2/t5 权威投影） ——

/// 单场「今日可 LIVE」对阵（比赛日闸门查询；序列化 = 前端 `LiveTodayMatch`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveGateFixture {
    pub event_name: String,
    pub event_date: String,
    pub tier: &'static str,
    pub fixture_id: u32,
    pub round: u32,
    pub team_a: u32,
    pub team_b: u32,
    pub stage: &'static str,
    pub best_of: i32,
    /// t2 契约（可选，旧后端缺失时前端回退全量 /state）：双方首发（权威口径，排除退役）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_a_roster: Option<Vec<LivePlayerSlot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_b_roster: Option<Vec<LivePlayerSlot>>,
}

/// 首发选手槽（LIVE 阵容展示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LivePlayerSlot {
    pub id: u32,
    pub name: String,
}

/// 玩家今日可 LIVE 的对阵（`live/today` 权威源，t5）。
///
/// 判定（与 `csc-tournaments::skip_live_today` 的语义严格一致）：
/// - 主角队赛事（`player_team_id` 有值）；
/// - 赛事未完结（非 Completed/Cancelled）；
/// - 开赛日 == 今日（`date == 时钟日`）；
/// - fixture 为 Pending、非轮空（`team_b != TeamId::NONE`）、未跳过（`!skipped`）。
///
/// 返回空 = 今天无玩家比赛可 LIVE（比赛日闸门关闭）。
pub fn player_live_fixtures(state: &GameState) -> Vec<LiveGateFixture> {
    let today = format!(
        "{:04}-{:02}-{:02}",
        state.sim_year, state.sim_month, state.sim_day
    );
    let mut out = Vec::new();
    for rec in &state.scheduled_records {
        let Some(player_team) = rec.player_team_id else {
            continue;
        };
        if matches!(
            rec.status,
            csc_tournaments::scheduled::ScheduledStatus::Completed
                | csc_tournaments::scheduled::ScheduledStatus::Cancelled
        ) {
            continue;
        }
        if rec.date != today {
            continue;
        }
        let team_a_roster = state
            .world
            .roster(player_team)
            .into_iter()
            .map(|p| LivePlayerSlot {
                id: p.id.0,
                name: p.name.clone(),
            })
            .collect::<Vec<_>>();
        let team_b_roster = state
            .world
            .roster(
                rec.team_ids
                    .iter()
                    .find(|t| **t != player_team)
                    .copied()
                    .unwrap_or(csc_util::id::TeamId::NONE),
            )
            .into_iter()
            .map(|p| LivePlayerSlot {
                id: p.id.0,
                name: p.name.clone(),
            })
            .collect::<Vec<_>>();
        for f in &rec.fixtures {
            if f.status != csc_tournaments::scheduled::FixtureStatus::Pending {
                continue;
            }
            if f.team_b == csc_util::id::TeamId::NONE {
                continue; // 轮空无比赛
            }
            if f.skipped {
                continue;
            }
            out.push(LiveGateFixture {
                event_name: rec.event_name.clone(),
                event_date: rec.date.clone(),
                tier: rec.tier.name(),
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
                team_a_roster: Some(team_a_roster.clone()),
                team_b_roster: Some(team_b_roster.clone()),
            });
        }
    }
    out
}
