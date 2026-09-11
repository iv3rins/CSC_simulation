//! 跨年结算编排（Kotlin `YearlySettlement.kt` 转写）：日历年变化时的年度世界演化。
//!
//! 职责：颁奖 → 生涯归档 → 成长 → 人口 → 薪资 → 代言复位 → 合同结转，
//! **阶段顺序是业务契约**（先颁上年荣誉再成长，先退役再薪资，先归档再清空年度统计）。

use csc_career::archive::{CareerArchive, SeasonRecord};
use csc_domain::tourney_tier::TourneyTier;
use csc_entities::career::{CareerMemory, CareerMemoryKind, IndividualHonourType, PlayerStatus};
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::growth::GrowthModel;
use csc_systems::population::PopulationEngine;
use csc_time::clock::SimClock;
use csc_tournaments::yearly_rating::YearlyStat;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::context::WorldContext;

/// 跨年结算编排：无跨调用状态（全部依赖参数化）。
pub struct YearlySettlement;

impl YearlySettlement {
    /// 每名 NPC 的年度运营成本（经济闭环支出端；NPC 无薪资字段，按人头计运营开销）。
    ///
    /// **2026 修订（经济可持续性）**：原值 10 万 → 3 万。15 赛季长程验证发现
    /// 结构性赤字——每队年支出（5 NPC×10 万 = 50 万+）远超奖池均分（~22.5 万/队/年），
    /// 40 队 15 年后 38+ 队赤字、中位 -285 万，转会市场随预算枯竭而萎缩。
    /// 3 万 × 5 = 15 万/年 < 奖池均分 → 中游队可持续，顶级队奖金充裕养得起高薪。
    pub const NPC_ANNUAL_COST: i64 = 30_000;

    /// 执行一次跨年结算。
    ///
    /// @param prev_year 上一年度（颁奖与归档的归属年份）
    pub fn settle(ctx: &mut WorldContext<'_>, prev_year: i32, rng: &mut Xoshiro256StarStar) {
        let stats = ctx.tournaments.yearly_stats(); // 先采集年度统计（award 会清空累计器）
        ctx.tournaments
            .year_end_awards(ctx.world, ctx.clock, ctx.journal, prev_year, 20); // 颁上年度 TOP20（此时上年数据完整）
        Self::archive_year(ctx.world, ctx.archive, prev_year, &stats); // 生涯档案：逐赛季记录
        Self::update_player_career_layer(
            ctx.world,
            ctx.archive,
            ctx.tournaments,
            prev_year,
            &stats,
        );
        Self::reset_season_goals(ctx.world, prev_year); // 赛季目标年度重置：下赛季重新选择
        Self::apply_yearly_growth(ctx.world, rng); // 全选手年龄 +1 并按成长曲线更新属性（含 28+ 生理衰退）
        Self::apply_population_turnover(
            ctx.world,
            ctx.vrs,
            ctx.clock,
            ctx.journal,
            rng,
            ctx.rating_profile,
        ); // 人口再生：老将退役 + 青训新秀补位
        Self::apply_annual_payroll(ctx.world); // 经济闭环：队伍年度薪资支出
        Self::apply_sponsor_rollover(ctx.world, prev_year); // 代言评估年份复位
        Self::contracts_expire_one_year(ctx.world);
    }

    /// 从本年度已经结算的事实更新职业身份层。只写 CareerInfo 的新字段，
    /// 不参与比赛、Rating、VRS、TOP20 或 NPC 计算。
    fn update_player_career_layer(
        world: &mut World,
        archive: &CareerArchive,
        tournaments: &csc_tournaments::engine::TournamentEngine,
        year: i32,
        stats: &std::collections::HashMap<PlayerId, YearlyStat>,
    ) {
        let player_ids: Vec<PlayerId> = world.all_players_only().iter().map(|p| p.id).collect();
        for pid in player_ids {
            let Some(pc) = world.player(pid).cloned() else {
                continue;
            };
            let Some(career) = pc.career.as_ref() else {
                continue;
            };
            let stat = stats.get(&pid).copied().unwrap_or(YearlyStat {
                rating: 0.0,
                maps: 0,
                all_rating: 0.0,
                maps_all: 0,
                kills: 0,
                deaths: 0,
                assists: 0,
                rounds: 0,
                wins: 0,
            });
            let seasons = archive.seasons_of(pid);
            let tenure = seasons.iter().filter(|s| s.team_name.is_some()).count() as f64;
            let relation = pc
                .team
                .and_then(|tid| world.team(tid).map(|team| (tid, team)))
                .map(|(tid, team)| {
                    let names: Vec<String> =
                        world.roster(tid).iter().map(|p| p.name.clone()).collect();
                    team.chemistry.average_relation(&names)
                })
                .unwrap_or(50.0);
            let leadership = (pc.skill.leader as f64 * 0.45
                + pc.skill.communication as f64 * 0.25
                + pc.pro.mentality as f64 * 0.15
                + pc.pro.team_spirit as f64 * 0.15)
                .clamp(0.0, 100.0);
            let performance = if stat.maps_all > 0 {
                (50.0 + (stat.all_rating - 1.0) * 100.0).clamp(0.0, 100.0)
            } else {
                career.player_status.performance
            };
            let major_count = tournaments
                .results_ref()
                .iter()
                .filter(|ev| {
                    ev.event.date.starts_with(&format!("{year}-"))
                        && ev.event.tier == TourneyTier::Major
                })
                .flat_map(|ev| ev.series.iter())
                .filter(|s| {
                    s.maps
                        .iter()
                        .any(|m| m.lines.iter().any(|l| l.player_name == pc.name))
                })
                .count() as f64;
            let top20 = career
                .honours
                .individual
                .iter()
                .filter(|h| h.year == year && h.r#type == IndividualHonourType::Top20)
                .count() as f64;
            let major_bonus = (major_count * 4.0).min(16.0);
            let influence = (performance * 0.28
                + leadership * 0.22
                + relation * 0.15
                + (career.reputation as f64) * 0.15
                + (tenure.min(8.0) * 3.0)
                + major_bonus
                + top20 * 10.0)
                .clamp(0.0, 100.0);
            let status = PlayerStatus {
                performance,
                reputation: career.reputation as f64,
                leadership,
                influence,
            };

            let mut memories = Vec::new();
            if career.contract_years > 0
                && (pc.team.is_some() || !career.attended_tournaments.is_empty())
            {
                memories.push((
                    CareerMemoryKind::FirstT1Contract,
                    "你的职业生涯首次站上顶级职业合同。",
                ));
            }
            for honour in career.honours.individual.iter().filter(|h| h.year == year) {
                let (kind, detail) = match honour.r#type {
                    IndividualHonourType::Top20 => {
                        (CareerMemoryKind::Top20, honour.detail.as_str())
                    }
                    IndividualHonourType::Mvp => (CareerMemoryKind::Mvp, honour.detail.as_str()),
                    _ => continue,
                };
                memories.push((kind, detail));
            }
            for ev in tournaments.results_ref().iter().filter(|ev| {
                ev.event.date.starts_with(&format!("{year}-"))
                    && ev.event.tier == TourneyTier::Major
            }) {
                let played = ev.series.iter().any(|s| {
                    s.maps
                        .iter()
                        .any(|m| m.lines.iter().any(|l| l.player_name == pc.name))
                });
                if !played {
                    continue;
                }
                memories.push((CareerMemoryKind::FirstMajor, ev.event.name.as_str()));
                for series in &ev.series {
                    if !series
                        .maps
                        .iter()
                        .any(|m| m.lines.iter().any(|l| l.player_name == pc.name))
                    {
                        continue;
                    }
                    if series.stage.is_playoff() {
                        memories.push((CareerMemoryKind::MajorPlayoff, ev.event.name.as_str()));
                    }
                    if series.stage == csc_simulation::series::SeriesStage::Final {
                        memories.push((CareerMemoryKind::MajorFinal, ev.event.name.as_str()));
                    }
                    if !series.live_feedback.is_empty() {
                        memories.push((
                            CareerMemoryKind::MajorKeyMoment,
                            series.live_feedback[0].narrative.as_str(),
                        ));
                    }
                }
                if ev.champion == pc.team.unwrap_or(csc_util::id::TeamId(u32::MAX)) {
                    memories.push((CareerMemoryKind::MajorChampion, ev.event.name.as_str()));
                }
            }
            if let Some(career_mut) = world.player_mut(pid).and_then(|p| p.career_mut()) {
                career_mut.player_status = status;
                for (kind, detail) in memories {
                    career_mut.remember(CareerMemory {
                        kind,
                        year,
                        date: None,
                        event_name: Some(detail.to_string()),
                        detail: detail.to_string(),
                    });
                }
            }
        }
    }

    /// 赛季目标结算（可解释）：由本赛季档案与上赛季档案对比产生。
    fn evaluate_season_goal(
        career: &csc_entities::career::CareerInfo,
        previous: Option<&SeasonRecord>,
        year: i32,
        rating: f64,
        maps_played: i32,
        team_name: &Option<String>,
    ) -> (Option<bool>, String) {
        let Some(goal) = career.season_goal else {
            return (None, String::new());
        };
        let won_elite_title = career
            .honours
            .team
            .iter()
            .any(|h| h.year == year && h.tier.is_elite());
        let made_top20 = career.honours.individual.iter().any(|h| {
            h.year == year && h.r#type == csc_entities::career::IndividualHonourType::Top20
        });
        let previous_rating = previous.map(|p| p.rating).unwrap_or(0.0);
        let previous_team = previous.and_then(|p| p.team_name.clone());
        let previous_injury_days: Option<i32> = previous.map(|p| p.injury_days);
        let (met, text) = match goal {
            csc_domain::season_goal::SeasonGoal::WinTitle => (
                won_elite_title,
                if won_elite_title {
                    "你拿下了顶级赛事冠军，冠军目标达成。".to_string()
                } else {
                    "本赛季未能捧起顶级赛事奖杯，冠军目标未达成。".to_string()
                },
            ),
            csc_domain::season_goal::SeasonGoal::Top20 => (
                made_top20,
                if made_top20 {
                    "你进入了年度 TOP20，个人目标达成。".to_string()
                } else {
                    "本年度未能进入 TOP20，距离目标还有差距。".to_string()
                },
            ),
            csc_domain::season_goal::SeasonGoal::ImproveRating => {
                let improved = previous.is_none() || rating >= previous_rating + 0.03;
                (
                    improved,
                    format!(
                        "赛季 Rating {rating:.2}（上赛季 {previous_rating:.2}），提升 Rating 目标{}达成。",
                        if improved { "" } else { "未" }
                    ),
                )
            }
            csc_domain::season_goal::SeasonGoal::SecureStarting => {
                let secured = maps_played >= 30 && team_name.is_some();
                (
                    secured,
                    format!(
                        "赛季出战 {maps_played} 图，稳固首发目标{}达成。",
                        if secured { "" } else { "未" }
                    ),
                )
            }
            csc_domain::season_goal::SeasonGoal::SeekTransfer => {
                let moved = previous_team.is_some() && team_name != &previous_team;
                (
                    moved,
                    if moved {
                        "你完成了转会，转会目标达成。".to_string()
                    } else {
                        "本赛季未完成转会，转会目标未达成。".to_string()
                    },
                )
            }
            csc_domain::season_goal::SeasonGoal::RecoverForm => {
                let recovered = match previous_injury_days {
                    None => true, // 首个赛季：无上赛季伤病基准，视作已恢复（行为变更：原为溢出 panic/负值）
                    Some(prev) => {
                        career.injury_days_this_year < prev || career.injury_days_this_year == 0
                    }
                };
                let prev_display = match previous_injury_days {
                    None => "（无上赛季记录）".to_string(),
                    Some(prev) => prev.to_string(), // 直接展示上赛季天数；saturating 溢出已随哨兵删除消除
                };
                (
                    recovered,
                    format!(
                        "本赛季伤病缺勤 {} 天（上赛季 {} 天），恢复状态目标{}达成。",
                        career.injury_days_this_year,
                        prev_display,
                        if recovered { "" } else { "未" }
                    ),
                )
            }
        };
        (Some(met), text)
    }

    /// 赛季目标重置：目标年份 ≤ 上一年度时清空，下赛季重新选择。
    fn reset_season_goals(world: &mut World, prev_year: i32) {
        for pc in &mut world.players {
            let Some(career) = pc.career.as_mut() else {
                continue;
            };
            if career.season_goal_year <= prev_year {
                career.season_goal = None;
                career.season_goal_year = 0;
            }
        }
    }

    /// 生涯档案：把每个玩家的上一年度（赛事/数据/荣誉/财务/印记）归档为 `SeasonRecord`。
    fn archive_year(
        world: &mut World,
        archive: &mut CareerArchive,
        year: i32,
        stats: &std::collections::HashMap<PlayerId, YearlyStat>,
    ) {
        let player_ids: Vec<PlayerId> = world.all_players_only().iter().map(|p| p.id).collect();
        for pid in player_ids {
            let record = {
                let pc = world.player(pid).expect("选手不存在");
                let career = pc.career.as_ref().expect("非玩家");
                let stat = stats.get(&pid);
                let previous = archive.seasons_of(pid).last().cloned();
                let honours: Vec<String> = career
                    .honours
                    .individual
                    .iter()
                    .filter(|h| h.year == year)
                    .map(|h| format!("{}（{}）", honour_type_name(h.r#type), h.detail))
                    .chain(
                        career
                            .honours
                            .team
                            .iter()
                            .filter(|h| h.year == year)
                            .map(|h| format!("{} 冠军", h.title)),
                    )
                    .collect();
                let events_played = career
                    .attended_tournaments
                    .iter()
                    .filter(|t| t.date.starts_with(&format!("{year}-")))
                    .count() as i32;
                let marks: Vec<csc_entities::mark::CareerMarkType> = {
                    let mut v: Vec<_> = career
                        .marks
                        .iter()
                        .filter(|m| m.year == year)
                        .map(|m| m.r#type)
                        .collect();
                    v.sort_by_key(|t| format!("{:?}", t));
                    v.dedup();
                    v
                };
                let maps_played = stat.map(|s| s.maps_all).unwrap_or(0);
                // 档案 Rating 是全年累计计算值，不是逐图 Rating 的算术平均。
                // YearlyStat 已保留全赛事 K/D/A 与总回合（MR12/加时比分之和）。
                let rating = stat
                    .filter(|s| s.rounds > 0)
                    .map(|s| {
                        csc_simulation::rating::RatingCalculator::rating_of_counts(
                            s.kills, s.deaths, s.assists, s.rounds,
                        )
                    })
                    .unwrap_or(0.0);
                let team_name = pc
                    .team
                    .map(|tid| world.team(tid).map(|t| t.name.clone()).unwrap_or_default());
                let (goal_met, goal_outcome) = Self::evaluate_season_goal(
                    career,
                    previous.as_ref(),
                    year,
                    rating,
                    maps_played,
                    &team_name,
                );
                SeasonRecord {
                    player_id: pid,
                    player_name: pc.name.clone(),
                    year,
                    team_name,
                    events_played,
                    maps_played,
                    kills: stat.map(|s| s.kills).unwrap_or(0),
                    deaths: stat.map(|s| s.deaths).unwrap_or(0),
                    assists: stat.map(|s| s.assists).unwrap_or(0),
                    // 档案口径 = 全赛事场均（2026 试玩修复：低级别赛事选手 rating 不再恒 0）
                    rating,
                    wins: stat.map(|s| s.wins).unwrap_or(0),
                    honours,
                    earnings: career.finance.earnings_of(year),
                    marks,
                    injury_days: career.injury_days_this_year,
                    season_goal: career.season_goal,
                    goal_met,
                    goal_outcome,
                }
            };
            archive.record_season(pid, record);
            // 归档后复位（下一年重新累计）
            world
                .player_mut(pid)
                .expect("选手不存在")
                .career_mut()
                .expect("非玩家")
                .injury_days_this_year = 0;
        }
    }

    /// 年度成长结算：全部选手（玩家 + NPC）年龄 +1 并按成长曲线更新属性；
    /// 25 岁起掷生理衰退（自律/指挥抗衰，见 GrowthModel）+ **玩家声誉年度回归**
    /// （2026 试玩修订：声誉向基准 50 收敛 15%——持续辉煌才维持高位）。
    fn apply_yearly_growth(world: &mut World, rng: &mut Xoshiro256StarStar) {
        for pc in &mut world.players {
            if !pc.retired {
                GrowthModel::apply_yearly_growth(pc, rng);
                if pc.is_player() {
                    csc_simulation::FormModel::yearly_reputation_decay(pc);
                }
            }
        }
    }

    /// 年度人口更新：退役 + 新秀补位（委托人口子系统；成长结算后年龄已 +1）。
    ///
    /// @param profile 新选手实力分布模型（None = 新秀固定 Tier4 旧逻辑）
    fn apply_population_turnover(
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        rng: &mut Xoshiro256StarStar,
        profile: Option<&csc_entities::baseline::RatingProfile>,
    ) {
        let event = PopulationEngine::apply_annual_turnover(world, vrs, rng, profile);
        // 事件流降噪（黄金生涯集基线：NPC 退役/新秀占主 journal 的 37%+）：
        // 主角保留单条 Retirement/RookieIntake，NPC 聚合为一条 LiveUpdate。
        let mut npc_retired = 0usize;
        let mut npc_intake = 0usize;
        for r in &event.retired {
            if world.player(r.player_id).is_some_and(|p| p.is_player()) {
                journal.record(WorldEvent::Retirement {
                    date: clock.date_label(),
                    seq: -1,
                    player_name: r.player_name.clone(),
                    player_id: r.player_id,
                    age: r.age,
                    from_team: r.from_team.clone(),
                    from_team_id: r.from_team_id,
                });
            } else {
                npc_retired += 1;
            }
        }
        for i in &event.intake {
            if world.player(i.player_id).is_some_and(|p| p.is_player()) {
                journal.record(WorldEvent::RookieIntake {
                    date: clock.date_label(),
                    seq: -1,
                    player_name: i.player_name.clone(),
                    player_id: i.player_id,
                    age: i.age,
                    into_team: i.into_team.clone(),
                    into_team_id: i.into_team_id,
                });
            } else {
                npc_intake += 1;
            }
        }
        if npc_retired > 0 || npc_intake > 0 {
            journal.record(WorldEvent::LiveUpdate {
                date: clock.date_label(),
                seq: -1,
                headline: "职业生态更替".to_string(),
                detail: format!("本年度 {npc_retired} 名老将退役，{npc_intake} 名新秀进入职业赛场"),
            });
        }
    }

    /// 队伍年度薪资支出（经济闭环的支出端）：每队从预算扣除「玩家薪资 + NPC 运营成本」。
    /// 预算可透支为负（赤字），转会候选过滤保证付不起薪水的队签不进新玩家。
    fn apply_annual_payroll(world: &mut World) {
        // 先收集（借 world 只读），再统一扣减（避免双借用）
        let payrolls: Vec<(csc_util::id::TeamId, i64)> = world
            .teams
            .iter()
            .map(|team| {
                let mut player_payroll: i64 = 0;
                let mut npc_count: i64 = 0;
                for pid in &team.roster_ids {
                    if let Some(pc) = world.player(*pid) {
                        if let Some(career) = &pc.career {
                            player_payroll += career.salary;
                        } else {
                            npc_count += 1;
                        }
                    }
                }
                (team.id, player_payroll + npc_count * Self::NPC_ANNUAL_COST)
            })
            .collect();
        for (tid, cost) in payrolls {
            world.team_mut(tid).expect("队伍不存在").budget -= cost;
        }
    }

    /// 代言结转（跨年结算）：**只**复位评估年份（允许决策批次重新评估）。
    /// 合同年限结转（tickYear）是 `EconomyEngine::annual_sponsor_check` 的
    /// 单点职责——若在跨年结算也扣一次，同一批合同一年会被扣两年（历史 bug 修复约定）。
    fn apply_sponsor_rollover(world: &mut World, prev_year: i32) {
        for pc in &mut world.players {
            if let Some(career) = pc
                .career_mut()
                .filter(|c| c.finance.last_sponsor_check_year >= prev_year)
            {
                career.finance.last_sponsor_check_year = prev_year - 1;
            }
        }
    }

    /// 合同年度结转：日历年变化时所有玩家合同年限 -1（0 不再减，到期后下一转会窗可转会）。
    fn contracts_expire_one_year(world: &mut World) {
        for pc in &mut world.players {
            if let Some(career) = pc.career_mut().filter(|c| c.contract_years > 0) {
                career.contract_years -= 1;
            }
        }
    }
}

/// 个人荣誉类型名（= Kotlin `IndividualHonourType.name`，档案字符串用）。
pub fn honour_type_name(t: IndividualHonourType) -> &'static str {
    match t {
        IndividualHonourType::Top20 => "TOP20",
        IndividualHonourType::Mvp => "MVP",
        IndividualHonourType::Evp => "EVP",
        IndividualHonourType::AllStarTeam => "ALL_STAR_TEAM",
        IndividualHonourType::RookieOfTheYear => "ROOKIE_OF_THE_YEAR",
        IndividualHonourType::BestClutchPlayer => "BEST_CLUTCH_PLAYER",
        IndividualHonourType::Other => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::season_goal::SeasonGoal;

    #[test]
    fn honour_names_match_kotlin() {
        assert_eq!(honour_type_name(IndividualHonourType::Mvp), "MVP");
        assert_eq!(honour_type_name(IndividualHonourType::Top20), "TOP20");
    }

    #[test]
    fn contracts_and_sponsor_rollover() {
        let mut world = World::new();
        let p = world.create_player(
            csc_domain::tier::Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        {
            let c = world.player_mut(p).unwrap().career.as_mut().unwrap();
            c.contract_years = 3;
            c.finance.last_sponsor_check_year = 2026;
        }
        YearlySettlement::contracts_expire_one_year(&mut world);
        YearlySettlement::apply_sponsor_rollover(&mut world, 2026);
        let c = world.player(p).unwrap().career.as_ref().unwrap();
        assert_eq!(c.contract_years, 2);
        assert_eq!(c.finance.last_sponsor_check_year, 2025);
    }

    #[test]
    fn payroll_deducts_salary_and_npc_cost() {
        let mut world = World::new();
        let t = world.create_team("Vitality", 1, 2000);
        world.team_mut(t).unwrap().budget = 1_000_000;
        let p = world.create_player(
            csc_domain::tier::Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        world.player_mut(p).unwrap().career.as_mut().unwrap().salary = 200_000;
        world.assign_player_to_team(p, t).unwrap();
        for i in 0..4 {
            world
                .create_npc(
                    csc_domain::tier::Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("N{i}")),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                )
                .unwrap();
        }
        YearlySettlement::apply_annual_payroll(&mut world);
        // 200k 薪资 + 4 × 30k NPC 成本
        assert_eq!(world.team(t).unwrap().budget, 1_000_000 - 200_000 - 120_000);
    }

    // —— 赛季目标结算六分支（A-E2）——

    /// 构造目标选手（free_agent_default + 目标字段 + 本季伤病天数）。
    fn career(goal: SeasonGoal, injury_days: i32) -> csc_entities::career::CareerInfo {
        let mut c = csc_entities::career::CareerInfo::free_agent_default(2026);
        c.season_goal = Some(goal);
        c.season_goal_year = 2026;
        c.injury_days_this_year = injury_days;
        c
    }

    /// 构造一条上赛季档案记录。
    fn season_record(injury_days: i32, rating: f64, team: Option<&str>) -> SeasonRecord {
        SeasonRecord {
            player_id: PlayerId(1),
            player_name: "P".into(),
            year: 2025,
            team_name: team.map(String::from),
            events_played: 10,
            maps_played: 35,
            kills: 100,
            deaths: 80,
            assists: 20,
            rating,
            wins: 5,
            honours: Vec::new(),
            earnings: 0,
            marks: Vec::new(),
            injury_days,
            season_goal: None,
            goal_met: None,
            goal_outcome: String::new(),
        }
    }

    #[test]
    fn season_goal_win_title() {
        // 达成：本年度 elite 团队荣誉
        let mut c = career(SeasonGoal::WinTitle, 0);
        c.honours.team.push(csc_entities::career::TeamHonour {
            title: "2026 BLAST Bounty".into(),
            year: 2026,
            tier: TourneyTier::Elite,
            detail: String::new(),
        });
        let (met, text) = YearlySettlement::evaluate_season_goal(&c, None, 2026, 1.0, 30, &None);
        assert_eq!(met, Some(true), "elite 冠军应达成：{text}");
        assert!(text.contains("达成"));

        // 未达成：无荣誉
        let c2 = career(SeasonGoal::WinTitle, 0);
        let (met2, text2) = YearlySettlement::evaluate_season_goal(&c2, None, 2026, 1.0, 30, &None);
        assert_eq!(met2, Some(false), "无冠军应未达成：{text2}");
    }

    #[test]
    fn season_goal_top20() {
        let mut c = career(SeasonGoal::Top20, 0);
        c.honours
            .individual
            .push(csc_entities::career::IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year: 2026,
                detail: "TOP20 第 5 名".into(),
                tier: None,
                event_name: None,
                rank: Some(5),
            });
        let (met, text) = YearlySettlement::evaluate_season_goal(&c, None, 2026, 1.0, 30, &None);
        assert_eq!(met, Some(true), "TOP20 应达成：{text}");

        let c2 = career(SeasonGoal::Top20, 0);
        let (met2, _) = YearlySettlement::evaluate_season_goal(&c2, None, 2026, 1.0, 30, &None);
        assert_eq!(met2, Some(false), "无 TOP20 应未达成");
    }

    #[test]
    fn season_goal_improve_rating() {
        // previous=None → 达成（无基准视作达成）
        let c = career(SeasonGoal::ImproveRating, 0);
        let (met, _) = YearlySettlement::evaluate_season_goal(&c, None, 2026, 0.5, 30, &None);
        assert_eq!(met, Some(true), "previous=None 应达成");
        // previous=1.00、rating=1.02 → 未达成（< +0.03）
        let prev = season_record(0, 1.00, Some("A"));
        let c2 = career(SeasonGoal::ImproveRating, 0);
        let (met2, _) =
            YearlySettlement::evaluate_season_goal(&c2, Some(&prev), 2026, 1.02, 30, &None);
        assert_eq!(met2, Some(false), "1.02 < 1.03 应未达成");
        // previous=1.00、rating=1.04 → 达成
        let c3 = career(SeasonGoal::ImproveRating, 0);
        let (met3, _) =
            YearlySettlement::evaluate_season_goal(&c3, Some(&prev), 2026, 1.04, 30, &None);
        assert_eq!(met3, Some(true), "1.04 >= 1.03 应达成（边界含等号）");
        // 边界：rating=1.03 → 达成（实现是 >=）
        let c4 = career(SeasonGoal::ImproveRating, 0);
        let (met4, _) =
            YearlySettlement::evaluate_season_goal(&c4, Some(&prev), 2026, 1.03, 30, &None);
        assert_eq!(met4, Some(true), "1.03 == +0.03 边界应达成");
    }

    #[test]
    fn season_goal_secure_starting() {
        // maps_played=30 + team_name=Some → 达成
        let c = career(SeasonGoal::SecureStarting, 0);
        let (met, _) =
            YearlySettlement::evaluate_season_goal(&c, None, 2026, 1.0, 30, &Some("Team".into()));
        assert_eq!(met, Some(true), "30 图且有队应达成");
        // maps_played=29 → 未达成
        let c2 = career(SeasonGoal::SecureStarting, 0);
        let (met2, _) =
            YearlySettlement::evaluate_season_goal(&c2, None, 2026, 1.0, 29, &Some("Team".into()));
        assert_eq!(met2, Some(false), "29 图应未达成");
        // team_name=None → 未达成
        let c3 = career(SeasonGoal::SecureStarting, 0);
        let (met3, _) = YearlySettlement::evaluate_season_goal(&c3, None, 2026, 1.0, 30, &None);
        assert_eq!(met3, Some(false), "自由身应未达成");
    }

    #[test]
    fn season_goal_seek_transfer() {
        // 上赛季 A 队、当前 B 队 → 达成
        let prev = season_record(0, 1.0, Some("A"));
        let c = career(SeasonGoal::SeekTransfer, 0);
        let (met, _) = YearlySettlement::evaluate_season_goal(
            &c,
            Some(&prev),
            2026,
            1.0,
            30,
            &Some("B".into()),
        );
        assert_eq!(met, Some(true), "换队应达成");
        // 同队 → 未达成
        let c2 = career(SeasonGoal::SeekTransfer, 0);
        let (met2, _) = YearlySettlement::evaluate_season_goal(
            &c2,
            Some(&prev),
            2026,
            1.0,
            30,
            &Some("A".into()),
        );
        assert_eq!(met2, Some(false), "同队应未达成");
        // previous=None → 未达成
        let c3 = career(SeasonGoal::SeekTransfer, 0);
        let (met3, _) =
            YearlySettlement::evaluate_season_goal(&c3, None, 2026, 1.0, 30, &Some("B".into()));
        assert_eq!(met3, Some(false), "无上赛季应未达成");
    }

    #[test]
    fn season_goal_recover_form_no_overflow() {
        // 核心：previous=None + 本季 25 天 → 不 panic、达成、文案含「无上赛季记录」
        let c = career(SeasonGoal::RecoverForm, 25);
        let (met, text) = YearlySettlement::evaluate_season_goal(&c, None, 2026, 1.0, 30, &None);
        assert_eq!(met, Some(true), "无上赛季基准应达成：{text}");
        assert!(
            text.contains("无上赛季记录"),
            "文案应含「无上赛季记录」：{text}"
        );

        // previous=30 + 本季=10 → 达成（下降）
        let prev = season_record(30, 1.0, None);
        let c2 = career(SeasonGoal::RecoverForm, 10);
        let (met2, _) =
            YearlySettlement::evaluate_season_goal(&c2, Some(&prev), 2026, 1.0, 30, &None);
        assert_eq!(met2, Some(true), "伤病下降应达成");

        // previous=30 + 本季=40 → 未达成（恶化）
        let c3 = career(SeasonGoal::RecoverForm, 40);
        let (met3, _) =
            YearlySettlement::evaluate_season_goal(&c3, Some(&prev), 2026, 1.0, 30, &None);
        assert_eq!(met3, Some(false), "伤病恶化应未达成");

        // previous=30 + 本季=0 → 达成（零缺勤）
        let c4 = career(SeasonGoal::RecoverForm, 0);
        let (met4, _) =
            YearlySettlement::evaluate_season_goal(&c4, Some(&prev), 2026, 1.0, 30, &None);
        assert_eq!(met4, Some(true), "零缺勤应达成");
    }

    #[test]
    fn season_goal_none_returns_empty() {
        // 无目标：直接构造无目标 career
        let mut c2 = csc_entities::career::CareerInfo::free_agent_default(2026);
        c2.season_goal = None;
        let (met, text) = YearlySettlement::evaluate_season_goal(&c2, None, 2026, 1.0, 30, &None);
        assert_eq!(met, None);
        assert_eq!(text, "");
    }
}
