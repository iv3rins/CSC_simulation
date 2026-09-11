//! 队伍级别 + 静态分级规则（Kotlin `TeamTier.kt`）。

use serde::{Deserialize, Serialize};

/// 队伍级别（按 VRS 全球排名划分，决定可参加的赛事级别）。
///
/// 依据 Valve 官方手册与真实梯度：
/// - T1：**连续三个月**入选前 12 的队伍（当月 + 前两个月均在前 12）
/// - T2：排名 13~32（12 名以内即 T1 的队伍无法报名 T2 级别赛事）
/// - T3：排名 33~120（32 名开外）
/// - T4：排名 120 名开外
///
/// 本枚举承载「单月排名 → 级别」的静态分级规则（无数据库依赖）；
/// 依赖历史数据的判定（连续三个月前 12）由 VRS 子系统负责。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TeamTier {
    /// 顶级：连续三个月前 12（单月分级时即 1~12 名）。
    T1,
    /// 次顶级：13~32 名。
    T2,
    /// 中游：33~120 名。
    T3,
    /// 下游：120 名开外。
    T4,
}

impl TeamTier {
    /// T1 名额上限（同时也是 T2 的起始排名前一位）。
    pub const T1_TEAM_COUNT: i32 = 12;

    /// T2 赛事最高可报名排名（12 名以内不能报 T2）。
    pub const T2_MAX_RANKING: i32 = 32;

    /// T3 赛事最高排名（120 名开外为 T4）。
    pub const T3_MAX_RANKING: i32 = 120;

    /// 单月快速分级（无历史数据时的兜底）。
    /// 1~12 → T1，13~32 → T2，33~120 → T3，其余（含 <1 的非法排名）→ T4。
    /// 边界与 Kotlin `when` 分支完全一致（常量见 `T1_TEAM_COUNT` 等）。
    pub fn from_ranking(ranking: i32) -> Self {
        match ranking {
            1..=12 => Self::T1,
            13..=32 => Self::T2,
            33..=120 => Self::T3,
            _ => Self::T4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_boundaries() {
        assert_eq!(TeamTier::from_ranking(1), TeamTier::T1);
        assert_eq!(TeamTier::from_ranking(12), TeamTier::T1);
        assert_eq!(TeamTier::from_ranking(13), TeamTier::T2);
        assert_eq!(TeamTier::from_ranking(32), TeamTier::T2);
        assert_eq!(TeamTier::from_ranking(33), TeamTier::T3);
        assert_eq!(TeamTier::from_ranking(120), TeamTier::T3);
        assert_eq!(TeamTier::from_ranking(121), TeamTier::T4);
        assert_eq!(TeamTier::from_ranking(9999), TeamTier::T4);
    }

    #[test]
    fn ranking_out_of_range_below_one() {
        // 语义与 Kotlin when 一致：非正排名落到 else 分支（T4）
        assert_eq!(TeamTier::from_ranking(0), TeamTier::T4);
        assert_eq!(TeamTier::from_ranking(-5), TeamTier::T4);
    }

    #[test]
    fn constants_match_kotlin() {
        assert_eq!(TeamTier::T1_TEAM_COUNT, 12);
        assert_eq!(TeamTier::T2_MAX_RANKING, 32);
        assert_eq!(TeamTier::T3_MAX_RANKING, 120);
    }

    #[test]
    fn serde_names_match_kotlin() {
        assert_eq!(serde_json::to_string(&TeamTier::T2).unwrap(), r#""T2""#);
    }
}
