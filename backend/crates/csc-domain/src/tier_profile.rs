//! Tier 赛事画像：奖池分级 + 线下/线上规则 + 系列赛制（Kotlin `TierProfile.kt`）。

use serde::{Deserialize, Serialize};

use crate::match_result::MatchVenue;
use crate::tourney_tier::TourneyTier;

/// Tier 赛事画像：奖池分级 + 线下/线上规则 + 系列赛制（赛事域配置）。
///
/// 规则（与官方数据源字段对应，原型用 Tier 近似）：
/// - **T1 及以上（MAJOR / SUPERELITE / T1）全部线下**；
/// - **T2 / T3（原型用 QUALIFY 表示 T3 网吧赛）全部线上**。
///
/// 奖池按 Tier 分级：MAJOR/SUPERELITE $1,000,000（官方封顶 $1M）、T1 $500,000、
/// T2 $100,000、QUALIFY（T3 预选/网吧赛）$10,000。
///
/// 系列赛制（best of）：MAJOR/SUPERELITE/T1/T2 为 bo3（Major 决赛可传 bo5 覆盖）、
/// QUALIFY 为 bo1。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TierProfile {
    /// 奖池（美元）
    pub prize_pool: i32,
    /// 是否线下
    pub lan: bool,
    /// 系列赛地图数上限（1 / 3 / 5）
    pub best_of: i32,
}

/// 各 Tier 的奖池与线下/线上/赛制配置。
///
/// 设计差异：Kotlin 用顶层 `val TIER_PROFILES: Map<TourneyTier, TierProfile>`；
/// Rust 用**静态表 + match 返回 `&'static`**（零堆分配、可 `const` 展开），
/// 查表语义一致。`TIER_PROFILES` 常量保留为兼容性窗口（返回引用不可变表）。
static TIER_PROFILE_MAJOR: TierProfile = TierProfile {
    prize_pool: 1_250_000,
    lan: true,
    best_of: 3,
};
static TIER_PROFILE_SUPERELITE: TierProfile = TierProfile {
    prize_pool: 1_000_000,
    lan: true,
    best_of: 3,
};
static TIER_PROFILE_ELITE: TierProfile = TierProfile {
    prize_pool: 750_000,
    lan: true,
    best_of: 3,
};
static TIER_PROFILE_T1: TierProfile = TierProfile {
    prize_pool: 500_000,
    lan: true,
    best_of: 3,
};
static TIER_PROFILE_T2: TierProfile = TierProfile {
    prize_pool: 100_000,
    lan: false,
    best_of: 3,
};
static TIER_PROFILE_QUALIFY: TierProfile = TierProfile {
    prize_pool: 10_000,
    lan: false,
    best_of: 1,
};
pub fn tier_profiles(tier: TourneyTier) -> &'static TierProfile {
    match tier {
        TourneyTier::Major => &TIER_PROFILE_MAJOR,
        TourneyTier::SuperElite => &TIER_PROFILE_SUPERELITE,
        TourneyTier::Elite => &TIER_PROFILE_ELITE,
        TourneyTier::T1 => &TIER_PROFILE_T1,
        TourneyTier::T2 => &TIER_PROFILE_T2,
        TourneyTier::Qualify => &TIER_PROFILE_QUALIFY,
    }
}

/// 赛事等级 → 估计奖池（美元）：按 [`tier_profiles`] 分级。
#[inline]
pub fn tier_prize_pool(tier: TourneyTier) -> i32 {
    tier_profiles(tier).prize_pool
}

/// 赛事等级 → 是否线下（官方 event.lan）：T1 及以上线下，T2/T3 线上。
#[inline]
pub fn tier_lan(tier: TourneyTier) -> bool {
    tier_profiles(tier).lan
}

/// 赛事等级 → 系列赛地图数上限（1 / 3 / 5）：T1/T2 为 bo3，T3 预选为 bo1。
#[inline]
pub fn tier_best_of(tier: TourneyTier) -> i32 {
    tier_profiles(tier).best_of
}

/// 赛事等级 → 默认比赛形式（线下→[`MatchVenue::Lan`]、线上→[`MatchVenue::Online`]）。
#[inline]
pub fn tier_venue(tier: TourneyTier) -> MatchVenue {
    if tier_lan(tier) {
        MatchVenue::Lan
    } else {
        MatchVenue::Online
    }
}

/// 赛事等级 → 玩家体验重要度（`Major→Championship`、`SuperElite/Elite→Major`、
/// `T1→Important`、`T2/Qualify→Background`）。
///
/// 本映射是 **tier → `EventImportance` 的单一事实源**：排程面（`build_event` 写入
/// `scheduled.importance`）、结算/运行面（engine 的 `scheduled_importance` 重构、
/// conductor 无透传时的推导）统一调用此处，口径只维护一份。
#[inline]
pub fn tier_importance(tier: TourneyTier) -> crate::event_importance::EventImportance {
    match tier {
        TourneyTier::Major => crate::event_importance::EventImportance::Championship,
        TourneyTier::SuperElite | TourneyTier::Elite => {
            crate::event_importance::EventImportance::Major
        }
        TourneyTier::T1 => crate::event_importance::EventImportance::Important,
        TourneyTier::T2 | TourneyTier::Qualify => {
            crate::event_importance::EventImportance::Background
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prize_pool_table_matches_kotlin() {
        assert_eq!(tier_prize_pool(TourneyTier::Major), 1_250_000);
        assert_eq!(tier_prize_pool(TourneyTier::SuperElite), 1_000_000);
        assert_eq!(tier_prize_pool(TourneyTier::Elite), 750_000);
        assert_eq!(tier_prize_pool(TourneyTier::T1), 500_000);
        assert_eq!(tier_prize_pool(TourneyTier::T2), 100_000);
        assert_eq!(tier_prize_pool(TourneyTier::Qualify), 10_000);
    }

    #[test]
    fn lan_and_best_of_table_matches_kotlin() {
        assert!(tier_lan(TourneyTier::Major));
        assert!(tier_lan(TourneyTier::SuperElite));
        assert!(tier_lan(TourneyTier::Elite));
        assert!(tier_lan(TourneyTier::T1));
        assert!(!tier_lan(TourneyTier::T2));
        assert!(!tier_lan(TourneyTier::Qualify));

        assert_eq!(tier_best_of(TourneyTier::Major), 3);
        assert_eq!(tier_best_of(TourneyTier::T1), 3);
        assert_eq!(tier_best_of(TourneyTier::T2), 3);
        assert_eq!(tier_best_of(TourneyTier::Qualify), 1);
    }

    #[test]
    fn tier_venue_default_matches_kotlin() {
        assert_eq!(tier_venue(TourneyTier::T1), MatchVenue::Lan);
        assert_eq!(tier_venue(TourneyTier::T2), MatchVenue::Online);
        assert_eq!(tier_venue(TourneyTier::Qualify), MatchVenue::Online);
    }
}
