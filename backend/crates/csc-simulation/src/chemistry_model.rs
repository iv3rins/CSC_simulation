//! 队内关系/矛盾规则模型（Kotlin `ChemistryModel.kt` 转写）——纯函数层。

use csc_entities::chemistry::TeamChemistry;
use csc_util::rng::Xoshiro256StarStar;

/// 玩家对队友失误的反应（决策选项的枚举形态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlunderReaction {
    Support,
    Confront,
    Ignore,
}

impl BlunderReaction {
    /// 中文标签（= Kotlin `label`）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Support => "鼓励队友",
            Self::Confront => "当面指责",
            Self::Ignore => "无视",
        }
    }
}

/// 队内关系/矛盾规则——失误反应 → 关系变化、关系矩阵 → 凝聚力、凝聚力 → 胜率修正。
/// 结算在 `csc-systems::chemistry`。
pub struct ChemistryModel;

impl ChemistryModel {
    /// 关系默认值（未建立关系）
    pub const DEFAULT_RELATION: f64 = 50.0;
    /// 冲突阈值：关系跌破该值触发 Conflict 事件
    pub const CONFLICT_THRESHOLD: f64 = 20.0;
    /// 关系恢复速率（每月 tick 向 50 收敛，防永久仇恨）
    pub const RELATION_RECOVERY: f64 = 2.0;

    // —— 反应选项（与决策点选项 id 对齐）——

    /// 鼓励（支持型）：关系上升、队友士气回升
    pub const REACTION_SUPPORT: &'static str = "SUPPORT";
    /// 指责（毒瘤路径）：关系下降、队友士气崩、冲突风险
    pub const REACTION_CONFRONT: &'static str = "CONFRONT";
    /// 无视
    pub const REACTION_IGNORE: &'static str = "IGNORE";

    /// 由决策选项 id 解析反应类型（未知 → 无视，防御）。
    pub fn reaction_of(option_id: &str) -> BlunderReaction {
        match option_id {
            Self::REACTION_SUPPORT => BlunderReaction::Support,
            Self::REACTION_CONFRONT => BlunderReaction::Confront,
            _ => BlunderReaction::Ignore,
        }
    }

    /// 反应对关系的影响量（基础值；IGNORE 也有轻微损耗 = 冷暴力）。
    pub fn relation_delta(reaction: BlunderReaction) -> f64 {
        match reaction {
            BlunderReaction::Support => 6.0,
            BlunderReaction::Confront => -10.0,
            BlunderReaction::Ignore => -1.5,
        }
    }

    /// 反应对队友士气的影响（SUPPORT 补士气；CONFRONT 崩士气）。
    pub fn teammate_morale_delta(reaction: BlunderReaction) -> i32 {
        match reaction {
            BlunderReaction::Support => 4,
            BlunderReaction::Confront => -6,
            BlunderReaction::Ignore => -1,
        }
    }

    /// 反应对玩家自身士气的影响（指责一时爽，心态微调）。
    pub fn self_morale_delta(reaction: BlunderReaction) -> i32 {
        match reaction {
            BlunderReaction::Support => 1,
            BlunderReaction::Confront => 2,
            BlunderReaction::Ignore => 0,
        }
    }

    /// 失误类型判定（= Kotlin `rollBlunderKind`：`nextInt(4)` 单参数原语）。
    pub fn roll_blunder_kind(rng: &mut Xoshiro256StarStar) -> crate::directives::BlunderKind {
        match rng.next_i32_bound(4) {
            0 => crate::directives::BlunderKind::Choke,
            1 => crate::directives::BlunderKind::Miscarry,
            2 => crate::directives::BlunderKind::Tilt,
            _ => crate::directives::BlunderKind::LackOfFocus,
        }
    }

    /// 凝聚力派生：由关系矩阵平均 + 当前值向目标收敛（避免单事件直接改 cohesion 漂移）。
    pub fn cohesion_of(chemistry: &TeamChemistry, roster_names: &[String]) -> f64 {
        let avg = chemistry.average_relation(roster_names);
        // 目标 = 平均关系；当前 cohesion 向目标移动 30%（平滑，防突变）
        chemistry.cohesion + (avg - chemistry.cohesion) * 0.3
    }

    /// 凝聚力 → 胜率修正乘数（0..100 → 0.92..1.08，50 中性）。
    pub fn cohesion_factor(cohesion: f64) -> f64 {
        1.0 + (cohesion.clamp(0.0, 100.0) - 50.0) / 100.0 * 0.16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directives::BlunderKind;

    #[test]
    fn reaction_mapping_defensive() {
        assert_eq!(
            ChemistryModel::reaction_of("SUPPORT"),
            BlunderReaction::Support
        );
        assert_eq!(
            ChemistryModel::reaction_of("CONFRONT"),
            BlunderReaction::Confront
        );
        assert_eq!(
            ChemistryModel::reaction_of("IGNORE"),
            BlunderReaction::Ignore
        );
        assert_eq!(
            ChemistryModel::reaction_of("NONSENSE"),
            BlunderReaction::Ignore,
            "未知 → 无视"
        );
    }

    #[test]
    fn relation_deltas_match_kotlin() {
        assert_eq!(
            ChemistryModel::relation_delta(BlunderReaction::Support),
            6.0
        );
        assert_eq!(
            ChemistryModel::relation_delta(BlunderReaction::Confront),
            -10.0
        );
        assert_eq!(
            ChemistryModel::relation_delta(BlunderReaction::Ignore),
            -1.5
        );
        assert_eq!(
            ChemistryModel::teammate_morale_delta(BlunderReaction::Support),
            4
        );
        assert_eq!(
            ChemistryModel::teammate_morale_delta(BlunderReaction::Confront),
            -6
        );
        assert_eq!(
            ChemistryModel::self_morale_delta(BlunderReaction::Confront),
            2
        );
    }

    #[test]
    fn roll_blunder_kind_covers_all() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.insert(ChemistryModel::roll_blunder_kind(&mut rng));
        }
        assert_eq!(seen.len(), 4, "四类失误都应出现");
        assert!(seen.contains(&BlunderKind::Choke));
    }

    #[test]
    fn cohesion_converges_toward_average() {
        let mut c = TeamChemistry {
            cohesion: 50.0,
            ..Default::default()
        };
        // 关系全部 80 → avg=80 → cohesion 向 80 移动 30%
        c.adjust("a", "b", 30.0); // a↔b = 80
        let roster = vec!["a".to_string(), "b".to_string()];
        let after = ChemistryModel::cohesion_of(&c, &roster);
        assert!(
            (after - 59.0).abs() < 1e-9,
            "50 + (80-50)*0.3 = 59: {after}"
        );
    }

    #[test]
    fn cohesion_factor_range() {
        assert_eq!(ChemistryModel::cohesion_factor(50.0), 1.0);
        assert!((ChemistryModel::cohesion_factor(100.0) - 1.08).abs() < 1e-12);
        assert!((ChemistryModel::cohesion_factor(0.0) - 0.92).abs() < 1e-12);
        // clamp：超出范围按边界
        assert_eq!(ChemistryModel::cohesion_factor(150.0), 1.08);
    }
}
