//! 选手实力档位 + 各档位属性区间表（Kotlin `Tier.kt`）。

use serde::{Deserialize, Serialize};

use crate::attr_range::AttrRange;

/// 选手实力档位（Tier）。
///
/// 现实选手用「客观锚点分档」标定：先按 HLTV Rating / VRS 排名 / 战队成绩
/// 确定选手所在档位，再在 [`TierRanges`] 对应档位的区间内按风格取值。
/// 虚拟选手则在指定档位区间内随机生成。
///
/// 数值约定 0..100。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tier {
    /// 世界顶级（NiKo 等常年 TOP 选手）
    Tier0,
    /// 一线（Major 八强 / S-Tier 冠军主力）
    Tier1,
    /// 中游（中游战队核心）
    Tier2,
    /// 下游（次级联赛主力，如 deko）
    Tier3,
    /// 新人 / 青训
    Tier4,
}

/// 单个档位的各属性取值区间。
///
/// 区间刻意设计得较宽：档位只锚定队伍/选手的「平均强度」，
/// 内部留出足够空间让角色（Role）差异化展开——
/// 同一 Tier0 里 AWP 可到 97、IGL 可只有 84，风格差距真实存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TierRanges {
    // —— 基础属性 ——
    /// 反应时间区间
    pub reaction: AttrRange,
    /// 稳定性区间
    pub stability: AttrRange,
    /// 体能区间
    pub endurance: AttrRange,
    /// 精力区间
    pub stamina: AttrRange,
    /// 健康区间
    pub health: AttrRange,
    // —— 技能属性 ——
    /// 瞄准区间
    pub aim: AttrRange,
    /// 领导力区间
    pub leader: AttrRange,
    /// 沟通区间
    pub communication: AttrRange,
    /// 残局区间
    pub clutch: AttrRange,
    // —— 心理属性 ——
    /// 抗压区间
    pub mentality: AttrRange,
    /// 自信区间
    pub confidence: AttrRange,
    /// 团队配合区间
    pub team_spirit: AttrRange,
    /// 忠诚度区间
    pub loyalty: AttrRange,
    /// 士气区间
    pub morale: AttrRange,
    // —— 武器熟练度 ——
    /// AK 熟练度区间
    pub ak: AttrRange,
    /// AWP 熟练度区间
    pub awp: AttrRange,
    /// 手枪熟练度区间
    pub pistol: AttrRange,
    /// 道具熟练度区间
    pub smoke: AttrRange,
    /// 战术/地图理解区间
    pub utility: AttrRange,
}

// —— 各档位区间静态表（Kotlin `TierTable.ranges` 的 `Map` → 编译期常量表）——

/// Tier0：世界顶级，各区间上限最高
static TIER_RANGES_TIER0: TierRanges = TierRanges {
    reaction: AttrRange::new(80, 98),
    stability: AttrRange::new(82, 98),
    endurance: AttrRange::new(82, 96),
    stamina: AttrRange::new(80, 96),
    health: AttrRange::new(82, 96),
    aim: AttrRange::new(82, 98),
    leader: AttrRange::new(60, 92),
    communication: AttrRange::new(60, 90),
    clutch: AttrRange::new(75, 96),
    mentality: AttrRange::new(75, 95),
    confidence: AttrRange::new(75, 95),
    team_spirit: AttrRange::new(65, 90),
    loyalty: AttrRange::new(60, 88),
    morale: AttrRange::new(70, 92),
    ak: AttrRange::new(80, 98),
    awp: AttrRange::new(55, 95),
    pistol: AttrRange::new(72, 92),
    smoke: AttrRange::new(55, 88),
    utility: AttrRange::new(60, 90),
};

/// Tier1：一线，整体下调约 4~6 点
static TIER_RANGES_TIER1: TierRanges = TierRanges {
    reaction: AttrRange::new(76, 94),
    stability: AttrRange::new(78, 94),
    endurance: AttrRange::new(78, 92),
    stamina: AttrRange::new(76, 92),
    health: AttrRange::new(78, 92),
    aim: AttrRange::new(78, 94),
    leader: AttrRange::new(56, 88),
    communication: AttrRange::new(56, 86),
    clutch: AttrRange::new(71, 92),
    mentality: AttrRange::new(71, 91),
    confidence: AttrRange::new(71, 91),
    team_spirit: AttrRange::new(62, 86),
    loyalty: AttrRange::new(57, 84),
    morale: AttrRange::new(66, 88),
    ak: AttrRange::new(76, 94),
    awp: AttrRange::new(51, 91),
    pistol: AttrRange::new(68, 88),
    smoke: AttrRange::new(52, 84),
    utility: AttrRange::new(57, 86),
};

/// Tier2：中游，再下调约 4 点
static TIER_RANGES_TIER2: TierRanges = TierRanges {
    reaction: AttrRange::new(72, 90),
    stability: AttrRange::new(74, 90),
    endurance: AttrRange::new(74, 88),
    stamina: AttrRange::new(72, 88),
    health: AttrRange::new(74, 88),
    aim: AttrRange::new(74, 90),
    leader: AttrRange::new(52, 84),
    communication: AttrRange::new(52, 82),
    clutch: AttrRange::new(67, 88),
    mentality: AttrRange::new(67, 87),
    confidence: AttrRange::new(67, 87),
    team_spirit: AttrRange::new(58, 82),
    loyalty: AttrRange::new(54, 80),
    morale: AttrRange::new(62, 84),
    ak: AttrRange::new(72, 90),
    awp: AttrRange::new(47, 87),
    pistol: AttrRange::new(64, 84),
    smoke: AttrRange::new(48, 80),
    utility: AttrRange::new(54, 82),
};

/// Tier3：下游（次级联赛主力）
static TIER_RANGES_TIER3: TierRanges = TierRanges {
    reaction: AttrRange::new(68, 86),
    stability: AttrRange::new(70, 86),
    endurance: AttrRange::new(70, 84),
    stamina: AttrRange::new(68, 84),
    health: AttrRange::new(70, 84),
    aim: AttrRange::new(70, 86),
    leader: AttrRange::new(48, 80),
    communication: AttrRange::new(48, 78),
    clutch: AttrRange::new(63, 84),
    mentality: AttrRange::new(63, 83),
    confidence: AttrRange::new(63, 83),
    team_spirit: AttrRange::new(54, 78),
    loyalty: AttrRange::new(50, 76),
    morale: AttrRange::new(58, 80),
    ak: AttrRange::new(68, 86),
    awp: AttrRange::new(43, 83),
    pistol: AttrRange::new(60, 80),
    smoke: AttrRange::new(44, 76),
    utility: AttrRange::new(50, 78),
};

/// Tier4：新人 / 青训，区间最低
static TIER_RANGES_TIER4: TierRanges = TierRanges {
    reaction: AttrRange::new(62, 82),
    stability: AttrRange::new(64, 82),
    endurance: AttrRange::new(64, 80),
    stamina: AttrRange::new(62, 80),
    health: AttrRange::new(66, 82),
    aim: AttrRange::new(64, 82),
    leader: AttrRange::new(42, 76),
    communication: AttrRange::new(42, 74),
    clutch: AttrRange::new(57, 80),
    mentality: AttrRange::new(57, 79),
    confidence: AttrRange::new(57, 79),
    team_spirit: AttrRange::new(50, 74),
    loyalty: AttrRange::new(46, 72),
    morale: AttrRange::new(52, 76),
    ak: AttrRange::new(62, 82),
    awp: AttrRange::new(37, 79),
    pistol: AttrRange::new(54, 76),
    smoke: AttrRange::new(38, 72),
    utility: AttrRange::new(44, 74),
};

/// 各档位的属性区间表（可按真实比赛数据微调）。
///
/// 相邻档位中心约差 5~6 点：Tier0 中心最高，逐级下降，
/// 保证「强队由强选手组成」的排名锚定仍然成立。
///
/// 设计差异：Kotlin 用 `object TierTable` + `Map<Tier, TierRanges>`；
/// Rust 用**静态表 + match 返回 `&'static`**——零堆分配、可 `const`，
/// 语义完全一致（查表逻辑不变）。
pub struct TierTable;

impl TierTable {
    /// tier → 该档位全部 19 项属性的取值区间（Kotlin `TierTable.ranges` 的静态形态）。
    pub fn ranges(tier: Tier) -> &'static TierRanges {
        match tier {
            Tier::Tier0 => &TIER_RANGES_TIER0,
            Tier::Tier1 => &TIER_RANGES_TIER1,
            Tier::Tier2 => &TIER_RANGES_TIER2,
            Tier::Tier3 => &TIER_RANGES_TIER3,
            Tier::Tier4 => &TIER_RANGES_TIER4,
        }
    }

    /// 取某档位的区间（Kotlin `TierTable.of`）。
    #[inline]
    pub fn of(tier: Tier) -> &'static TierRanges {
        Self::ranges(tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ranges_valid_and_contained() {
        for tier in [
            Tier::Tier0,
            Tier::Tier1,
            Tier::Tier2,
            Tier::Tier3,
            Tier::Tier4,
        ] {
            let r = TierTable::of(tier);
            // 所有区间合法（lo <= hi）且在 0..=100 数值约定内
            let ranges = [
                r.reaction,
                r.stability,
                r.endurance,
                r.stamina,
                r.health,
                r.aim,
                r.leader,
                r.communication,
                r.clutch,
                r.mentality,
                r.confidence,
                r.team_spirit,
                r.loyalty,
                r.morale,
                r.ak,
                r.awp,
                r.pistol,
                r.smoke,
                r.utility,
            ];
            for range in ranges {
                assert!(!range.is_empty(), "tier {tier:?} 含空区间");
                assert!(
                    range.lo >= 0 && range.hi <= 100,
                    "tier {tier:?} 超出 0..100 约定: {range:?}"
                );
            }
        }
    }

    #[test]
    fn tier_centers_decrease_monotonically() {
        // 相邻档位中心约差 5~6 点：以 aim 为代表验证单调递减
        let centers: Vec<i32> = (0..=4)
            .map(|i| {
                let tier = match i {
                    0 => Tier::Tier0,
                    1 => Tier::Tier1,
                    2 => Tier::Tier2,
                    3 => Tier::Tier3,
                    _ => Tier::Tier4,
                };
                TierTable::of(tier).aim.mid()
            })
            .collect();
        for w in centers.windows(2) {
            assert!(w[0] > w[1], "aim 中心应逐档递减: {centers:?}");
        }
        // 与 Kotlin 表完全一致：aim 中心 90/86/82/78/73
        assert_eq!(centers, vec![90, 86, 82, 78, 73]);
    }

    #[test]
    fn tier0_awp_spread_allows_role_differentiation() {
        // Tier0 里 AWP 可到 95、IGL 的 leader 下限 60 —— 区间内差异真实存在
        let r = TierTable::of(Tier::Tier0);
        assert_eq!(r.awp.hi, 95);
        assert_eq!(r.leader.lo, 60);
    }

    #[test]
    fn serde_roundtrip() {
        let r = TierTable::of(Tier::Tier2);
        let json = serde_json::to_string(r).unwrap();
        let back: TierRanges = serde_json::from_str(&json).unwrap();
        assert_eq!(*r, back);
    }
}
