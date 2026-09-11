//! 比赛指挥（Match Conductor，Kotlin `MatchConductor.kt` 转写）——
//! 从赛事引擎拆出的**比赛级决策循环**。
//!
//! 职责：对局级 LOD 分派 + 玩家在场时的图间决策循环（场内干预 / 队友失误）。
//!
//! 设计要点：
//! - **场内干预只由玩家的决策决定**：无玩家队伍的对局走粗略路径
//!   （[`run_quick_series`]），不产生任何决策点；
//! - 模拟器保持纯函数（`MatchSimulator::simulate_map`），决策控制流在本模块；
//! - 结算（生涯/疲劳/日志/VRS 积分）委托 [`crate::settlement::SeriesSettlement`]；
//! - 全部比赛级决策经 [`DecisionRecorder::record`] 记录（决策日志 + 事件镜像）。
//!
//! 转写差异：Kotlin 类持 `SimClock`/`DecisionSource`/`DecisionLog`/`WorldJournal`/
//! `ChemistryEngine`/`SeriesSettlement` 引用 → Rust 为**自由函数**（依赖全部
//! 参数化，消除借用冲突）；结算层状态（YearlyRatingTracker）在 settlement 中。

use csc_decision::log::DecisionLog;
use csc_decision::point::{DecisionPoint, InterventionOption, PlayerDecision};
use csc_decision::recorder::DecisionRecorder;
use csc_decision::source::DecisionSource;
use csc_domain::event_importance::EventImportance;
use csc_domain::live::{LiveDecisionFeedback, LiveMatchState, MatchOutcomeAnalysis};
use csc_domain::tier_profile::tier_importance;
use csc_domain::tourney_tier::TourneyTier;
use csc_entities::mark::{CareerMarkType, CareerMarks};
use csc_entities::power::PowerCalculator;
use csc_entities::world::World;
use csc_events::journal::WorldJournal;
use csc_simulation::chemistry_model::ChemistryModel;
use csc_simulation::directives::{MatchDirectives, Playstyle};
use csc_simulation::mark_effects::MarkEffects;
use csc_simulation::match_simulator::{MatchSimulator, SeriesTeam};
use csc_simulation::series::{MapScore, SeriesResult, SeriesStage};
use csc_systems::chemistry::ChemistryEngine;
use csc_time::clock::SimClock;
use csc_util::id::TeamId;
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::settlement::SeriesSettlement;

/// 对局级 LOD 结算入口：任一方队伍含玩家 → 精确 + **图间决策循环**
/// （[`run_player_series`]：每图前场内干预决策、赛后队友失误检测）；
/// 否则 → 粗略（[`run_quick_series`]，仅胜负与比分）。
#[allow(clippy::too_many_arguments)]
pub fn run_lod_series(
    world: &mut World,
    vrs: &mut VrsEngine,
    clock: &SimClock,
    decision: &mut dyn DecisionSource,
    decision_log: &mut DecisionLog,
    journal: &mut WorldJournal,
    settlement: &mut SeriesSettlement,
    tier: TourneyTier,
    team_a: TeamId,
    team_b: TeamId,
    best_of: i32,
    rng: &mut Xoshiro256StarStar,
    event_name: &str,
    stage: SeriesStage,
) -> SeriesResult {
    run_lod_series_with_importance(
        world,
        vrs,
        clock,
        decision,
        decision_log,
        journal,
        settlement,
        tier,
        team_a,
        team_b,
        best_of,
        rng,
        event_name,
        stage,
        tier_importance(tier),
    )
}

/// 赛事体验层入口。Background/Important 仍完整结算，但不会产生 LIVE 决策点。
#[allow(clippy::too_many_arguments)]
pub fn run_lod_series_with_importance(
    world: &mut World,
    vrs: &mut VrsEngine,
    clock: &SimClock,
    decision: &mut dyn DecisionSource,
    decision_log: &mut DecisionLog,
    journal: &mut WorldJournal,
    settlement: &mut SeriesSettlement,
    tier: TourneyTier,
    team_a: TeamId,
    team_b: TeamId,
    best_of: i32,
    rng: &mut Xoshiro256StarStar,
    event_name: &str,
    stage: SeriesStage,
    importance: EventImportance,
) -> SeriesResult {
    let has_player = |world: &World, id: TeamId| {
        world
            .team(id)
            .map(|t| {
                t.roster_ids
                    .iter()
                    .any(|pid| world.player(*pid).map(|p| p.is_player()).unwrap_or(false))
            })
            .unwrap_or(false)
    };
    if has_player(world, team_a) || has_player(world, team_b) {
        run_player_series(
            world,
            vrs,
            clock,
            decision,
            decision_log,
            journal,
            settlement,
            tier,
            team_a,
            team_b,
            best_of,
            rng,
            event_name,
            stage,
            importance,
        )
    } else {
        run_quick_series(
            world, vrs, clock, journal, settlement, tier, team_a, team_b, best_of, rng, event_name,
            stage,
        )
    }
}

/// **图间决策循环**（玩家队伍在场的精确路径）：逐图推进，每图前产出
/// 场内干预决策点（`DecisionPoint::MatchIntervention`）→ 决策应用为
/// `MatchDirectives`（风格/暂停/印记加成）→ 模拟本图 → 赛后检测队友失误
/// （[`handle_blunders`]）→ 决策反应（鼓励/指责/无视）塑造队内关系。
#[allow(clippy::too_many_arguments)]
fn run_player_series(
    world: &mut World,
    vrs: &mut VrsEngine,
    clock: &SimClock,
    decision: &mut dyn DecisionSource,
    decision_log: &mut DecisionLog,
    journal: &mut WorldJournal,
    settlement: &mut SeriesSettlement,
    tier: TourneyTier,
    team_a: TeamId,
    team_b: TeamId,
    best_of: i32,
    rng: &mut Xoshiro256StarStar,
    event_name: &str,
    stage: SeriesStage,
    importance: EventImportance,
) -> SeriesResult {
    let player_of =
        |world: &World, id: TeamId| -> Option<(csc_util::id::PlayerId, String, String)> {
            // (player_id, player_name, team_signature)
            world
                .team(id)
                .and_then(|t| {
                    t.roster_ids
                        .iter()
                        .find(|pid| world.player(**pid).map(|p| p.is_player()).unwrap_or(false))
                        .copied()
                })
                .and_then(|pid| {
                    world
                        .player(pid)
                        .map(|p| (pid, p.name.clone(), world.signature_of(id)))
                })
        };

    let mut maps: Vec<MapScore> = Vec::new();
    let (mut a_wins, mut b_wins) = (0i32, 0i32);
    let target = best_of / 2 + 1;
    let mut dir_a: Option<MatchDirectives> = None;
    let mut dir_b: Option<MatchDirectives> = None;
    let mut feedback: Vec<LiveDecisionFeedback> = Vec::new();
    let mut live_states: Vec<LiveMatchState> = Vec::new();
    let mut state_a = live_state_of(world, team_a, 0, 0);
    let mut state_b = live_state_of(world, team_b, 0, 0);
    let date = clock.date_label();
    let player_a = player_of(world, team_a);
    let player_b = player_of(world, team_b);

    // 只有顶级赛事才开放现场决策。决策按“系列赛阶段”而不是按地图/回合堆叠：
    // 首图前给一次赛前方案，若系列赛进入决胜局再给一次关键调整窗口。
    // 低级别赛事完整结算并可回放，但不以密集弹窗打断职业生涯节奏。
    let interactive = importance.is_live();

    // 组装赛前参赛队伍引用（签名 + roster 快照）
    let (sig_a, sig_b) = (world.signature_of(team_a), world.signature_of(team_b));

    while a_wins < target && b_wins < target {
        let map_no = maps.len() as i32 + 1;
        // 1. LIVE 阶段节点：整个系列赛最多两次（开局 / 决胜局）。
        if interactive {
            for node in live_nodes(map_no, a_wins, b_wins, target, state_a, state_b) {
                let mut points: Vec<DecisionPoint> = Vec::new();
                if let Some(p) = player_a.as_ref().and_then(|(pid, name, sig)| {
                    intervention_point(
                        world,
                        *pid,
                        name,
                        sig,
                        event_name,
                        map_no,
                        &format!("{a_wins}:{b_wins}"),
                        &date,
                        node,
                    )
                }) {
                    points.push(p);
                }
                if let Some(p) = player_b.as_ref().and_then(|(pid, name, sig)| {
                    intervention_point(
                        world,
                        *pid,
                        name,
                        sig,
                        event_name,
                        map_no,
                        &format!("{a_wins}:{b_wins}"),
                        &date,
                        node,
                    )
                }) {
                    points.push(p);
                }
                if points.is_empty() {
                    continue;
                }
                let decisions = decide(decision, decision_log, journal, &date, &points);
                for chosen in decisions {
                    let Some(point) = points.iter().find(|p| p.id() == chosen.point_id) else {
                        continue;
                    };
                    let DecisionPoint::MatchIntervention {
                        options, player_id, ..
                    } = point
                    else {
                        continue;
                    };
                    let Some(option) = options.iter().find(|o| o.id == chosen.option_id) else {
                        continue;
                    };
                    let (state, directives, team_id) =
                        if player_a.as_ref().map(|(pid, _, _)| *pid) == Some(*player_id) {
                            (&mut state_a, &mut dir_a, team_a)
                        } else {
                            (&mut state_b, &mut dir_b, team_b)
                        };
                    let before = *state;
                    apply_live_decision(state, chosen.option_id.as_str());
                    *directives = Some(directive_from_state(option.directive, *state));
                    feedback.push(feedback_for(
                        chosen.option_id.as_str(),
                        before,
                        *state,
                        node,
                    ));
                    live_states.push(*state);
                    apply_style_mark(world, team_id, clock, chosen.option_id.as_str(), point.id());
                }
            }
        }

        // 2. 模拟本图（指令应用给对应队伍）
        let ta = SeriesTeam {
            id: team_a,
            signature: sig_a.clone(),
            roster: world.roster(team_a),
            cohesion: ChemistryEngine::cohesion_factor_of(world, team_a),
        };
        let tb = SeriesTeam {
            id: team_b,
            signature: sig_b.clone(),
            roster: world.roster(team_b),
            cohesion: ChemistryEngine::cohesion_factor_of(world, team_b),
        };
        let map = MatchSimulator::simulate_map(map_no, tier, &ta, &tb, rng, dir_a, dir_b);
        // 释放 roster 借用
        drop(ta);
        drop(tb);
        maps.push(map.clone());
        if map.winner_sig == sig_a {
            a_wins += 1;
        } else {
            b_wins += 1;
        }

        // 3. 队友失误只在决定系列赛的最终地图后评估一次，避免每张图、每位队友
        // 都打断玩家。它仍保留为赛后团队管理事件，而非比赛中反复弹出的操作。
        let series_ended = a_wins >= target || b_wins >= target;
        if interactive && series_ended {
            if let Some((_, _, sig)) = &player_a {
                handle_blunders(
                    world,
                    clock,
                    decision,
                    decision_log,
                    journal,
                    rng,
                    &map,
                    team_a,
                    sig,
                    event_name,
                    &date,
                );
            }
            if let Some((_, _, sig)) = &player_b {
                handle_blunders(
                    world,
                    clock,
                    decision,
                    decision_log,
                    journal,
                    rng,
                    &map,
                    team_b,
                    sig,
                    event_name,
                    &date,
                );
            }
        }
    }

    let winner_sig = if a_wins > b_wins {
        sig_a.clone()
    } else {
        sig_b.clone()
    };
    let loser_sig = if a_wins > b_wins {
        sig_b.clone()
    } else {
        sig_a.clone()
    };
    let outcome_analysis = interactive.then(|| {
        outcome_analysis(
            &maps,
            &sig_a,
            &sig_b,
            player_a.as_ref().map(|(pid, _, _)| *pid),
            world,
            state_a,
            state_b,
            &feedback,
            player_a.is_some(),
            player_a.is_some_and(|(_, _, ref sig)| winner_sig == *sig)
                || player_b.is_some_and(|(_, _, ref sig)| winner_sig == *sig),
        )
    });
    let series = SeriesResult {
        tier,
        team_a_id: team_a,
        team_b_id: team_b,
        team_a_sig: sig_a,
        team_b_sig: sig_b,
        best_of,
        maps,
        winner_sig,
        loser_sig,
        stage,
        replayable: true, // 玩家参与过的对局：可回放
        live_feedback: feedback,
        live_states,
        outcome_analysis,
    };
    settlement.settle_series(world, vrs, clock, journal, &series, event_name);
    series
}

/// 粗略路径：只出系列赛胜负与逐图比分（无 10 人 KDA，不写生涯），VRS 照常结算。
#[allow(clippy::too_many_arguments)]
fn run_quick_series(
    world: &mut World,
    vrs: &mut VrsEngine,
    clock: &SimClock,
    journal: &mut WorldJournal,
    settlement: &mut SeriesSettlement,
    tier: TourneyTier,
    team_a: TeamId,
    team_b: TeamId,
    best_of: i32,
    rng: &mut Xoshiro256StarStar,
    event_name: &str,
    stage: SeriesStage,
) -> SeriesResult {
    let sig_a = world.signature_of(team_a);
    let sig_b = world.signature_of(team_b);
    let ta = SeriesTeam {
        id: team_a,
        signature: sig_a,
        roster: world.roster(team_a),
        cohesion: ChemistryEngine::cohesion_factor_of(world, team_a),
    };
    let tb = SeriesTeam {
        id: team_b,
        signature: sig_b,
        roster: world.roster(team_b),
        cohesion: ChemistryEngine::cohesion_factor_of(world, team_b),
    };
    let series = MatchSimulator::simulate_quick_series(tier, ta, tb, best_of, rng, stage);
    settlement.settle_quick_series(world, vrs, clock, journal, &series, event_name);
    series
}

#[derive(Debug, Clone, Copy)]
enum LiveNode {
    Preparation,
    Decider,
}

fn live_nodes(
    map_no: i32,
    a_wins: i32,
    b_wins: i32,
    target: i32,
    _state_a: LiveMatchState,
    _state_b: LiveMatchState,
) -> Vec<LiveNode> {
    if map_no == 1 {
        return vec![LiveNode::Preparation];
    }
    // 系列赛双方均已各拿 target-1 图时，下一图是明确的决胜阶段；仅此再打断一次。
    // BO3（target=2）：1:1 → 第 3 图；BO5（target=3）：2:2 → 第 5 图。
    if a_wins == target - 1 && b_wins == target - 1 {
        return vec![LiveNode::Decider];
    }
    Vec::new()
}

fn live_state_of(
    world: &World,
    team_id: TeamId,
    series_lost: i32,
    rounds_lost: i32,
) -> LiveMatchState {
    let roster = world.roster(team_id);
    let count = roster.len().max(1) as f64;
    let avg = |f: fn(&csc_entities::character::PlayerCharacter) -> f64| {
        roster.iter().map(|player| f(player)).sum::<f64>() / count
    };
    let fatigue = avg(|p| p.fatigue);
    let morale = avg(|p| p.pro.morale as f64);
    let confidence = avg(|p| p.pro.confidence as f64);
    let chemistry = ChemistryEngine::cohesion_factor_of(world, team_id) * 50.0;
    LiveMatchState {
        form: ((morale + confidence) / 2.0 - fatigue * 0.25).clamp(0.0, 100.0),
        fatigue,
        confidence,
        team_morale: morale,
        chemistry: chemistry.clamp(0.0, 100.0),
        map_preparation: (55.0 + chemistry * 0.2 - fatigue * 0.15).clamp(0.0, 100.0),
        rounds_lost,
        momentum: (55.0 - series_lost as f64 * 8.0 - rounds_lost as f64 * 3.0).clamp(0.0, 100.0),
        round_stability: (50.0 + chemistry * 0.25 - fatigue * 0.2).clamp(0.0, 100.0),
        economy_pressure: (rounds_lost as f64 * 8.0 + fatigue * 0.2).clamp(0.0, 100.0),
    }
}

fn apply_live_decision(state: &mut LiveMatchState, option_id: &str) {
    match option_id {
        "AGGRESSIVE" => {
            state.momentum = (state.momentum + 6.0).min(100.0);
            state.confidence = (state.confidence + 3.0).min(100.0);
            state.round_stability = (state.round_stability - 3.0).max(0.0);
            state.economy_pressure = (state.economy_pressure + 5.0).min(100.0);
        }
        "CONSERVATIVE" => {
            state.round_stability = (state.round_stability + 6.0).min(100.0);
            state.economy_pressure = (state.economy_pressure - 5.0).max(0.0);
            state.momentum = (state.momentum - 2.0).max(0.0);
        }
        "TIMEOUT" | "TACTICAL" => {
            state.team_morale = (state.team_morale + 8.0).min(100.0);
            state.momentum = (state.momentum + 5.0).min(100.0);
            state.round_stability = (state.round_stability + 6.0).min(100.0);
        }
        "ENCOURAGE" => {
            state.team_morale = (state.team_morale + 7.0).min(100.0);
            state.chemistry = (state.chemistry + 4.0).min(100.0);
            state.confidence = (state.confidence + 4.0).min(100.0);
        }
        "CRITICIZE" => {
            state.team_morale = (state.team_morale - 6.0).max(0.0);
            state.chemistry = (state.chemistry - 5.0).max(0.0);
            state.round_stability = (state.round_stability + 2.0).min(100.0);
        }
        _ => {}
    }
}

fn directive_from_state(base: MatchDirectives, state: LiveMatchState) -> MatchDirectives {
    // 状态只提供有限的表现分布偏移；随机和对手实力仍决定最终地图结果。
    let state_effect = ((state.momentum - 50.0) * 0.04
        + (state.round_stability - 50.0) * 0.03
        + (state.team_morale - 50.0) * 0.02
        - state.economy_pressure * 0.02)
        .round() as i32;
    MatchDirectives {
        style: base.style,
        bonus: base.bonus + state_effect.clamp(-3, 3),
    }
}

fn feedback_for(
    option_id: &str,
    before: LiveMatchState,
    after: LiveMatchState,
    node: LiveNode,
) -> LiveDecisionFeedback {
    let phase = match node {
        LiveNode::Preparation => "Map preparation",
        LiveNode::Decider => "Deciding phase",
    };
    let affected_state = vec![
        format!(
            "Team morale: {:.0} -> {:.0}",
            before.team_morale, after.team_morale
        ),
        format!("Momentum: {:.0} -> {:.0}", before.momentum, after.momentum),
        format!(
            "Round stability: {:.0} -> {:.0}",
            before.round_stability, after.round_stability
        ),
    ];
    let narrative = match option_id {
        "TIMEOUT" => "暂停切断了连败节奏，队伍重新统一了下一阶段执行。",
        "AGGRESSIVE" => "队伍提高首杀争夺强度，扩大上限，同时承担更多经济风险。",
        "CONSERVATIVE" => "队伍收紧防守和残局纪律，降低波动并保护经济。",
        "TACTICAL" => "战术重置让队伍更稳定地执行下一阶段计划。",
        "ENCOURAGE" => "你稳住了队友的情绪，沟通和信心有所恢复。",
        "CRITICIZE" => "压力被直接指出，纪律提高，但队内气氛承受代价。",
        _ => "队伍维持既定方案，等待比赛自身给出回应。",
    };
    LiveDecisionFeedback {
        decision_id: option_id.to_string(),
        decision: option_id.to_string(),
        immediate_effect: phase.to_string(),
        affected_state: affected_state.clone(),
        narrative: narrative.to_string(),
        next_state: "下一阶段的执行稳定性和风险已更新；结果仍由比赛模拟决定。".into(),
        headline: phase.to_string(),
        what_happened: narrative.to_string(),
        affected_metrics: affected_state,
        explanation: "决策改变了球队状态和表现分布，不保证下一回合或地图获胜。".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn outcome_analysis(
    maps: &[MapScore],
    _team_a_sig: &str,
    _team_b_sig: &str,
    player_id: Option<csc_util::id::PlayerId>,
    world: &World,
    state_a: LiveMatchState,
    state_b: LiveMatchState,
    feedback: &[LiveDecisionFeedback],
    player_on_team_a: bool,
    player_team_won: bool,
) -> MatchOutcomeAnalysis {
    let player_name = player_id
        .and_then(|id| world.player(id))
        .map(|p| p.name.as_str());
    let player_performance = player_name.map_or(0.0, |name| {
        let lines: Vec<_> = maps
            .iter()
            .flat_map(|map| map.lines.iter())
            .filter(|line| line.player_name == name)
            .collect();
        if lines.is_empty() {
            0.0
        } else {
            lines
                .iter()
                .map(|line| line.adr / 10.0 + line.kast / 10.0)
                .sum::<f64>()
                / lines.len() as f64
                - 10.0
        }
    });
    let score_delta: i32 = maps
        .iter()
        .map(|map| map.team_a_score - map.team_b_score)
        .sum();
    let (own, opponent) = if player_on_team_a {
        (state_a, state_b)
    } else {
        (state_b, state_a)
    };
    let tactical_impact =
        feedback.len() as f64 * 0.6 + (own.round_stability - opponent.round_stability) * 0.08;
    let key_moments = feedback
        .iter()
        .take(3)
        .map(|f| f.narrative.clone())
        .collect();
    let team_strength = (score_delta.abs() as f64 * 0.35 + own.form * 0.04).min(10.0);
    MatchOutcomeAnalysis {
        player_performance,
        team_strength,
        form: own.form - opponent.form,
        chemistry: own.chemistry - opponent.chemistry,
        map_preparation: own.map_preparation - opponent.map_preparation,
        tactical_impact,
        opponent_performance: opponent.form * 0.1,
        key_moments,
        won: player_team_won,
    }
}

/// 场内操作打 flag：干预决策（非默认）累积生涯印记——
/// 激进攻防/保守打法/战术领袖（多次选择后成为打法基线，影响深远）。
fn apply_style_mark(
    world: &mut World,
    team_id: TeamId,
    clock: &SimClock,
    option_id: &str,
    point_id: &str,
) {
    let year = clock.year();
    let Some(pid) = world.team(team_id).and_then(|t| {
        t.roster_ids
            .iter()
            .find(|pid| world.player(**pid).map(|p| p.is_player()).unwrap_or(false))
            .copied()
    }) else {
        return;
    };
    let Some(career) = world.player_mut(pid).expect("选手不存在").career_mut() else {
        return;
    };
    match option_id {
        "AGGRESSIVE" => CareerMarks::apply(
            &mut career.marks,
            CareerMarkType::AggressivePlaystyle,
            year,
            point_id,
            1,
        ),
        "CONSERVATIVE" => CareerMarks::apply(
            &mut career.marks,
            CareerMarkType::ConservativePlaystyle,
            year,
            point_id,
            1,
        ),
        "TIMEOUT" => CareerMarks::apply(
            &mut career.marks,
            CareerMarkType::TacticalLeader,
            year,
            point_id,
            1,
        ),
        _ => {} // DEFAULT / BALANCED 不累积（维持基线）
    }
}

/// 场内干预决策点（图间暂停）：选项第 0 项 = 玩家当前打法风格基线
/// （由生涯印记 [`MarkEffects::default_playstyle`] 派生），其余为临时调整；
/// 所有选项携带印记的胜率加成（[`MarkEffects::win_rate_modifier`]）。
#[allow(clippy::too_many_arguments)]
fn intervention_point(
    world: &World,
    player_id: csc_util::id::PlayerId,
    player_name: &str,
    team_signature: &str,
    event_name: &str,
    map_no: i32,
    score: &str,
    date: &str,
    node: LiveNode,
) -> Option<DecisionPoint> {
    let pc = world.player(player_id)?;
    let marks = pc
        .career
        .as_ref()
        .map(|c| c.marks.clone())
        .unwrap_or_default();
    let base_bonus = MarkEffects::win_rate_modifier(&marks).round() as i32; // Kotlin `.toInt()`（截断；round 更稳）
    // 玩家主动设定的赛前 BP 优先于生涯印记基线（FIFA 生涯模式：BP 在赛季间选好）
    let planned_style = pc
        .career
        .as_ref()
        .and_then(|c| c.match_style.as_deref())
        .and_then(|s| match s {
            "AGGRESSIVE" => Some(Playstyle::Aggressive),
            "CONSERVATIVE" => Some(Playstyle::Conservative),
            "BALANCED" => Some(Playstyle::Balanced),
            _ => None,
        });
    let default_style = planned_style.unwrap_or_else(|| MarkEffects::default_playstyle(&marks));
    Some(DecisionPoint::MatchIntervention {
        id: format!(
            "{date}|inter|{event_name}|{map_no}|{:?}|{team_signature}",
            node
        ),
        date: date.to_string(),
        player_id,
        player_name: player_name.to_string(),
        event_name: event_name.to_string(),
        map_number: map_no,
        series_score: score.to_string(),
        options: vec![
            InterventionOption {
                id: "DEFAULT".into(),
                directive: MatchDirectives {
                    style: default_style,
                    bonus: base_bonus,
                },
                label: format!("{}（当前风格）", default_style.label()),
                description: "生涯印记决定的打法基线".into(),
            },
            InterventionOption {
                id: "TACTICAL".into(),
                directive: MatchDirectives {
                    style: Playstyle::Balanced,
                    bonus: base_bonus + 2,
                },
                label: "战术调整".into(),
                description: "提高执行稳定性，重置下一阶段的比赛节奏".into(),
            },
            InterventionOption {
                id: "AGGRESSIVE".into(),
                directive: MatchDirectives {
                    style: Playstyle::Aggressive,
                    bonus: base_bonus,
                },
                label: "激进攻防".into(),
                description: "提高首杀争夺与翻盘上限，但会放大波动和经济压力".into(),
            },
            InterventionOption {
                id: "CONSERVATIVE".into(),
                directive: MatchDirectives {
                    style: Playstyle::Conservative,
                    bonus: base_bonus,
                },
                label: "保守稳扎".into(),
                description: "提高残局稳定性并保护经济，但降低主动抢分的上限".into(),
            },
            InterventionOption {
                id: "BALANCED".into(),
                directive: MatchDirectives {
                    style: Playstyle::Balanced,
                    bonus: base_bonus,
                },
                label: "平衡".into(),
                description: "不做风格调整".into(),
            },
            InterventionOption {
                id: "TIMEOUT".into(),
                directive: MatchDirectives {
                    style: Playstyle::Balanced,
                    bonus: base_bonus + 3,
                },
                label: "叫暂停".into(),
                description: "提升士气、协同和下一阶段战术执行稳定性".into(),
            },
        ],
    })
}

/// 队友失误检测（赛后）：玩家队伍内 NPC 单图击杀显著低于预期（按实力占比
/// 的击杀份额）→ 产出 `DecisionPoint::TeammateBlunder` 决策点，玩家的反应
/// 由 [`ChemistryEngine::apply_blunder_reaction`] 应用（关系/士气/印记/冲突）。
#[allow(clippy::too_many_arguments)]
fn handle_blunders(
    world: &mut World,
    clock: &SimClock,
    decision: &mut dyn DecisionSource,
    decision_log: &mut DecisionLog,
    journal: &mut WorldJournal,
    rng: &mut Xoshiro256StarStar,
    map: &MapScore,
    team_id: TeamId,
    team_sig: &str,
    event_name: &str,
    date: &str,
) {
    let Some(player_id) = world.team(team_id).and_then(|t| {
        t.roster_ids
            .iter()
            .find(|pid| world.player(**pid).map(|p| p.is_player()).unwrap_or(false))
            .copied()
    }) else {
        return;
    };
    let player_name = world.player(player_id).expect("选手不存在").name.clone();
    let team_lines: Vec<&csc_simulation::series::PlayerLine> = map
        .lines
        .iter()
        .filter(|l| l.team_sig == team_sig)
        .collect();
    let total_kills: i32 = team_lines.iter().map(|l| l.kills).sum();
    if total_kills < 5 {
        return; // 数据太少不判定
    }
    let roster_powers: Vec<(csc_util::id::PlayerId, f64)> = world
        .team(team_id)
        .map(|t| {
            t.roster_ids
                .iter()
                .filter_map(|pid| {
                    world
                        .player(*pid)
                        .map(|p| (*pid, PowerCalculator::player_power(p)))
                })
                .collect()
        })
        .unwrap_or_default();
    let team_power: f64 = roster_powers.iter().map(|(_, p)| p).sum();
    let npc_ids: Vec<csc_util::id::PlayerId> = world
        .team(team_id)
        .map(|t| {
            t.roster_ids
                .iter()
                .filter(|pid| world.player(**pid).map(|p| p.is_npc()).unwrap_or(false))
                .copied()
                .collect()
        })
        .unwrap_or_default();

    for npc_id in npc_ids {
        let npc_name = world.player(npc_id).expect("选手不存在").name.clone();
        let Some(line) = team_lines.iter().find(|l| l.player_name == npc_name) else {
            continue;
        };
        if line.kills > 1 {
            continue; // 击杀 ≥ 2 不算"犯罪"
        }
        let share = roster_powers
            .iter()
            .find(|(pid, _)| *pid == npc_id)
            .map(|(_, p)| p / team_power)
            .unwrap_or(0.0);
        let expected = share * total_kills as f64;
        if expected < 1.5 || line.kills as f64 >= expected * 0.4 {
            continue; // 预期过低或没差太多 → 放过
        }

        let kind = ChemistryModel::roll_blunder_kind(rng);
        let npc_role = world.player(npc_id).expect("选手不存在").role;
        let detail = format!(
            "本图 {} 杀（预期 ~{}）",
            line.kills,
            expected.round() as i32
        );
        let point = ChemistryEngine::blunder_point(
            date,
            player_id,
            &player_name,
            event_name,
            map.map_number,
            npc_id,
            &npc_name,
            npc_role,
            kind,
            &detail,
        );
        let decisions = decide(
            decision,
            decision_log,
            journal,
            date,
            std::slice::from_ref(&point),
        );
        let Some(d) = decisions.iter().find(|d| d.point_id == point.id()) else {
            continue;
        };
        let reaction = ChemistryModel::reaction_of(&d.option_id);
        let year = clock.year();
        ChemistryEngine::apply_blunder_reaction(
            world,
            player_id,
            npc_id,
            kind,
            reaction,
            team_id,
            year,
            point.id(),
            Some(journal),
        );
        // 一个系列赛只处理一个最先命中的关键失误，避免赛后连续弹出队友反应。
        break;
    }
}

// —— 决策记录（单点）——

/// 比赛级现场决策（决策日志 + 事件镜像，单点实现见 `DecisionRecorder`）。
fn decide(
    decision: &mut dyn DecisionSource,
    decision_log: &mut DecisionLog,
    journal: &mut WorldJournal,
    date: &str,
    points: &[DecisionPoint],
) -> Vec<PlayerDecision> {
    let decisions = decision.decide(points);
    DecisionRecorder::record(decision_log, journal, date, points, &decisions);
    decisions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervention_point_default_derived_from_marks() {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let p = w.create_player(
            csc_domain::tier::Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        w.assign_player_to_team(p, t).unwrap();
        // 无印记 → 默认平衡
        let point = intervention_point(
            &w,
            p,
            "P",
            &w.signature_of(t),
            "EV",
            1,
            "0:0",
            "2026-06-08",
            LiveNode::Preparation,
        )
        .unwrap();
        let DecisionPoint::MatchIntervention {
            options, player_id, ..
        } = &point
        else {
            panic!()
        };
        assert_eq!(*player_id, p, "干预点携带稳定 ID");
        assert_eq!(options[0].id, "DEFAULT");
        assert_eq!(options[0].directive.style, Playstyle::Balanced);
        assert_eq!(options.len(), 6);
        assert_eq!(options[1].id, "TACTICAL");
        assert_eq!(options[5].id, "TIMEOUT");
        assert_eq!(options[5].directive.bonus, 3, "暂停 +3 加成");
    }

    struct CountingDecisionSource {
        option: &'static str,
        calls: usize,
    }

    impl DecisionSource for CountingDecisionSource {
        fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> {
            self.calls += points.len();
            points
                .iter()
                .map(|point| match point {
                    DecisionPoint::MatchIntervention { id, options, .. } => PlayerDecision::new(
                        id,
                        options
                            .iter()
                            .find(|option| option.id == self.option)
                            .or_else(|| options.first())
                            .expect("LIVE options exist")
                            .id
                            .as_str(),
                    ),
                    DecisionPoint::TeammateBlunder { id, .. } => PlayerDecision::new(id, "IGNORE"),
                    _ => csc_decision::source::AutoDecisionSource::default_decision_for(point),
                })
                .collect()
        }
    }

    fn live_fixture() -> (World, TeamId, TeamId) {
        let mut world = World::new();
        let a = world.create_team("Player Team", 1, 2_000);
        let b = world.create_team("Opponent", 2, 1_900);
        let mut player_rng = Xoshiro256StarStar::seed(7);
        let player = world.create_player(
            csc_domain::tier::Tier::Tier1,
            Some("Hero"),
            &mut player_rng,
            None,
            None,
        );
        world.assign_player_to_team(player, a).unwrap();
        for index in 0..4 {
            world
                .create_npc(
                    csc_domain::tier::Tier::Tier1,
                    Some(a),
                    None,
                    Some(&format!("A{index}")),
                    &mut player_rng,
                    None,
                )
                .unwrap();
        }
        for index in 0..5 {
            world
                .create_npc(
                    csc_domain::tier::Tier::Tier1,
                    Some(b),
                    None,
                    Some(&format!("B{index}")),
                    &mut player_rng,
                    None,
                )
                .unwrap();
        }
        assert_eq!(world.roster(a).len(), 5, "active roster stays five players");
        assert_eq!(
            world.roster(b).len(),
            5,
            "opponent active roster stays five players"
        );
        (world, a, b)
    }

    fn run_fixture(
        importance: EventImportance,
        option: &'static str,
        seed: u64,
    ) -> (SeriesResult, usize) {
        let (mut world, a, b) = live_fixture();
        let mut vrs =
            VrsEngine::from_database(csc_vrs::database::VrsDatabase::from_json_files(&[]).unwrap());
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut settlement = SeriesSettlement::default();
        let mut source = CountingDecisionSource { option, calls: 0 };
        let mut rng = Xoshiro256StarStar::seed(seed);
        let series = run_lod_series_with_importance(
            &mut world,
            &mut vrs,
            &clock,
            &mut source,
            &mut log,
            &mut journal,
            &mut settlement,
            TourneyTier::Major,
            a,
            b,
            3,
            &mut rng,
            "Major Test",
            SeriesStage::Quarterfinal,
            importance,
        );
        (series, source.calls)
    }

    /// 与 `run_fixture` 相同，但返回决策日志以便断言现场决策点数量与类型。
    fn run_fixture_log(
        importance: EventImportance,
        best_of: i32,
        seed: u64,
    ) -> (SeriesResult, DecisionLog) {
        let (mut world, a, b) = live_fixture();
        let mut vrs =
            VrsEngine::from_database(csc_vrs::database::VrsDatabase::from_json_files(&[]).unwrap());
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut settlement = SeriesSettlement::default();
        let mut source = CountingDecisionSource {
            option: "BALANCED",
            calls: 0,
        };
        let mut rng = Xoshiro256StarStar::seed(seed);
        let series = run_lod_series_with_importance(
            &mut world,
            &mut vrs,
            &clock,
            &mut source,
            &mut log,
            &mut journal,
            &mut settlement,
            TourneyTier::Major,
            a,
            b,
            best_of,
            &mut rng,
            "P1-5 Test Event",
            SeriesStage::Quarterfinal,
            importance,
        );
        (series, log)
    }

    /// 统计决策日志中某类决策点的条数（按 point_id 标记区分）。
    fn count_decisions(log: &DecisionLog, marker: &str) -> usize {
        log.entries()
            .iter()
            .filter(|d| d.point_id.contains(marker))
            .count()
    }

    #[test]
    fn background_tournament_does_not_enter_live() {
        let (series, calls) = run_fixture(EventImportance::Background, "TIMEOUT", 42);
        assert_eq!(calls, 0);
        assert!(series.live_feedback.is_empty());
        assert!(series.live_states.is_empty());
        assert!(series.outcome_analysis.is_none());
    }

    #[test]
    fn major_tournament_enters_live_with_multiple_interventions() {
        let (series, calls) = run_fixture(EventImportance::Major, "TIMEOUT", 42);
        assert!(
            calls >= 2,
            "LIVE series must offer multiple intervention points"
        );
        assert!(series.live_feedback.len() >= 2);
        assert!(series.live_feedback.len() <= series.maps.len() * 3);
        assert!(series.outcome_analysis.is_some());
        assert_eq!(series.best_of, 3);
    }

    #[test]
    fn live_decisions_are_deterministic_but_do_not_force_wins() {
        let (a, _) = run_fixture(EventImportance::Major, "TIMEOUT", 99);
        let (b, _) = run_fixture(EventImportance::Major, "TIMEOUT", 99);
        assert_eq!(a, b, "same seed and decisions must reproduce exactly");
        let (aggressive, _) = run_fixture(EventImportance::Major, "AGGRESSIVE", 99);
        assert_ne!(
            a.live_states, aggressive.live_states,
            "decision sequence changes match state"
        );
        assert!(
            a.maps.len() >= 2 && a.maps.len() <= 3,
            "BO3 remains governed by series rules"
        );
    }

    #[test]
    fn bo5_and_playoff_stages_are_preserved() {
        let (mut world, a, b) = live_fixture();
        let mut vrs =
            VrsEngine::from_database(csc_vrs::database::VrsDatabase::from_json_files(&[]).unwrap());
        let clock = SimClock::of(2026, 6, 1);
        let mut journal = WorldJournal::default();
        let mut log = DecisionLog::default();
        let mut settlement = SeriesSettlement::default();
        let mut source = CountingDecisionSource {
            option: "BALANCED",
            calls: 0,
        };
        let mut rng = Xoshiro256StarStar::seed(123);
        let series = run_lod_series_with_importance(
            &mut world,
            &mut vrs,
            &clock,
            &mut source,
            &mut log,
            &mut journal,
            &mut settlement,
            TourneyTier::Major,
            a,
            b,
            5,
            &mut rng,
            "Major Final",
            SeriesStage::Final,
            EventImportance::Championship,
        );
        assert_eq!(series.best_of, 5);
        assert_eq!(series.stage, SeriesStage::Final);
        assert!((3..=5).contains(&series.maps.len()));
        assert!(series.outcome_analysis.is_some());
        assert_eq!(world.roster(a).len(), 5);
        assert_eq!(world.roster(b).len(), 5);
    }

    #[test]
    fn live_nodes_are_series_phased_and_bounded() {
        let stable = LiveMatchState {
            form: 60.0,
            fatigue: 10.0,
            confidence: 60.0,
            team_morale: 60.0,
            chemistry: 60.0,
            map_preparation: 60.0,
            rounds_lost: 0,
            momentum: 60.0,
            round_stability: 60.0,
            economy_pressure: 10.0,
        };
        assert!(matches!(
            live_nodes(1, 0, 0, 2, stable, stable).as_slice(),
            [LiveNode::Preparation]
        ));
        assert!(matches!(
            live_nodes(3, 1, 1, 2, stable, stable).as_slice(),
            [LiveNode::Decider]
        ));
        assert!(live_nodes(2, 1, 0, 2, stable, stable).is_empty());
        // BO5（target=3）：仅当双方 2:2 时第 5 图才进入决胜局；2:1 时非决胜阶段。
        assert!(matches!(
            live_nodes(5, 2, 2, 3, stable, stable).as_slice(),
            [LiveNode::Decider]
        ));
        assert!(live_nodes(4, 2, 1, 3, stable, stable).is_empty());
        assert!(live_nodes(3, 1, 1, 3, stable, stable).is_empty());
    }

    #[test]
    fn top_tier_bo3_limits_live_interventions_to_two_and_final_map_blunder_to_one() {
        for seed in [7u64, 42, 99, 123, 2024] {
            let (series, log) = run_fixture_log(EventImportance::Major, 3, seed);
            let interventions = count_decisions(&log, "|inter|");
            let blunders = count_decisions(&log, "|blunder|");
            assert!(
                interventions <= 2,
                "seed {seed}: BO3 MatchIntervention 应最多 2 次（开局/决胜局），实际 {interventions}"
            );
            assert!(
                blunders <= 1,
                "seed {seed}: 最终图 TeammateBlunder 应最多 1 次，实际 {blunders}"
            );
            // 决胜局（decider）应发生在双方各拿一图后；系列赛图数受 best_of 约束。
            assert!((2..=3).contains(&series.maps.len()), "BO3 图数应在 2..=3");
            assert!(series.best_of == 3);
        }
    }

    #[test]
    fn top_tier_bo5_limits_live_interventions_to_two_and_final_map_blunder_to_one() {
        for seed in [7u64, 42, 99, 123, 2024] {
            let (series, log) = run_fixture_log(EventImportance::Championship, 5, seed);
            let interventions = count_decisions(&log, "|inter|");
            let blunders = count_decisions(&log, "|blunder|");
            assert!(
                interventions <= 2,
                "seed {seed}: BO5 MatchIntervention 应最多 2 次（开局/决胜局），实际 {interventions}"
            );
            assert!(
                blunders <= 1,
                "seed {seed}: 最终图 TeammateBlunder 应最多 1 次，实际 {blunders}"
            );
            assert!((3..=5).contains(&series.maps.len()), "BO5 图数应在 3..=5");
            assert!(series.best_of == 5);
        }
    }

    #[test]
    fn low_tier_series_produce_zero_live_decisions() {
        // T2/T3/Qualify 走 EventImportance::Background → is_live()=false → 现场决策点为零。
        for seed in [7u64, 42, 99] {
            let (series, log) = run_fixture_log(EventImportance::Background, 3, seed);
            assert_eq!(
                count_decisions(&log, "|inter|"),
                0,
                "seed {seed}: 低级别赛事不应产生 MatchIntervention"
            );
            assert_eq!(
                count_decisions(&log, "|blunder|"),
                0,
                "seed {seed}: 低级别赛事不应产生 TeammateBlunder"
            );
            // 完整结算但无现场干预：不进入 LIVE。
            assert!(series.live_feedback.is_empty());
            assert!(series.live_states.is_empty());
            assert!(series.outcome_analysis.is_none());
        }
    }

    #[test]
    fn timeout_changes_visible_state_without_guaranteeing_result() {
        let before = LiveMatchState {
            form: 50.0,
            fatigue: 30.0,
            confidence: 45.0,
            team_morale: 50.0,
            chemistry: 50.0,
            map_preparation: 50.0,
            rounds_lost: 3,
            momentum: 42.0,
            round_stability: 45.0,
            economy_pressure: 60.0,
        };
        let mut after = before;
        apply_live_decision(&mut after, "TIMEOUT");
        let feedback = feedback_for("TIMEOUT", before, after, LiveNode::Decider);
        assert!(after.team_morale > before.team_morale);
        assert!(after.momentum > before.momentum);
        assert!(feedback.explanation.contains("不保证"));
    }
}
