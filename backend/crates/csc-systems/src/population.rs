//! 人口再生子系统（Kotlin `PopulationEngine.kt` 转写）：**退役 + 新秀补位**。
//!
//! 背景：世界人口是**封闭**的——NPC 只从初始 standings 快照生成一次，
//! 选手同生共死、若干年后全员衰退、无新鲜血液。本引擎在每年跨年结算时
//! 执行人口更新：
//! - **主角退役**：年龄 ≥ [`PopulationEngine::PROTAGONIST_RETIREMENT_AGE`]（36 岁）
//!   的玩家谢幕——离队 + 标记退役（实体保留，生涯档案/结局仍可查），队伍由
//!   青训新秀补位（2026 修订：修复"主角永不退役、36 岁仍霸榜"的试玩缺陷）；
//! - **NPC 退役**：年龄 ≥ [`PopulationEngine::RETIREMENT_AGE`] 的 NPC 退役；
//!   自由市场老将直接移除；
//! - **新秀补位**：队伍内老将退役 → 生成一名**青训新秀**（18~20 岁，潜力高、
//!   成长快）补进同一队伍——保持 roster 5 人不变式，并同步 VRS 阵容迁移
//!   （[`VrsEngine::apply_roster_change_verified`]）。新秀起始档位默认 Tier4；
//!   装配了 [`RatingProfile`]（`rating_profile.json`）时按真实职业分布采样
//!   （多数青训 Tier3/4、少量一线天才、极少数 Tier0——donk 级）。
//!
//! 因果闭环：老将衰退（GrowthModel 32+ 岁逐年 -3）→ 到达退役年龄 → 被年轻新秀
//! 顶替 → 新秀经多年成长接棒——人口与实力梯队随年代自然更替。

use csc_entities::world::World;
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

/// 年度人口更新事件（信息用途，供展示/测试断言）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PopulationEvent {
    pub retired: Vec<RetirementRecord>,
    pub intake: Vec<IntakeRecord>,
}

impl PopulationEvent {
    pub fn is_empty(&self) -> bool {
        self.retired.is_empty() && self.intake.is_empty()
    }
}

/// 一名退役选手。
#[derive(Debug, Clone, PartialEq)]
pub struct RetirementRecord {
    /// 退役选手稳定 ID
    pub player_id: PlayerId,
    pub player_name: String,
    pub age: i32,
    /// 退役时所在队伍（None = 自由市场老将）
    pub from_team: Option<String>,
    /// 退役时所在队伍 ID（None = 自由市场老将）
    pub from_team_id: Option<TeamId>,
}

/// 一名新入世界的新秀。
#[derive(Debug, Clone, PartialEq)]
pub struct IntakeRecord {
    /// 新秀稳定 ID
    pub player_id: PlayerId,
    pub player_name: String,
    pub age: i32,
    pub into_team: String,
    pub into_team_id: TeamId,
}

/// 人口再生子系统：无跨调用状态。
pub struct PopulationEngine;

impl PopulationEngine {
    /// NPC 退役年龄阈值（GrowthModel 32+ 岁已衰退多年，35 岁退役合理）。
    pub const RETIREMENT_AGE: i32 = 35;
    /// **主角退役年龄（2026 修订）**：36 岁谢幕——比 NPC 晚一年，给玩家完整的
    /// 巅峰-衰退体验（36 岁时 GrowthModel 已连续衰退 4 年，实力显著下滑）。
    pub const PROTAGONIST_RETIREMENT_AGE: i32 = 36;
    /// 青训新秀年龄区间（18~20 岁：潜力大、成长快）。
    pub const ROOKIE_MIN_AGE: i32 = 18;
    pub const ROOKIE_MAX_AGE: i32 = 20;

    /// 执行一次年度人口更新：主角退役（≥36 岁）+ NPC 退役 + 新秀补位。
    ///
    /// @param rng 随机源（新秀生成；固定种子可复现）
    /// @param profile 新选手实力分布模型（`rating_profile.json`；None = 新秀
    ///        固定 Tier4 旧逻辑）。存在时新秀按全职业 Rating 分布采样起始档位
    ///        （多数青训 Tier3/4，少量一线天才，~1.6% Tier0——donk 级）。
    pub fn apply_annual_turnover(
        world: &mut World,
        vrs: &mut VrsEngine,
        rng: &mut Xoshiro256StarStar,
        profile: Option<&csc_entities::baseline::RatingProfile>,
    ) -> PopulationEvent {
        let mut event = PopulationEvent::default();

        // 0. 主角退役（36 岁谢幕）：离队 + 标记退役（保留实体——生涯档案/结局仍可查）
        //    队伍由青训新秀补位（保持 5 人不变式）+ VRS 阵容迁移。
        let protagonist_retire: Option<PlayerId> = world
            .players
            .iter()
            .find(|p| p.is_player() && !p.retired && p.age >= Self::PROTAGONIST_RETIREMENT_AGE)
            .map(|p| p.id);
        if let Some(pid) = protagonist_retire {
            let (name, age, team_id) = {
                let pc = world.player(pid).expect("选手不存在");
                (pc.name.clone(), pc.age, pc.team)
            };
            match team_id {
                Some(tid) => {
                    let team_name = world.team(tid).map(|t| t.name.clone()).unwrap_or_default();
                    let old_sig = world.signature_of(tid);
                    world
                        .release_player(pid)
                        .expect("内部不变量：主角必存在于 roster");
                    world.player_mut(pid).expect("选手不存在").retired = true;
                    // 补位新秀（与 NPC 退役共用 replenish_roster；唯一名 + 档位采样 +
                    // 18~20 岁）。注意：`tail_seed` 必须先于 helper 消费——历史 RNG
                    // 顺序是 名字→年龄→长尾→档位采样，改动会改变世界序列。
                    let tail_seed = rng.next_double();
                    let (rookie, rookie_name, rookie_age) = Self::replenish_roster(
                        world,
                        vrs,
                        profile,
                        tid,
                        &old_sig,
                        Some(tail_seed),
                        rng,
                    );
                    event.retired.push(RetirementRecord {
                        player_id: pid,
                        player_name: name,
                        age,
                        from_team: Some(team_name.clone()),
                        from_team_id: Some(tid),
                    });
                    event.intake.push(IntakeRecord {
                        player_id: rookie,
                        player_name: rookie_name,
                        age: rookie_age,
                        into_team: team_name,
                        into_team_id: tid,
                    });
                }
                None => {
                    // 自由身主角退役：直接标记（无队伍可补位）
                    world.player_mut(pid).expect("选手不存在").retired = true;
                    event.retired.push(RetirementRecord {
                        player_id: pid,
                        player_name: name,
                        age,
                        from_team: None,
                        from_team_id: None,
                    });
                }
            }
        }

        let retire_ids: Vec<PlayerId> = world
            .players
            .iter()
            .filter(|p| p.is_npc() && p.age >= Self::RETIREMENT_AGE)
            .map(|p| p.id)
            .collect();
        // Kotlin partition 语义：先分类（自由人 / 队伍内），各自处理——
        // 退役移除会改变 arena，第二遍只能迭代"队伍内"列表（不可重复访问已删选手）
        let (in_teams, free_agents): (Vec<PlayerId>, Vec<PlayerId>) =
            retire_ids.iter().partition(|id| {
                world
                    .player(**id)
                    .map(|p| p.team.is_some())
                    .unwrap_or(false)
            });

        // 1. 自由市场老将退役：直接移除（无队伍可补位）
        for id in &free_agents {
            let (name, age) = {
                let pc = world.player(*id).expect("选手不存在");
                (pc.name.clone(), pc.age)
            };
            world
                .remove_npc(*id)
                .expect("内部不变量：退役自由人必存在于 arena");
            event.retired.push(RetirementRecord {
                player_id: *id,
                player_name: name,
                age,
                from_team: None,
                from_team_id: None,
            });
        }

        // 2. 队伍内老将退役 → 新秀补位（保持 roster 5 人不变式）
        for npc_id in &in_teams {
            let npc_id = *npc_id;
            let team_id = world
                .player(npc_id)
                .expect("选手不存在")
                .team
                .expect("选手不存在");
            assert!(
                world.player(npc_id).expect("选手不存在").is_npc(),
                "退役者应为 NPC（主角不退役）"
            );

            // 数据一致性守卫：NPC 声称属队但不在该队 roster → 只移除、不补位
            let in_roster = world
                .team(team_id)
                .map(|t| t.roster_ids.contains(&npc_id))
                .unwrap_or(false);
            let (npc_name, npc_age, team_name) = {
                let pc = world.player(npc_id).expect("选手不存在");
                (
                    pc.name.clone(),
                    pc.age,
                    world
                        .team(team_id)
                        .map(|t| t.name.clone())
                        .unwrap_or_default(),
                )
            };
            if !in_roster {
                world
                    .remove_npc(npc_id)
                    .expect("内部不变量：退役 NPC 必存在于 arena");
                event.retired.push(RetirementRecord {
                    player_id: npc_id,
                    player_name: npc_name,
                    age: npc_age,
                    from_team: Some(team_name),
                    from_team_id: Some(team_id),
                });
                continue;
            }
            let old_sig = world.signature_of(team_id);

            // 老将离队（World 原子操作：roster_ids 移除 + team=None）
            world
                .remove_npc(npc_id)
                .expect("内部不变量：退役 NPC 必存在于 arena");

            // 青训新秀补位（与主角退役共用 replenish_roster；唯一名 + 档位采样 +
            // 18~20 岁；NPC 路径无潜力长尾 roll）
            let (rookie, rookie_name, rookie_age) =
                Self::replenish_roster(world, vrs, profile, team_id, &old_sig, None, rng);

            event.retired.push(RetirementRecord {
                player_id: npc_id,
                player_name: npc_name,
                age: npc_age,
                from_team: Some(team_name.clone()),
                from_team_id: Some(team_id),
            });
            event.intake.push(IntakeRecord {
                player_id: rookie,
                player_name: rookie_name,
                age: rookie_age,
                into_team: team_name,
                into_team_id: team_id,
            });
        }
        event
    }

    /// 队伍补位新秀（退役/离队后保持 roster 5 人不变式）——主角退役与 NPC 退役
    /// 共用（结构拆分收敛：此前两份 ~50 行复制）。返回 `(rookie_id, name, age)`。
    ///
    /// - `old_sig` 由调用方在移除退役者**之前**捕获（签名 = 退役前阵容）；
    /// - `tail_roll` = 调用方**预先**消费的 `next_double`（主角路径的潜力长尾
    ///   采样；NPC 路径传 `None`）——**RNG 消耗顺序必须与历史一致**：
    ///   `random_name → 年龄 → (tail_roll 已抽) → newcomer_tier → create_npc`；
    /// - 撞名加数字后缀（名字池耗尽时），确定性、不额外消耗 rng
    ///   （20 年浸泡测试捕获：8 年 ~40 名新秀后原查重循环死转）。
    fn replenish_roster(
        world: &mut World,
        vrs: &mut VrsEngine,
        profile: Option<&csc_entities::baseline::RatingProfile>,
        team_id: TeamId,
        old_sig: &str,
        tail_roll: Option<f64>,
        rng: &mut Xoshiro256StarStar,
    ) -> (PlayerId, String, i32) {
        let base = csc_entities::generator::RandomPlayerGenerator::random_name(rng);
        let mut rookie_name = base.clone();
        let mut suffix = 2u32;
        while world.player_by_name(&rookie_name).is_some() {
            rookie_name = format!("{base}{suffix}");
            suffix += 1;
        }
        let rookie_roll_age = Self::ROOKIE_MIN_AGE
            + rng.next_i32_bound(Self::ROOKIE_MAX_AGE - Self::ROOKIE_MIN_AGE + 1);
        // 新秀起始档位：真实分布采样（profile）或固定 Tier4（旧逻辑）
        let tier = csc_entities::generator::RandomPlayerGenerator::newcomer_tier(profile, rng);
        let rookie = world
            .create_npc(
                tier,
                Some(team_id),
                None,
                Some(&rookie_name),
                rng,
                Some(rookie_roll_age),
            )
            .expect("内部不变量：补位队伍必存在于 arena");
        // 天才长尾（生态校准注入）：真实三年榜 19% 选手能连续登顶——新秀以
        // 同比例获得接近满值的潜力（未来之星 / 时代统治者素材）。
        if rookie_roll_age <= 23
            && let Some(seed) = tail_roll
            && seed < csc_simulation::calibration::CalibrationProfile::default().elite_tail_ratio
            && let Some(pc) = world.player_mut(rookie)
        {
            pc.potential = (pc.potential + 25).min(100);
        }
        let rookie_age = world.player(rookie).expect("选手不存在").age;
        // 新秀已由 create_npc 加入 roster → 队伍签名已变 → VRS 阵容迁移
        let new_names: Vec<String> = world
            .roster(team_id)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        vrs.apply_roster_change_verified(old_sig, &new_names);
        (rookie, rookie_name, rookie_age)
    }

    /// 队伍阵容签名变化辅助（供测试断言）。
    pub fn roster_names(world: &World, team_id: TeamId) -> Vec<String> {
        world
            .roster(team_id)
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;

    fn world_with_old_npc() -> (World, TeamId, PlayerId) {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        // 35 岁老将
        let old = w
            .create_npc(
                Tier::Tier1,
                Some(t),
                None,
                Some("OldMan"),
                &mut Xoshiro256StarStar::seed(5),
                Some(35),
            )
            .unwrap();
        // 4 名年轻队员补满
        for i in 0..4 {
            w.create_npc(
                Tier::Tier1,
                Some(t),
                None,
                Some(&format!("Young{i}")),
                &mut Xoshiro256StarStar::seed(5),
                Some(20),
            )
            .unwrap();
        }
        assert_eq!(w.team(t).unwrap().roster_ids.len(), 5);
        (w, t, old)
    }

    #[test]
    fn veteran_retires_and_rookie_replenishes() {
        let (mut w, t, _old) = world_with_old_npc();
        // VRS 登记该队（签名迁移验证）
        let sig = w.signature_of(t);
        let mut db = csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法");
        db.upsert_state(
            sig.clone(),
            csc_vrs::database::VrsTeamState {
                team_name: "Vitality".into(),
                points: 2000,
                ranking: 1,
                roster: vec![
                    "OldMan".into(),
                    "Young0".into(),
                    "Young1".into(),
                    "Young2".into(),
                    "Young3".into(),
                ],
                last_settled_roster: vec![
                    "OldMan".into(),
                    "Young0".into(),
                    "Young1".into(),
                    "Young2".into(),
                    "Young3".into(),
                ],
                seed_points: 2000,
            },
        );
        let mut vrs = VrsEngine::from_database(db);

        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            None,
        );
        assert_eq!(event.retired.len(), 1);
        assert_eq!(event.intake.len(), 1, "老将退役应补位新秀");
        assert_eq!(event.retired[0].player_name, "OldMan");
        assert_eq!(event.retired[0].age, 35);
        // roster 保持 5 人
        assert_eq!(w.team(t).unwrap().roster_ids.len(), 5);
        // 老将已移除
        assert!(w.player_by_name("OldMan").is_none());
        // 新秀年龄 18~20、档位 Tier4
        let rookie = event.intake[0].player_name.clone();
        let pc = w.player(w.player_by_name(&rookie).unwrap()).unwrap();
        assert!((18..=20).contains(&pc.age));
        // VRS 签名已迁移
        let new_sig = w.signature_of(t);
        assert_ne!(new_sig, sig);
        assert!(vrs.team_of(&new_sig).is_some(), "VRS 已迁移到新签名");
    }

    #[test]
    fn free_agent_veteran_removed_without_intake() {
        let mut w = World::new();
        let old = w
            .create_npc(
                Tier::Tier1,
                None,
                None,
                Some("OldFree"),
                &mut Xoshiro256StarStar::seed(5),
                Some(36),
            )
            .unwrap();
        w.add_free_agent(old).unwrap();
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            None,
        );
        assert_eq!(event.retired.len(), 1);
        assert_eq!(event.intake.len(), 0);
        assert_eq!(event.retired[0].from_team, None);
        assert!(w.player_by_name("OldFree").is_none());
    }

    #[test]
    fn young_npcs_not_retired() {
        let (mut w, _t, _old) = world_with_old_npc();
        // 全部年轻化：把 OldMan 换年轻
        let old_id = w.player_by_name("OldMan").unwrap();
        w.player_mut(old_id).unwrap().age = 25;
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            None,
        );
        assert!(event.is_empty());
    }

    /// 回归（20 年浸泡测试捕获）：名字池仅 40 个，单窗需要 >40 名新秀时
    /// 原查重循环**死转**——现在撞名加数字后缀，必须终止且名字全唯一。
    #[test]
    fn rookie_name_pool_exhaustion_terminates_with_suffixes() {
        let mut w = World::new();
        // 10 队 × 5 名 35 岁老将 = 50 名退役者 → 50 名新秀 > 40 名字池
        for ti in 0..10 {
            let t = w.create_team(format!("Old{ti}"), ti + 1, 2000 - ti * 10);
            for i in 0..5 {
                w.create_npc(
                    Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("O{ti}P{i}")),
                    &mut Xoshiro256StarStar::seed(5),
                    Some(35),
                )
                .unwrap();
            }
        }
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            None,
        );
        assert_eq!(event.retired.len(), 50, "全部老将退役");
        assert_eq!(event.intake.len(), 50, "全部补位新秀");
        // 世界内名字全唯一（后缀兜底）
        let mut names: Vec<&str> = w.players.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        for pair in names.windows(2) {
            assert_ne!(pair[0], pair[1], "新秀名字必须唯一：{}", pair[0]);
        }
        // roster 保持 5 人
        for team in &w.teams {
            assert_eq!(team.roster_ids.len(), 5);
        }
    }

    #[test]
    fn id_stability_across_removal() {
        // remove_npc 的 swap_remove + 引用修复：其余选手 ID 不变
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let keep_ids: Vec<PlayerId> = (0..4)
            .map(|i| {
                w.create_npc(
                    Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("K{i}")),
                    &mut Xoshiro256StarStar::seed(5),
                    Some(20),
                )
                .unwrap()
            })
            .collect();
        let victim = w
            .create_npc(
                Tier::Tier1,
                Some(t),
                None,
                Some("Victim"),
                &mut Xoshiro256StarStar::seed(5),
                Some(35),
            )
            .unwrap();
        assert_eq!(victim, PlayerId(4));
        w.remove_npc(victim).unwrap();
        // 其余选手 ID 不变
        for (i, id) in keep_ids.iter().enumerate() {
            assert_eq!(*id, PlayerId(i as u32), "K{i} ID 应稳定");
            assert_eq!(w.player(*id).unwrap().name, format!("K{i}"));
        }
        // roster 引用全部有效且指向正确选手
        let names: Vec<String> = w.roster(t).iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["K0", "K1", "K2", "K3"]);
    }

    const PROFILE: &str = r#"{
        "generated_at": "x", "source": "test", "sample_size": 672,
        "global": {"mean": 1.021, "sd": 0.103},
        "roles": {},
        "ages": [],
        "rookie": {"mean_offset": -0.02, "sd_scale": 1.0, "clamp": [0.5, 1.7]}
    }"#;

    /// 装配 RatingProfile 后：新秀起始档位按真实分布采样（不再清一色 Tier4）。
    #[test]
    fn rookie_tiers_follow_real_profile_when_loaded() {
        let mut w = World::new();
        // 12 队 × 5 名 35 岁老将 = 60 名退役者 → 60 名新秀（样本足够看分布）
        for ti in 0..12 {
            let t = w.create_team(format!("Old{ti}"), ti + 1, 2000 - ti * 10);
            for i in 0..5 {
                w.create_npc(
                    Tier::Tier1,
                    Some(t),
                    None,
                    Some(&format!("O{ti}P{i}")),
                    &mut Xoshiro256StarStar::seed(5),
                    Some(35),
                )
                .unwrap();
            }
        }
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let profile = csc_entities::baseline::RatingProfile::from_json_str(PROFILE)
            .expect("测试 profile 合法");
        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            Some(&profile),
        );
        assert_eq!(event.intake.len(), 60, "全部补位新秀");
        // 新秀档位应有分布：弱档（Tier3/4）占多数（青训主体），但存在更高档位（天才）
        let tiers: Vec<f64> = event
            .intake
            .iter()
            .map(|i| {
                let pc = w.player(i.player_id).expect("新秀存在");
                csc_entities::power::PowerCalculator::player_power(pc)
            })
            .collect();
        let low = tiers.iter().filter(|p| **p < 68.0).count();
        let high = tiers.iter().filter(|p| **p >= 80.0).count();
        let min = tiers.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = tiers.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        // power 刻度整体偏高（Tier4 也有 62~70），断言用相对梯度而非绝对档位：
        // 青训边缘（<68）必须存在，且出现 ≥80 的天才（Tier1/Tier0 采样发生）
        assert!(low >= 8, "青训边缘新秀应存在（low={low}）");
        assert!(high >= 1, "应有天才档新秀（high={high}）");
        assert!(max - min >= 15.0, "新秀实力梯度应拉开（{min:.0}~{max:.0}）");
    }

    /// 回归（2026 试玩捕获）：主角退役事件的 `age` 曾被新秀年龄变量遮蔽
    /// （36 岁退役写成 18 岁）。断言退役记录年龄 = 主角实际年龄。
    #[test]
    fn protagonist_retirement_event_reports_actual_age() {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let pid = w.create_player(
            Tier::Tier1,
            Some("OldHero"),
            &mut Xoshiro256StarStar::seed(3),
            Some(36),
            None,
        );
        w.assign_player_to_team(pid, t).unwrap();
        for i in 0..4 {
            w.create_npc(
                Tier::Tier1,
                Some(t),
                None,
                Some(&format!("N{i}")),
                &mut Xoshiro256StarStar::seed(3),
                Some(22),
            )
            .unwrap();
        }
        w.player_mut(pid).unwrap().age = 36;
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let event = PopulationEngine::apply_annual_turnover(
            &mut w,
            &mut vrs,
            &mut Xoshiro256StarStar::seed(11),
            None,
        );
        let rec = event
            .retired
            .iter()
            .find(|r| r.player_name == "OldHero")
            .expect("主角退役记录存在");
        assert_eq!(
            rec.age, 36,
            "退役事件年龄必须是主角实际年龄（变量遮蔽回归）"
        );
        // 补位新秀照常生成
        assert_eq!(event.intake.len(), 1, "主角退役应补位新秀");
    }
}
