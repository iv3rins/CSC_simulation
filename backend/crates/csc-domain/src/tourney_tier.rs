//! 赛事等级（Kotlin `TourneyTier.kt`）。

use serde::{Deserialize, Serialize};

/// 赛事等级（赛事域概念，衡量团队冠军分量与积分权重）。
///
/// 与队伍级别 [`crate::team_tier::TeamTier`]（按 VRS 排名划分的实力层级）
/// 是两个独立概念：前者描述「一场赛事有多重要」，后者描述「一支队伍有多强」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TourneyTier {
    /// 顶级赛事（Major）
    Major,
    /// 卡托维兹 / 科隆（超级精英赛）
    #[serde(rename = "SUPERELITE")]
    SuperElite,
    /// 大型 Premier 级职业联赛（如 EPL / BLAST 决赛圈）
    #[serde(rename = "ELITE")]
    Elite,
    /// 一级国际赛事
    T1,
    /// 二级赛事
    T2,
    /// 预选赛
    Qualify,
}

impl TourneyTier {
    /// 枚举名（= Kotlin `name`，如 "MAJOR"）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Major => "MAJOR",
            Self::SuperElite => "SUPERELITE",
            Self::Elite => "ELITE",
            Self::T1 => "T1",
            Self::T2 => "T2",
            Self::Qualify => "QUALIFY",
        }
    }

    /// 顶级赛事（Major / 超级精英 / Elite / T1）——MVP/EVP 录入资格与
    /// 场内决策弹窗的门槛。T2 / 预选赛自动模拟，不打扰玩家。
    pub const fn is_elite(self) -> bool {
        matches!(
            self,
            Self::Major | Self::SuperElite | Self::Elite | Self::T1
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_names_match_kotlin() {
        assert_eq!(
            serde_json::to_string(&TourneyTier::Major).unwrap(),
            r#""MAJOR""#
        );
        assert_eq!(
            serde_json::to_string(&TourneyTier::SuperElite).unwrap(),
            r#""SUPERELITE""#
        );
        assert_eq!(
            serde_json::to_string(&TourneyTier::Qualify).unwrap(),
            r#""QUALIFY""#
        );
    }
}
