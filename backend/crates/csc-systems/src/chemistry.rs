//! 队内关系/矛盾子系统（Kotlin `ChemistryEngine.kt` 转写）：
//! **队友失误瞬间 → 玩家反应 → 关系/凝聚力/印记**。
//!
//! 触发端：赛事引擎精确路径赛后检测玩家队伍 NPC 的糟糕表现，产出
//! `DecisionPoint::TeammateBlunder`；本引擎只负责**应用反应**
//! （决策结果 → 世界状态），规则公式在 `csc-simulation::ChemistryModel`。

use csc_decision::point::DecisionPoint;
use csc_entities::chemistry::TeamChemistry;
use csc_entities::mark::CareerMarkType;
use csc_entities::mark::CareerMarks;
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::chemistry_model::BlunderReaction;
use csc_simulation::chemistry_model::ChemistryModel;
use csc_simulation::directives::BlunderKind;
use csc_util::id::{PlayerId, TeamId};

/// 队内关系/矛盾子系统：无跨调用状态（journal 参数化传入）。
pub struct ChemistryEngine;

impl ChemistryEngine {
    /// 应用玩家对队友失误的一次反应。
    ///
    /// 效果：关系调整 → 双方士气 → 生涯印记（支持型/毒瘤/战术领袖）→
    /// 凝聚力重算 → （关系跌破阈值）冲突事件。
    ///
    /// @param source 决策点 id（印记可追溯；事件日期取 `|` 前段）
    #[allow(clippy::too_many_arguments)]
    pub fn apply_blunder_reaction(
        world: &mut World,
        player_id: PlayerId,
        teammate_id: PlayerId,
        _blunder: BlunderKind,
        reaction: BlunderReaction,
        team_id: TeamId,
        year: i32,
        source: &str,
        journal: Option<&mut WorldJournal>,
    ) {
        // 先取名字副本（避免借用冲突）
        let (player_name, teammate_name) = {
            let p = world.player(player_id).expect("玩家不存在");
            let t = world.player(teammate_id).expect("队友不存在");
            (p.name.clone(), t.name.clone())
        };
        let delta = ChemistryModel::relation_delta(reaction);
        let roster_names: Vec<String> = world
            .roster(team_id)
            .iter()
            .map(|p| p.name.clone())
            .collect();

        // 1. 关系矩阵调整 + 凝聚力重算
        {
            let team = world.team_mut(team_id).expect("队伍不存在");
            team.chemistry.adjust(&player_name, &teammate_name, delta);
            team.chemistry.cohesion = ChemistryModel::cohesion_of(&team.chemistry, &roster_names);
        }

        // 2. 双方士气
        {
            let team_morale = ChemistryModel::teammate_morale_delta(reaction);
            let self_morale = ChemistryModel::self_morale_delta(reaction);
            let p = world.player_mut(player_id).expect("玩家不存在");
            p.pro.morale = (p.pro.morale + self_morale).clamp(0, 100);
            let t = world.player_mut(teammate_id).expect("队友不存在");
            t.pro.morale = (t.pro.morale + team_morale).clamp(0, 100);
        }

        // 3. 印记累积（支持型/毒瘤/战术领袖）
        if let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        {
            match reaction {
                BlunderReaction::Support => {
                    CareerMarks::apply(
                        &mut career.marks,
                        CareerMarkType::SupportiveTeammate,
                        year,
                        source,
                        1,
                    );
                    CareerMarks::apply(
                        &mut career.marks,
                        CareerMarkType::TacticalLeader,
                        year,
                        source,
                        0,
                    );
                }
                BlunderReaction::Confront => {
                    CareerMarks::apply(&mut career.marks, CareerMarkType::Toxic, year, source, 1);
                }
                BlunderReaction::Ignore => {}
            }
        }

        // 4. 冲突事件：关系跌破阈值 → 凝聚力直接受损 + 日志
        let relation = world
            .team(team_id)
            .expect("队伍不存在")
            .chemistry
            .relation_of(&player_name, &teammate_name);
        if relation < ChemistryModel::CONFLICT_THRESHOLD {
            world
                .team_mut(team_id)
                .expect("队伍不存在")
                .chemistry
                .cohesion = (world.team(team_id).expect("队伍不存在").chemistry.cohesion - 3.0)
                .clamp(0.0, 100.0);
            if let Some(j) = journal {
                j.record(WorldEvent::Conflict {
                    date: source.split('|').next().unwrap_or("?").to_string(),
                    seq: -1,
                    player_name,
                    player_id,
                    teammate_name,
                    teammate_id,
                    severity: ChemistryModel::CONFLICT_THRESHOLD - relation,
                });
            }
        }
    }

    /// 月度关系恢复 tick：全部队伍的关系向默认值（50）小幅收敛 + 凝聚力重算
    /// （"时间治愈一切"——防止一次矛盾永久锁死队伍氛围）。
    pub fn monthly_recovery(world: &mut World) {
        for team_id in 0..world.teams.len() {
            let team_id = TeamId(team_id as u32);
            let names: Vec<String> = world
                .roster(team_id)
                .iter()
                .map(|p| p.name.clone())
                .collect();
            {
                let team = world.team_mut(team_id).expect("队伍不存在");
                let chem: &mut TeamChemistry = &mut team.chemistry;
                for a in &names {
                    for b in &names {
                        if a == b {
                            continue;
                        }
                        let v = chem.relation_of(a, b);
                        if v > ChemistryModel::DEFAULT_RELATION {
                            chem.set_relation(a, b, v - ChemistryModel::RELATION_RECOVERY);
                        } else if v < ChemistryModel::DEFAULT_RELATION {
                            chem.set_relation(a, b, v + ChemistryModel::RELATION_RECOVERY);
                        }
                    }
                }
                chem.cohesion = ChemistryModel::cohesion_of(chem, &names);
            }
        }
    }

    /// 便捷：团队阵容的凝聚力修正乘数（供胜率计算）。
    pub fn cohesion_factor_of(world: &World, team_id: TeamId) -> f64 {
        world
            .team(team_id)
            .map(|t| ChemistryModel::cohesion_factor(t.chemistry.cohesion))
            .unwrap_or(1.0)
    }

    /// 关系查询（表现层用）。
    pub fn relation_between(world: &World, team_id: TeamId, a: &str, b: &str) -> f64 {
        world
            .team(team_id)
            .map(|t| t.chemistry.relation_of(a, b))
            .unwrap_or(ChemistryModel::DEFAULT_RELATION)
    }

    /// 玩家失误反应决策点 id 段（与 `DecisionPoint::TeammateBlunder` 对齐，供测试）。
    /// M1：队友段由名字改 PlayerId 键控（同名不同人不再碰撞）。
    pub fn point_id_of(
        date: &str,
        event_name: &str,
        map_number: i32,
        teammate_id: PlayerId,
    ) -> String {
        format!("{date}|blunder|{event_name}|{map_number}|{teammate_id}")
    }

    /// 构造一次失误决策点（Kotlin `handleBlunders` 内的点构造；id 确定性；
    /// 决策主体与队友均携带稳定 ID）。
    #[allow(clippy::too_many_arguments)]
    pub fn blunder_point(
        date: &str,
        player_id: PlayerId,
        player_name: &str,
        event_name: &str,
        map_number: i32,
        teammate_id: PlayerId,
        teammate_name: &str,
        teammate_role: csc_entities::role::Role,
        blunder: BlunderKind,
        detail: &str,
    ) -> DecisionPoint {
        DecisionPoint::TeammateBlunder {
            id: Self::point_id_of(date, event_name, map_number, teammate_id),
            date: date.to_string(),
            player_id,
            player_name: player_name.to_string(),
            event_name: event_name.to_string(),
            map_number,
            teammate_name: teammate_name.to_string(),
            teammate_id,
            teammate_role,
            blunder,
            detail: detail.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;
    use csc_util::rng::Xoshiro256StarStar;

    fn world_with_team() -> (World, PlayerId, PlayerId, TeamId) {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let p = w.create_player(
            Tier::Tier1,
            Some("Player1"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        let n = w
            .create_npc(
                Tier::Tier1,
                Some(t),
                None,
                Some("ZywOo"),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
        w.assign_player_to_team(p, t).unwrap();
        (w, p, n, t)
    }

    #[test]
    fn support_reaction_improves_relation_and_marks() {
        let (mut w, p, n, t) = world_with_team();
        let mut journal = WorldJournal::default();
        // 先制造矛盾：CONFRONT 两次
        for _ in 0..2 {
            ChemistryEngine::apply_blunder_reaction(
                &mut w,
                p,
                n,
                BlunderKind::Choke,
                BlunderReaction::Confront,
                t,
                2026,
                "2026-06-08|blunder|EV|1|ZywOo",
                Some(&mut journal),
            );
        }
        let relation = w.team(t).unwrap().chemistry.relation_of("Player1", "ZywOo");
        assert!(relation < 50.0, "指责降低关系: {relation}");
        // 鼓励恢复
        ChemistryEngine::apply_blunder_reaction(
            &mut w,
            p,
            n,
            BlunderKind::Choke,
            BlunderReaction::Support,
            t,
            2026,
            "2026-06-08|blunder|EV|2|ZywOo",
            Some(&mut journal),
        );
        let relation = w.team(t).unwrap().chemistry.relation_of("Player1", "ZywOo");
        assert!(relation > 0.0);
        // 支持型印记
        let marks = &w.player(p).unwrap().career.as_ref().unwrap().marks;
        assert!(
            marks
                .iter()
                .any(|m| m.r#type == CareerMarkType::SupportiveTeammate)
        );
        // 毒瘤印记（CONFRONT 路径）
        assert!(marks.iter().any(|m| m.r#type == CareerMarkType::Toxic));
    }

    #[test]
    fn conflict_event_when_relation_drops() {
        let (mut w, p, n, t) = world_with_team();
        let mut journal = WorldJournal::default();
        // CONFRONT 多次直到跌破阈值
        let mut conflicted = false;
        for _ in 0..6 {
            ChemistryEngine::apply_blunder_reaction(
                &mut w,
                p,
                n,
                BlunderKind::Tilt,
                BlunderReaction::Confront,
                t,
                2026,
                "2026-06-08|blunder|EV|1|ZywOo",
                Some(&mut journal),
            );
            if !journal.is_empty() {
                conflicted = true;
                break;
            }
        }
        assert!(conflicted, "关系跌破阈值应记录 Conflict 事件");
        let cohesion = w.team(t).unwrap().chemistry.cohesion;
        assert!(cohesion < 50.0, "冲突损伤凝聚力: {cohesion}");
    }

    #[test]
    fn monthly_recovery_converges_to_default() {
        let (mut w, _p, _n, t) = world_with_team();
        w.team_mut(t)
            .unwrap()
            .chemistry
            .set_relation("Player1", "ZywOo", 20.0);
        ChemistryEngine::monthly_recovery(&mut w);
        let r = w.team(t).unwrap().chemistry.relation_of("Player1", "ZywOo");
        assert!((r - 22.0).abs() < 1e-9, "向默认值收敛 +2: {r}");
    }

    #[test]
    fn blunder_point_id_deterministic() {
        let p = ChemistryEngine::blunder_point(
            "2026-06-08",
            PlayerId(1),
            "Player1",
            "T1 Monthly #1",
            2,
            PlayerId(2),
            "ZywOo",
            csc_entities::role::Role::Awp,
            BlunderKind::Choke,
            "本图 1 杀（预期 ~5）",
        );
        assert_eq!(p.id(), "2026-06-08|blunder|T1 Monthly #1|2|2");
        assert_eq!(p.player_id(), PlayerId(1));
    }

    /// M1 回归：同名不同人 → 不同 point_id（旧「名字键控」格式会碰撞）。
    #[test]
    fn blunder_point_ids_differ_for_same_name_players() {
        let a = ChemistryEngine::blunder_point(
            "2026-06-08",
            PlayerId(1),
            "Player1",
            "T1 Monthly #1",
            2,
            PlayerId(2),
            "ZywOo",
            csc_entities::role::Role::Awp,
            BlunderKind::Choke,
            "detail",
        );
        let b = ChemistryEngine::blunder_point(
            "2026-06-08",
            PlayerId(1),
            "Player1",
            "T1 Monthly #1",
            2,
            PlayerId(3),
            "ZywOo",
            csc_entities::role::Role::Awp,
            BlunderKind::Choke,
            "detail",
        );
        assert_ne!(
            a.id(),
            b.id(),
            "同名队友（不同 PlayerId）→ 决策点 ID 必须不同"
        );
        assert_eq!(a.id(), "2026-06-08|blunder|T1 Monthly #1|2|2");
        assert_eq!(b.id(), "2026-06-08|blunder|T1 Monthly #1|2|3");
    }
}
