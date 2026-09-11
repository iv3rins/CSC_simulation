//! 把 VRS 排名数据映射到游戏实体（Kotlin `VrsMapper.kt` 的转写，M2 遗留补全）。
//!
//! 因果方向：**选手先有个人的强度与风格，强队由强选手组成**——
//! 队伍排名只校准「队伍基准档位」，队内按阵容槽位拉开实力梯度，
//! 位置分配：JSON 基准（RoleBaseline）真实 role 优先，否则按槽位。

use csc_domain::VrsEntry;
use csc_domain::tier::Tier;
use csc_util::rng::Xoshiro256StarStar;

use crate::baseline::RoleBaseline;
use crate::character::PlayerCharacter;
use crate::generator::RandomPlayerGenerator;
use crate::role::Role;

/// VRS 数据 → 游戏实体映射。
pub struct VrsMapper;

impl VrsMapper {
    /// 档位顺序（用于队内梯度移动；= Kotlin `Tier.entries`）。
    const TIER_ORDER: [Tier; 5] = [
        Tier::Tier0,
        Tier::Tier1,
        Tier::Tier2,
        Tier::Tier3,
        Tier::Tier4,
    ];

    /// 队伍排名 → 选手基准档位（= Kotlin `tierForRanking`；档位边界与 TeamTier 常量一致）。
    pub fn tier_for_ranking(ranking: i32) -> Tier {
        match ranking {
            1..=4 => Tier::Tier0,    // 世界前 4 → 顶级
            5..=12 => Tier::Tier1,   // 5~12 → 一线（T1_TEAM_COUNT）
            13..=32 => Tier::Tier2,  // 13~32 → 中游（T2_MAX_RANKING）
            33..=120 => Tier::Tier3, // 33~120 → 下游（T3_MAX_RANKING）
            _ => Tier::Tier4,        // 121+ → 新人档
        }
    }

    /// 阵容槽位 → 队内实力梯度偏移（= Kotlin `slotTierOffset`）。
    pub fn slot_tier_offset(index: usize) -> i32 {
        match index {
            1 | 2 => 1, // AWP 核心 / 头号步枪手（+1 档）
            0 | 3 => 0, // IGL / 突破手（基准）
            _ => -1,    // 辅助/替补（-1 档）
        }
    }

    /// 在档位顺序上移动 offset 步（clamp 到 TIER0..TIER4；= Kotlin `shiftTier`）。
    pub fn shift_tier(tier: Tier, offset: i32) -> Tier {
        let idx = Self::TIER_ORDER
            .iter()
            .position(|t| *t == tier)
            .map(|i| (i as i32 + offset).clamp(0, Self::TIER_ORDER.len() as i32 - 1))
            .unwrap_or(0) as usize;
        Self::TIER_ORDER[idx]
    }

    /// 阵容槽位 → 角色（标准 5 人结构；= Kotlin `roleForSlot`）。
    pub fn role_for_slot(index: usize) -> Role {
        match index {
            0 => Role::Igl,     // 第 1 人：指挥
            1 => Role::Awp,     // 第 2 人：狙击手
            2 => Role::Rifler,  // 第 3 人：步枪手
            3 => Role::Entry,   // 第 4 人：突破手
            _ => Role::Support, // 其余：辅助
        }
    }

    /// VrsEntry → 该队全体选手（career=None、team=None 占位；由 World 登记时分配 ID 与归属）。
    ///
    /// 位置分配：JSON 基准优先（真实 HLTV role），否则按槽位。
    /// **档位分配（2026 修订）**：真实评级先验（`player_ratings.json`）优先——
    /// 有 Rating 数据的选手按 [`RatingBaseline::tier_for_rating`] 校准档位，
    /// 无数据回退「队伍基准 + 槽位梯度」。修复「真实选手名只是皮肤、zont1x
    /// 随机打出 1.43 登顶」的输入失真。
    /// **队内去重**：真实 role 可能重复（多 Opener→Entry、多 Closer→Lurker），
    /// 保证最终 5 人 5 个不同角色——候选序 = [真实 role, 槽位 role]，取首个未占用者；
    /// 仍撞车则从 `ALL_ROLES` 取第一个空闲角色兜底。
    /// `rng` 必须为确定性源（初始装载是世界的起点）。
    pub fn to_players(
        entry: &VrsEntry,
        baseline: Option<&RoleBaseline>,
        ratings: Option<&crate::baseline::RatingBaseline>,
        rng: &mut Xoshiro256StarStar,
    ) -> Vec<PlayerCharacter> {
        let base_tier = Self::tier_for_ranking(entry.ranking); // 队伍基准档位（由排名校准）
        let mut used: std::collections::HashSet<Role> = std::collections::HashSet::new();
        entry
            .roster
            .iter()
            .enumerate()
            .map(|(index, name)| {
                // 候选序：真实 role → 槽位 role → 任意空闲 role
                let real = baseline.and_then(|b| b.role_of(name));
                let slot = Self::role_for_slot(index);
                let role = [real, Some(slot)]
                    .into_iter()
                    .flatten()
                    .find(|r| !used.contains(r))
                    .or_else(|| {
                        crate::role::ALL_ROLES
                            .iter()
                            .copied()
                            .find(|r| !used.contains(r))
                    })
                    .unwrap_or(slot);
                used.insert(role);
                // 档位：真实评级先验优先；否则队伍基准 + 槽位梯度
                let tier = ratings
                    .and_then(|r| r.rating_of(name))
                    .map(crate::baseline::RatingBaseline::tier_for_rating)
                    .unwrap_or_else(|| Self::shift_tier(base_tier, Self::slot_tier_offset(index)));
                RandomPlayerGenerator::generate_npc(
                    tier, // 个人档位 = 真实评级先验 或 基准 + 槽位梯度
                    None, // 生成期临时归属，登记时统一改挂
                    Some(role),
                    Some(name), // 保留真实选手名
                    rng,
                    baseline.and_then(|b| b.age_of(name)), // JSON 真实年龄优先；无基准 = 引擎随机
                )
            })
            .collect()
    }

    /// 初始预算映射：VRS 积分 × 1000（如 2000 分 → 200 万财政；= Kotlin `initialBudget`）。
    pub fn initial_budget(vrs_points: i32) -> i64 {
        vrs_points as i64 * 1_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_for_ranking_boundaries() {
        assert_eq!(VrsMapper::tier_for_ranking(1), Tier::Tier0);
        assert_eq!(VrsMapper::tier_for_ranking(4), Tier::Tier0);
        assert_eq!(VrsMapper::tier_for_ranking(5), Tier::Tier1);
        assert_eq!(VrsMapper::tier_for_ranking(12), Tier::Tier1);
        assert_eq!(VrsMapper::tier_for_ranking(13), Tier::Tier2);
        assert_eq!(VrsMapper::tier_for_ranking(32), Tier::Tier2);
        assert_eq!(VrsMapper::tier_for_ranking(33), Tier::Tier3);
        assert_eq!(VrsMapper::tier_for_ranking(120), Tier::Tier3);
        assert_eq!(VrsMapper::tier_for_ranking(121), Tier::Tier4);
        assert_eq!(VrsMapper::tier_for_ranking(999), Tier::Tier4);
    }

    #[test]
    fn slot_offsets_and_roles() {
        assert_eq!(VrsMapper::slot_tier_offset(1), 1);
        assert_eq!(VrsMapper::slot_tier_offset(2), 1);
        assert_eq!(VrsMapper::slot_tier_offset(0), 0);
        assert_eq!(VrsMapper::slot_tier_offset(3), 0);
        assert_eq!(VrsMapper::slot_tier_offset(4), -1);
        assert_eq!(VrsMapper::slot_tier_offset(5), -1);
        assert_eq!(VrsMapper::role_for_slot(0), Role::Igl);
        assert_eq!(VrsMapper::role_for_slot(1), Role::Awp);
        assert_eq!(VrsMapper::role_for_slot(2), Role::Rifler);
        assert_eq!(VrsMapper::role_for_slot(3), Role::Entry);
        assert_eq!(VrsMapper::role_for_slot(4), Role::Support);
        assert_eq!(VrsMapper::role_for_slot(9), Role::Support);
    }

    #[test]
    fn shift_tier_clamps() {
        // TIER 数字越大越弱：+1 向弱移动（Kotlin tierOrder 语义）
        assert_eq!(VrsMapper::shift_tier(Tier::Tier2, 1), Tier::Tier3);
        assert_eq!(VrsMapper::shift_tier(Tier::Tier2, -1), Tier::Tier1);
        // clamp：+offset 上界 = 最弱档 Tier4；-offset 下界 = 最强档 Tier0
        assert_eq!(
            VrsMapper::shift_tier(Tier::Tier0, 5),
            Tier::Tier4,
            "上界 clamp"
        );
        assert_eq!(
            VrsMapper::shift_tier(Tier::Tier4, -5),
            Tier::Tier0,
            "下界 clamp"
        );
    }

    #[test]
    fn to_players_uses_baseline_role_first() {
        let entry = VrsEntry::new(
            3,
            2000,
            "Vitality",
            vec![
                "apEX".into(),
                "ZywOo".into(),
                "ropz".into(),
                "mezii".into(),
                "flameZ".into(),
            ],
        );
        let baseline = RoleBaseline::from_json_str(
            r#"{"players":[{"player":"ZywOo","team":"Vitality","role":"AWPer","ctRole":"AWPer","tRole":"AWPer"}]}"#,
        )
        .unwrap();
        let mut rng = Xoshiro256StarStar::seed(42);
        let players = VrsMapper::to_players(&entry, Some(&baseline), None, &mut rng);
        assert_eq!(players.len(), 5);
        assert_eq!(players[0].name, "apEX");
        assert_eq!(players[0].role, Role::Igl, "无基准 → 槽位 0 = IGL");
        assert_eq!(players[1].name, "ZywOo");
        assert_eq!(players[1].role, Role::Awp, "JSON 基准优先");
        assert_eq!(players[2].role, Role::Rifler);
        // 全部为 NPC（career None）
        assert!(players.iter().all(|p| p.is_npc()));
    }

    #[test]
    fn initial_budget_mapping() {
        assert_eq!(VrsMapper::initial_budget(2000), 2_000_000);
        assert_eq!(VrsMapper::initial_budget(500), 500_000);
    }
}
