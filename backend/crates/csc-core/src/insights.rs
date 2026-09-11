//! 生涯洞察（结果解释层）——所有解释都由当前 world / 赛事结果的规则推导，
//! 前端只负责展示，不重复实现模拟公式。

use serde::Serialize;

use csc_entities::power::PowerCalculator;
use csc_simulation::rating::RatingCalculator;
use csc_tournaments::scheduled::TournamentResult;
use csc_util::id::PlayerId;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchInsight {
    pub event_name: String,
    pub date: String,
    pub won: bool,
    pub maps_won: usize,
    pub maps_lost: usize,
    pub kills: i32,
    pub deaths: i32,
    pub rating: Option<f64>,
    pub opponent_name: String,
    pub opponent_kills: i32,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TeamStatusInsight {
    pub role: String,
    pub power_rank: usize,
    pub roster_size: usize,
    pub fatigue: f64,
    pub injured: bool,
    pub explanation: String,
}

fn rounds_of_map(team_a_score: i32, team_b_score: i32) -> i32 {
    // RatingCalculator::rating_of_counts 使用整张地图的总回合数，
    // 不是获胜方的分数；加时也必须保留双方回合。
    team_a_score + team_b_score
}

fn player_rating(kills: i32, deaths: i32, assists: i32, rounds: i32) -> Option<f64> {
    (rounds > 0).then(|| RatingCalculator::rating_of_counts(kills, deaths, assists, rounds))
}

/// 最近一场主角参与的系列赛解释。
pub fn match_insight(
    world: &csc_entities::world::World,
    events: &[TournamentResult],
    player_id: PlayerId,
    text: &csc_text::TextBundle,
) -> Option<MatchInsight> {
    let player = world.player(player_id)?;
    let team_id = player.team?;
    let team_sig = world.signature_of(team_id);
    for ev in events.iter().rev() {
        let Some(series) = ev
            .series
            .iter()
            .rev()
            .find(|s| s.team_a_id == team_id || s.team_b_id == team_id)
        else {
            continue;
        };
        let mut kills = 0;
        let mut deaths = 0;
        let mut rating_sum = 0.0;
        let mut rating_maps = 0usize;
        let mut maps_won = 0usize;
        let mut maps_lost = 0usize;
        let opponent_sig = if series.team_a_id == team_id {
            &series.team_b_sig
        } else {
            &series.team_a_sig
        };
        let mut opponent_kills = 0;
        // 对手核心 Rating：对手阵容逐图单行评级取最高（无单行数据时退 1.0 中性基准）。
        let mut opp_core_rating = 1.0f64;
        let mut opponent_name = opponent_sig.split('|').next().unwrap_or("对手").to_string();
        for map in &series.maps {
            if map.winner_sig == team_sig {
                maps_won += 1;
            } else {
                maps_lost += 1;
            }
            let rounds = rounds_of_map(map.team_a_score, map.team_b_score);
            for line in &map.lines {
                if line.player_name == player.name {
                    kills += line.kills;
                    deaths += line.deaths;
                    if let Some(r) = player_rating(line.kills, line.deaths, line.assists, rounds) {
                        rating_sum += r;
                        rating_maps += 1;
                    }
                } else if line.team_sig == *opponent_sig {
                    if opponent_name == opponent_sig.split('|').next().unwrap_or("对手") {
                        opponent_name = line.player_name.clone();
                    }
                    opponent_kills += line.kills;
                    if let Some(r) = player_rating(line.kills, line.deaths, line.assists, rounds) {
                        opp_core_rating = opp_core_rating.max(r);
                    }
                }
            }
        }
        let rating = (rating_maps > 0).then(|| rating_sum / rating_maps as f64);
        let won = maps_won > maps_lost;
        let own_rating = rating.unwrap_or(0.0);
        let rating_txt = rating
            .map(|r| format!("{r:.2}"))
            .unwrap_or_else(|| text.get("insight.rating_dash").to_string());
        let explanation = if won {
            text.format(
                "insight.win",
                &[
                    &ev.event.nickname,
                    &maps_won.to_string(),
                    &maps_lost.to_string(),
                    &rating_txt,
                    &kills.to_string(),
                    &deaths.to_string(),
                ],
            )
        } else if own_rating + 0.1 < opp_core_rating {
            text.format("insight.lose.self", &[&rating_txt, &opponent_name])
        } else {
            text.format(
                "insight.lose.team",
                &[&rating_txt, &maps_won.to_string(), &maps_lost.to_string()],
            )
        };
        return Some(MatchInsight {
            event_name: ev.event.name.clone(),
            date: ev.event.date.clone(),
            won,
            maps_won,
            maps_lost,
            kills,
            deaths,
            rating,
            opponent_name,
            opponent_kills,
            explanation,
        });
    }
    None
}

/// 当前队内地位解释（是否面临首发竞争 / 伤病 / 疲劳）。
pub fn team_status_insight(
    world: &csc_entities::world::World,
    player_id: PlayerId,
    text: &csc_text::TextBundle,
) -> Option<TeamStatusInsight> {
    let player = world.player(player_id)?;
    let team_id = player.team?;
    let team = world.team(team_id)?;
    let mut powers: Vec<(PlayerId, f64)> = team
        .roster_ids
        .iter()
        .filter_map(|pid| world.player(*pid))
        .filter(|p| !p.retired)
        .map(|p| (p.id, PowerCalculator::player_power(p)))
        .collect();
    powers.sort_by(|a, b| b.1.total_cmp(&a.1));
    let rank = powers
        .iter()
        .position(|(pid, _)| *pid == player_id)
        .unwrap_or(0)
        + 1;
    let explanation = if let Some(injury) = &player.injury {
        text.format("insight.injured", &[&injury.days_left.to_string()])
    } else if rank >= 4 {
        text.format(
            "insight.bench_warning",
            &[&rank.to_string(), &powers.len().to_string()],
        )
    } else if player.fatigue > 70.0 {
        text.format(
            "insight.fatigue",
            &[&rank.to_string(), &format!("{:.0}", player.fatigue)],
        )
    } else {
        text.format(
            "insight.steady",
            &[&rank.to_string(), &powers.len().to_string()],
        )
    };
    Some(TeamStatusInsight {
        role: format!("{:?}", player.role),
        power_rank: rank,
        roster_size: powers.len(),
        fatigue: player.fatigue,
        injured: player.injury.is_some(),
        explanation,
    })
}
