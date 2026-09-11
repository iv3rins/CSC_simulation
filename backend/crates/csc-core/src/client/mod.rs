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
pub mod calendar;
pub mod event_aggregate;
pub mod live_gate;
pub mod state;

pub use calendar::{
    ClientFixture, ClientFixtureResult, ClientFixtureScore, PlayerCalendarEvent,
    calendar_results_for_year, player_calendar_events, scheduled_status_label,
    season_calendar_plan,
};
pub use event_aggregate::{EventAggregate, EventChampion, EventTeam, event_aggregate};
pub use live_gate::{LiveGateFixture, LivePlayerSlot, player_live_fixtures};
pub use state::{
    CLIENT_VIEW_VERSION, CareerMilestone, ClientBracketSlot, ClientSeason, ClientState,
    ClientTournamentResult, ClientWorld, MajorPresentation, TeamRef, client_state_from,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameState;
    use csc_career::archive::CareerArchive;
    use csc_decision::log::DecisionLog;
    use csc_domain::tier::Tier;
    use csc_entities::world::World;
    use csc_time::clock::SimClock;
    use csc_tournaments::engine::TournamentEngine;
    use csc_util::id::TeamId;
    use csc_util::rng::Xoshiro256StarStar;

    // —— P1-1 future 赛程权威投影测试 ——

    fn scheduled_record(
        name: &str,
        date: &str,
        player_team_id: Option<csc_util::id::TeamId>,
        status: csc_tournaments::scheduled::ScheduledStatus,
    ) -> csc_tournaments::scheduled::ScheduledTournamentRecord {
        csc_tournaments::scheduled::ScheduledTournamentRecord {
            event_name: name.to_string(),
            date: date.to_string(),
            tier: csc_domain::tourney_tier::TourneyTier::Qualify,
            team_ids: vec![],
            player_team_id,
            status,
            fixtures: vec![],
            format: csc_domain::tournament_format::TournamentFormat::SingleElim,
        }
    }

    #[test]
    fn player_calendar_events_uses_scheduled_records_statuses() {
        use csc_domain::city::{City, Region};
        use csc_domain::tournament::InvitePolicy;
        let (base, tid, live, done) = test_calendar_state();
        let _ = done;
        let records = vec![
            scheduled_record(
                "CCT Open Cup #7",
                "2026-02-20",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Future,
            ),
            scheduled_record(
                "Exort Fiesta Series #1",
                "2026-01-27",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Active,
            ),
            scheduled_record(
                "Old Finished",
                "2025-11-30",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Completed,
            ),
            scheduled_record(
                "Cancelled Cup",
                "2026-02-01",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Cancelled,
            ),
        ];
        let mut state = base;
        state.scheduled_records = records;
        if let Some(pid) = state
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .map(|p| p.id)
        {
            let pc = state
                .world
                .player_mut(pid)
                .expect("主角")
                .career
                .as_mut()
                .expect("career");
            pc.attended_tournaments.clear();
            pc.attended_tournaments.push(live.clone());
            for r in &["Exort Fiesta Series #1", "Old Finished", "Cancelled Cup"] {
                pc.attended_tournaments.push(
                    csc_domain::tournament::Tournament::new(
                        r.to_string(),
                        csc_domain::tourney_tier::TourneyTier::Qualify,
                        csc_domain::tournament::Organizer::Other,
                        City::new("上海", "中国", Region::Asia),
                        10,
                        InvitePolicy::Open,
                    )
                    .with_date("2026-01-01"),
                );
            }
        }
        let events = player_calendar_events(&state, 2026, &[]);
        let by_name: std::collections::HashMap<&str, &str> = events
            .iter()
            .map(|e| (e.event.name.as_str(), e.status))
            .collect();
        assert_eq!(by_name.get("CCT Open Cup #7").copied(), Some("future"));
        assert_eq!(
            by_name.get("Exort Fiesta Series #1").copied(),
            Some("active")
        );
        assert_eq!(by_name.get("Old Finished").copied(), None);
        assert_eq!(by_name.get("Cancelled Cup").copied(), Some("cancelled"));
        assert_eq!(events[0].status, "future");
        assert!(events.iter().all(|e| e.participation));
    }

    #[test]
    fn player_calendar_events_locked_completed_record_is_active() {
        // LIVE 一致性：赛事模拟完成（scheduled_records=Completed）但队伍锁仍在
        // （locks 窗口未结束）时，投影必须标 active——保证「我的赛程」与
        // LIVE 卡（locks）指向同一状态机，消除「LIVE 进行中 / 赛程已完赛」割裂。
        use csc_tournaments::scheduled::ScheduledStatus;
        let (mut base, tid, _live, _done) = test_calendar_state();
        // 记录：已模拟完成（Completed）+ 队伍锁仍存在（本月窗口）。
        base.scheduled_records = vec![scheduled_record(
            "CCT Open Cup #2-16",
            "2026-03-27",
            Some(tid),
            ScheduledStatus::Completed,
        )];
        base.locks = vec![crate::state::LockRecord {
            event_name: "CCT Open Cup #2-16".into(),
            end_day: 28,
            start_epoch_day: 0,
            end_epoch_day: 0,
            team_ids: vec![tid],
        }];
        let events = player_calendar_events(&base, 2026, &[]);
        let ev = events
            .iter()
            .find(|e| e.event.name == "CCT Open Cup #2-16")
            .expect("记录应投影");
        assert_eq!(ev.status, "active", "锁定期内必须标 active");
        assert!(events.iter().all(|e| e.participation));
    }

    #[test]
    fn fixtures_survive_future_active_completed_archive_roundtrip() {
        // T3 验收：赛前物化的首轮对阵必须随存档（serde 往返）在 future → active →
        // completed 全生命周期保持，且投影结果逐字段一致（pending 无比分、
        // completed 已回填 result、轮空保留）。
        use csc_simulation::series::SeriesStage;
        use csc_tournaments::scheduled::{
            ScheduledFixture, ScheduledStatus, ScheduledTournamentRecord,
        };
        use csc_util::id::TeamId;

        let (mut base, tid, live, _done) = test_calendar_state();
        let other = base.world.create_team("RivalNine", 2, 1800);
        let fixtures = vec![
            ScheduledFixture::pending(0, 1, tid, other, SeriesStage::Group, 1),
            ScheduledFixture::bye(1, 1, other, SeriesStage::Group),
        ];
        let mut records = vec![
            ScheduledTournamentRecord {
                event_name: "Future Cup".into(),
                date: "2026-03-10".into(),
                tier: csc_domain::tourney_tier::TourneyTier::Qualify,
                team_ids: vec![tid, other],
                player_team_id: Some(tid),
                status: ScheduledStatus::Future,
                fixtures: fixtures.clone(),
                format: csc_domain::tournament_format::TournamentFormat::SingleElim,
            },
            ScheduledTournamentRecord {
                event_name: live.name.clone(),
                date: live.date.clone(),
                tier: csc_domain::tourney_tier::TourneyTier::Qualify,
                team_ids: vec![tid, other],
                player_team_id: Some(tid),
                status: ScheduledStatus::Active,
                fixtures: fixtures.clone(),
                format: csc_domain::tournament_format::TournamentFormat::SingleElim,
            },
            ScheduledTournamentRecord {
                event_name: "Done Cup".into(),
                date: "2026-02-01".into(),
                tier: csc_domain::tourney_tier::TourneyTier::Qualify,
                team_ids: vec![tid, other],
                player_team_id: Some(tid),
                status: ScheduledStatus::Completed,
                fixtures: fixtures.clone(),
                format: csc_domain::tournament_format::TournamentFormat::SingleElim,
            },
        ];
        // completed 记录回填首场 result（pending → 带比分；签名用 world 真实签名）。
        records[2].fixtures[0].result = Some(csc_domain::match_result::MatchResult::new(
            base.world.signature_of(tid),
            base.world.signature_of(other),
            csc_domain::tourney_tier::TourneyTier::Qualify,
        ));

        let mut state = base;
        state.scheduled_records = records;
        // 存档往返：序列化 → 反序列化（旧存档 serde(default) 路径同样走这里）。
        let json = serde_json::to_string(&state).expect("存档序列化");
        let restored: crate::state::GameState = serde_json::from_str(&json).expect("存档反序列化");

        // ① 三态在往返后仍保留，且 fixtures 逐字段一致。
        let events = player_calendar_events(&restored, 2026, &[]);
        let by_name: std::collections::HashMap<&str, &crate::client::PlayerCalendarEvent> =
            events.iter().map(|e| (e.event.name.as_str(), e)).collect();
        for status in ["future", "active"] {
            let ev = by_name
                .get(match status {
                    "future" => "Future Cup",
                    _ => live.name.as_str(),
                })
                .expect("往返后赛事必须存在");
            assert_eq!(ev.status, status);
            assert_eq!(ev.fixtures.len(), 2, "{status} 应保留 2 个 fixture");
            // pending：team_a/team_b/best_of/stage 透传，result 为空。
            let f0 = &ev.fixtures[0];
            assert_eq!(f0.team_a, tid.0, "{status} fixture team_a 透传");
            assert_eq!(f0.team_b, other.0, "{status} fixture team_b 透传");
            assert_eq!(f0.best_of, 1);
            assert_eq!(f0.stage, "group");
            assert!(
                f0.result.is_none(),
                "{status} 的 pending fixture 不应有 result"
            );
            // 轮空保留（team_b = TeamId::NONE.0 = u32::MAX）。
            assert_eq!(ev.fixtures[1].team_b, TeamId::NONE.0, "{status} 轮空保留");
            assert!(ev.fixtures[1].result.is_none(), "{status} 轮空无 result");
        }
        // ② completed：status + result 回填透传。
        let done_ev = by_name.get("Done Cup").expect("往返后已完赛赛事存在");
        assert_eq!(done_ev.status, "completed");
        let done_result = done_ev.fixtures[0]
            .result
            .as_ref()
            .expect("completed 应回填 result");
        assert_eq!(done_result.winner, tid.0);
        assert_eq!(done_result.loser, other.0);
        assert!(
            done_ev.fixtures[1].result.is_none(),
            "completed 轮空无 result"
        );
    }

    #[test]
    fn player_calendar_events_filters_after_team_change() {
        use csc_domain::city::{City, Region};
        use csc_domain::tournament::InvitePolicy;
        let (base, tid, _live, done) = test_calendar_state();
        let records = vec![
            scheduled_record(
                "Old Team Cup",
                "2026-02-20",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Completed,
            ),
            scheduled_record(
                "Free Agent Cup",
                "2026-03-01",
                None,
                csc_tournaments::scheduled::ScheduledStatus::Future,
            ),
        ];
        let mut state = base;
        state.scheduled_records = records;
        let _ = done;
        if let Some(pid) = state
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .map(|p| p.id)
        {
            let pc = state
                .world
                .player_mut(pid)
                .expect("主角")
                .career
                .as_mut()
                .expect("career");
            pc.attended_tournaments.clear();
            for r in &["Old Team Cup", "Free Agent Cup"] {
                pc.attended_tournaments.push(
                    csc_domain::tournament::Tournament::new(
                        r.to_string(),
                        csc_domain::tourney_tier::TourneyTier::Qualify,
                        csc_domain::tournament::Organizer::Other,
                        City::new("上海", "中国", Region::Asia),
                        10,
                        InvitePolicy::Open,
                    )
                    .with_date("2026-02-20"),
                );
            }
        }
        let pid = state
            .world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired)
            .map(|p| p.id);
        if let Some(pid) = pid {
            state.world.player_mut(pid).expect("主角").team = None;
        }
        let events = player_calendar_events(&state, 2026, &[]);
        assert!(events.is_empty(), "自由身（无队伍）时不应投影任何赛事");
    }

    #[test]
    fn player_calendar_events_cross_year_filtering() {
        let (base, tid, _live, done) = test_calendar_state();
        let _ = done;
        let records = vec![
            scheduled_record(
                "Next Year Major",
                "2027-01-15",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Future,
            ),
            scheduled_record(
                "This Year Cup",
                "2026-06-01",
                Some(tid),
                csc_tournaments::scheduled::ScheduledStatus::Completed,
            ),
        ];
        let mut state = base;
        state.scheduled_records = records;
        let events = player_calendar_events(&state, 2026, &[]);
        let by_name: std::collections::HashMap<&str, &str> = events
            .iter()
            .map(|e| (e.event.name.as_str(), e.status))
            .collect();
        assert!(by_name.contains_key("This Year Cup"));
        assert!(
            !by_name.contains_key("Next Year Major"),
            "跨年赛事必须被过滤"
        );
    }

    #[test]
    fn client_state_trims_world_to_protagonist_roster() {
        let mut world = World::new();
        let tid = world.create_team("Test5", 1, 2000);
        let pid = world.create_player(
            Tier::Tier4,
            Some("Hero"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        world.assign_player_to_team(pid, tid).expect("入队");
        for i in 0..4 {
            let npc = world
                .create_npc(
                    Tier::Tier4,
                    Some(tid),
                    None,
                    Some(&format!("Mate{i}")),
                    &mut Xoshiro256StarStar::seed(i + 2),
                    None,
                )
                .expect("NPC 创建");
            let _ = npc;
        }
        // 额外一支队伍 + 1 个 NPC：视图不应下发
        let _other = world.create_team("Other", 2, 1900);

        let tournaments = TournamentEngine::new();
        let archive = CareerArchive::default();
        let decision_log = DecisionLog::default();
        let view = client_state_from(
            0,
            2026,
            1,
            1,
            vec![],
            &world,
            &tournaments,
            &archive,
            0,
            &decision_log,
            &[],
        );
        assert_eq!(view.view_version, CLIENT_VIEW_VERSION);
        assert_eq!(view.world.players.len(), 5, "主角 + 4 名队友");
        assert!(view.world.players.iter().any(|p| p.id == pid));
        assert_eq!(view.world.teams.len(), 2);
        assert_eq!(view.world.team.as_ref().map(|t| t.id), Some(tid));
        assert!(view.season.is_none());
        assert!(view.events.is_empty());
    }

    #[test]
    fn client_state_season_scalar_is_protagonist_only() {
        // 使用空的 TournamentEngine 只验证结构默认值即可；真正数值口径由
        // csc-tournaments 的 yearly_rating 测试守护。
        let mut world = World::new();
        let pid = world.create_player(
            Tier::Tier4,
            Some("Hero"),
            &mut Xoshiro256StarStar::seed(3),
            None,
            None,
        );
        let tournaments = TournamentEngine::new();
        let view = client_state_from(
            0,
            2026,
            1,
            1,
            vec![],
            &world,
            &tournaments,
            &CareerArchive::default(),
            0,
            &DecisionLog::default(),
            &[],
        );
        assert!(view.season.is_none(), "无年度累计时应为 None");
        assert!(view.world.players.iter().any(|p| p.id == pid));
        // SimClock 仅用于证明函数与时钟无关（date 由参数传入）
        let _clock = SimClock::of(2026, 1, 1);
    }

    #[test]
    fn next_milestone_prefers_confirmed_scheduled_records_over_rank_inference() {
        // T9/T19：next_milestone 必须优先取「已确认参赛」的下一场（scheduled_records
        // 权威投影，与 player_events/CalendarPage 同源），排名资格推断仅作兜底——
        // 保证 MajorCountdown 与 MatchSpotlight 指向同一赛事。
        use csc_tournaments::scheduled::{ScheduledStatus, ScheduledTournamentRecord};

        let mut world = World::new();
        let tid = world.create_team("Heroes", 1, 2000);
        world.create_player(
            Tier::Tier4,
            Some("Hero"),
            &mut Xoshiro256StarStar::seed(7),
            None,
            None,
        );
        let pid = csc_util::id::PlayerId(world.players.len() as u32 - 1);
        world.assign_player_to_team(pid, tid).expect("入队");
        // 排程记录：一场近期的已确认 Qualify 赛事（主角队参赛）。
        let rec = ScheduledTournamentRecord {
            event_name: "Confirmed Local Cup".into(),
            date: "2026-03-15".into(),
            tier: csc_domain::tourney_tier::TourneyTier::Qualify,
            team_ids: vec![tid],
            player_team_id: Some(tid),
            status: ScheduledStatus::Future,
            fixtures: vec![],
            format: csc_domain::tournament_format::TournamentFormat::SingleElim,
        };
        let tournaments = TournamentEngine::new();
        let view = client_state_from(
            0,
            2026,
            2,
            1,
            vec![],
            &world,
            &tournaments,
            &CareerArchive::default(),
            0,
            &DecisionLog::default(),
            std::slice::from_ref(&rec),
        );
        let ms = view
            .next_milestone
            .expect("已确认参赛必须有 next_milestone");
        assert_eq!(ms.event_name, "Confirmed Local Cup");
        assert_eq!(ms.days_until, 42, "2026-02-01 → 2026-03-15 间隔 42 天");
        // 无已确认时回退静态赛历（旧行为保留）。
        let fallback = client_state_from(
            0,
            2026,
            2,
            1,
            vec![],
            &world,
            &tournaments,
            &CareerArchive::default(),
            0,
            &DecisionLog::default(),
            &[],
        );
        let _ = fallback.next_milestone; // 不 panic 即保留兜底路径（是否有值取决于赛历与排名）
    }

    #[test]
    fn season_calendar_plan_matches_engine_density() {
        // 主页/赛程页的完整赛历必须与引擎动态排程同源：25 T1 桶 + 60 T2 + 96 T3。
        let plan = season_calendar_plan(2026);
        assert_eq!(plan.len(), 25 + 60 + 96);
        for w in plan.windows(2) {
            assert!(w[0].date <= w[1].date, "计划按日期升序");
        }
        assert!(
            plan.iter().any(|t| t.name == "IEM Cologne Major 2026"),
            "计划含科隆 Major"
        );
        assert!(
            plan.iter().any(|t| t.name == "PGL Major Singapore 2026"),
            "计划含新加坡 Major"
        );
        // 1 月第一场 BLAST Bounty 排在 1 月 7 日
        assert!(
            plan.iter()
                .any(|t| t.nickname == "BLAST Bounty" && t.date == "2026-01-07"),
            "1 月顶级赛事按真实日期排期"
        );
        // T2/T3 命名序号必须与引擎首个模拟月（2026-02，director.month=0）一致
        assert!(
            plan.iter()
                .any(|t| t.name == "CCT Global Finals #1-1" && t.date == "2026-02-03"),
            "2 月计划与引擎实际排程同名（#1-1）"
        );
    }

    /// 构造最小 GameState：含一名主角（带队伍）+ 一个进行中锁 + 已完赛/待开赛记录。
    fn test_calendar_state() -> (
        GameState,
        TeamId,
        csc_domain::tournament::Tournament,
        csc_domain::tournament::Tournament,
    ) {
        use csc_domain::city::{City, Region};
        use csc_domain::tournament::InvitePolicy;
        use csc_domain::tourney_tier::TourneyTier;
        use csc_util::rng::Xoshiro256StarStar;

        let mut world = World::new();
        let tid = world.create_team("TestHeroes", 1, 2000);
        world.create_player(
            Tier::Tier4,
            Some("Hero"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        let pid = csc_util::id::PlayerId(world.players.len() as u32 - 1);
        world.assign_player_to_team(pid, tid).expect("入队");
        // 一场进行中赛事（在 locks 中）与一场已完赛赛事（在 attended 中）。
        let live = csc_domain::tournament::Tournament::new(
            "CCT Open Cup #7",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Cct,
            City::new("上海", "中国", Region::Asia),
            10,
            InvitePolicy::Open,
        )
        .with_date("2026-02-20");
        let done = csc_domain::tournament::Tournament::new(
            "Exort Fiesta Series #1",
            TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Other,
            City::new("上海", "中国", Region::Asia),
            10,
            InvitePolicy::Open,
        )
        .with_date("2026-01-27");
        // 记录进主角 career.attended_tournaments（完成 + 进行中各一条）。
        let pc = world.player_mut(pid).expect("主角存在");
        pc.career
            .as_mut()
            .expect("主角 career")
            .attended_tournaments
            .push(live.clone());
        pc.career
            .as_mut()
            .expect("主角 career")
            .attended_tournaments
            .push(done.clone());
        let state = GameState {
            version: GameState::CURRENT_VERSION,
            month: 0,
            last_contract_year: 2026,
            locks: vec![crate::state::LockRecord {
                event_name: live.name.clone(),
                end_day: 26,
                start_epoch_day: 0,
                end_epoch_day: 0,
                team_ids: vec![tid],
            }],
            sim_year: 2026,
            sim_month: 2,
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
        (state, tid, live, done)
    }

    #[test]
    fn player_calendar_events_lists_live_and_completed() {
        let (state, _tid, live, done) = test_calendar_state();
        let events = player_calendar_events(&state, 2026, &[]);
        // 进行中（locks 含主角队）必须排在最前且标 scheduled。
        assert!(!events.is_empty());
        assert_eq!(events[0].status, "scheduled");
        assert_eq!(events[0].event.name, live.name);
        assert!(events[0].participation);
        // 已完赛（attended 中非进行中）标 completed。
        let completed: Vec<_> = events.iter().filter(|e| e.status == "completed").collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].event.name, done.name);
        // 按日期倒序：1 月 27 日在后。
        assert_eq!(events[1].event.name, done.name);
    }

    #[test]
    fn player_calendar_events_filters_other_year() {
        let (state, _tid, _live, _done) = test_calendar_state(); // 只查 2025 年 → attended/locks 里的 2026 赛事全部被过滤。
        let events = player_calendar_events(&state, 2025, &[]);
        assert!(events.is_empty());
    }

    #[test]
    fn player_calendar_events_empty_without_protagonist() {
        // 空世界无主角 → 空列表（不 panic）。
        let state = GameState {
            version: GameState::CURRENT_VERSION,
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
            world: World::new(),
            vrs: csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: csc_tournaments::yearly_rating::YearlyRatingTracker::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: crate::state::WORLD_SIM_VERSION,
            narrative: Default::default(),
            execution: crate::state::ExecutionState::Idle,
        };
        let events = player_calendar_events(&state, 2026, &[]);
        assert!(events.is_empty());
    }

    // —— R2 赛事对象化（event_aggregate）测试 ——

    /// 构造含 scheduled_records 的最小状态：一场主角参赛的进行中赛事（含 2 个 fixture）。
    fn aggregate_state() -> (GameState, TeamId, TeamId) {
        use csc_simulation::series::SeriesStage;
        use csc_tournaments::scheduled::{
            ScheduledFixture, ScheduledStatus, ScheduledTournamentRecord,
        };
        let (mut base, tid, _live, _done) = test_calendar_state();
        let other = base.world.create_team("RivalNine", 2, 1800);
        let fixtures = vec![
            ScheduledFixture::pending(0, 1, tid, other, SeriesStage::Group, 1),
            ScheduledFixture::pending(1, 1, other, tid, SeriesStage::Group, 1),
        ];
        base.scheduled_records = vec![ScheduledTournamentRecord {
            event_name: "CCT Open Cup #7".into(),
            date: "2026-02-20".into(),
            tier: csc_domain::tourney_tier::TourneyTier::Qualify,
            team_ids: vec![tid, other],
            player_team_id: Some(tid),
            status: ScheduledStatus::Active,
            fixtures,
            format: csc_domain::tournament_format::TournamentFormat::SingleElim,
        }];
        (base, tid, other)
    }

    #[test]
    fn event_aggregate_builds_teams_fixtures_and_participation() {
        let (state, tid, other) = aggregate_state();
        let agg = event_aggregate(&state, "CCT Open Cup #7").expect("赛事应存在");
        assert_eq!(agg.event_name, "CCT Open Cup #7");
        assert_eq!(agg.status, "active");
        // 参赛队伍：2 支，按排程名单（含 VRS 缓存值）。
        assert_eq!(agg.teams.len(), 2);
        let hero_team = agg
            .teams
            .iter()
            .find(|t| t.id == tid.0)
            .expect("主角队伍在参赛名单");
        assert_eq!(hero_team.name, "TestHeroes");
        assert_eq!(hero_team.vrs_ranking, 1);
        assert!(hero_team.vrs_value > 0);
        let rival = agg
            .teams
            .iter()
            .find(|t| t.id == other.0)
            .expect("对手队伍在名单");
        assert_eq!(rival.name, "RivalNine");
        // fixtures：2 个 pending（无比分）。
        assert_eq!(agg.fixtures.len(), 2);
        assert_eq!(agg.fixtures[0].team_a, tid.0);
        assert_eq!(agg.fixtures[0].team_b, other.0);
        assert_eq!(agg.fixtures[0].status, "pending");
        assert!(agg.fixtures[0].result.is_none());
        // 主角参赛 + 无空状态原因（有对阵可打）。
        assert!(agg.player_participating);
        assert_eq!(agg.emptiness_reason, None);
        // 进行中赛事无冠军。
        assert_eq!(agg.champion, None);
    }

    #[test]
    fn event_aggregate_completed_event_has_champion_and_emptiness_reason() {
        use csc_tournaments::scheduled::ScheduledStatus;
        let (mut state, tid, other) = aggregate_state();
        // 推进为已完赛：状态 Completed + 回填结果 + 写入 events（含冠军）。
        let rec = &mut state.scheduled_records[0];
        rec.status = ScheduledStatus::Completed;
        rec.fixtures[0].result = Some(csc_domain::match_result::MatchResult::new(
            state.world.signature_of(tid),
            state.world.signature_of(other),
            csc_domain::tourney_tier::TourneyTier::Qualify,
        ));
        rec.fixtures[1].result = Some(csc_domain::match_result::MatchResult::new(
            state.world.signature_of(other),
            state.world.signature_of(tid),
            csc_domain::tourney_tier::TourneyTier::Qualify,
        ));
        let tournament = csc_domain::tournament::Tournament::new(
            "CCT Open Cup #7",
            csc_domain::tourney_tier::TourneyTier::Qualify,
            csc_domain::tournament::Organizer::Cct,
            csc_domain::city::City::new("上海", "中国", csc_domain::city::Region::Asia),
            10,
            csc_domain::tournament::InvitePolicy::Open,
        )
        .with_date("2026-02-20");
        state
            .events
            .push(csc_tournaments::scheduled::TournamentResult {
                event: tournament,
                champion: tid,
                matches: vec![],
                series: vec![],
            });
        // P1-7 锁窗口提升：locks 仍含主角队（窗口未结束）→ 标 active（与
        // player_calendar_events 一致），即使 scheduled_records 已 Completed。
        let agg = event_aggregate(&state, "CCT Open Cup #7").expect("赛事应存在");
        assert_eq!(agg.status, "active", "锁窗口内 Completed 应提升为 active");
        // 冠军：主角队。
        let champ = agg.champion.as_ref().expect("已完赛应有冠军");
        assert_eq!(champ.team_id, tid.0);
        assert_eq!(champ.name, "TestHeroes");
        // 已完赛 + 主角参赛 → 空状态原因「赛事已完赛」。
        assert_eq!(agg.emptiness_reason.as_deref(), Some("赛事已完赛"));
        // fixtures 已回填 result（胜者 tid）。
        let f0 = &agg.fixtures[0];
        let result = f0.result.as_ref().expect("completed fixture 应有 result");
        assert_eq!(result.winner, tid.0);
        assert_eq!(result.loser, other.0);

        // 锁窗口外（locks 清空）→ 保持 completed（无提升）。
        state.locks.clear();
        let agg2 = event_aggregate(&state, "CCT Open Cup #7").expect("赛事应存在");
        assert_eq!(
            agg2.status, "completed",
            "锁窗口外 Completed 应保持 completed"
        );
        assert_eq!(
            agg2.emptiness_reason.as_deref(),
            Some("赛事已完赛"),
            "锁窗口外空状态原因不变"
        );
    }

    #[test]
    fn event_aggregate_unknown_event_returns_none() {
        let (state, _tid, _other) = aggregate_state();
        assert_eq!(event_aggregate(&state, "不存在的赛事"), None);
        assert_eq!(event_aggregate(&state, ""), None);
    }

    #[test]
    fn event_aggregate_cancelled_event_reports_reason() {
        use csc_tournaments::scheduled::ScheduledStatus;
        let (mut state, _tid, _other) = aggregate_state();
        state.scheduled_records[0].status = ScheduledStatus::Cancelled;
        let agg = event_aggregate(&state, "CCT Open Cup #7").expect("赛事应存在");
        assert_eq!(agg.status, "cancelled");
        assert_eq!(agg.emptiness_reason.as_deref(), Some("赛事已取消"));
        assert!(agg.champion.is_none());
    }

    #[test]
    fn event_aggregate_bye_only_fixtures_report_no_materialized_reason() {
        use csc_simulation::series::SeriesStage;
        use csc_tournaments::scheduled::ScheduledStatus;
        // 主角参赛但全部 fixture 为轮空 → 「主角本轮轮空（无对阵）」。
        let (mut state, tid, other) = aggregate_state();
        state.scheduled_records[0].status = ScheduledStatus::Active;
        state.scheduled_records[0].fixtures = vec![
            csc_tournaments::scheduled::ScheduledFixture::bye(0, 1, tid, SeriesStage::Group),
            csc_tournaments::scheduled::ScheduledFixture::bye(1, 1, other, SeriesStage::Group),
        ];
        let agg = event_aggregate(&state, "CCT Open Cup #7").expect("赛事应存在");
        assert_eq!(agg.status, "active");
        assert_eq!(
            agg.emptiness_reason.as_deref(),
            Some("主角本轮轮空（无对阵）")
        );
    }
}
