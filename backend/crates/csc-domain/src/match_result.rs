//! 比赛形式 + 一场比赛的结果（Kotlin `MatchResult.kt`）。

use serde::{Deserialize, Serialize};

use crate::tier_profile::tier_venue;
use crate::tourney_tier::TourneyTier;

/// 比赛形式（赛事域概念）。
///
/// 由赛事系统决定（线下场馆 / 线上 / 网吧赛），
/// 供 VRS 结算时判定 LAN 因子（见 [`venue_lan`]）与历史记录使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchVenue {
    /// 线下场馆赛（Major / S-Tier）
    Lan,
    /// 线上赛（T2 常规赛）
    Online,
    /// 网吧赛（T3）
    LanCafe,
}

/// 一场比赛的结果（赛事域概念，由赛事管理器在模拟中生成）。
///
/// 以队伍签名为标识（见 `csc-entities::Signatures::team_signature`），
/// 由赛事子系统生成并提交给 VRS 子系统结算积分。
///
/// `venue` 默认按 `tier` 推导（见 [`tier_venue`]）：T1 及以上线下、T2/T3 线上；
/// 需要特殊场地（如网吧赛）时用 `new` 构造后显式覆盖 `venue` 字段即可
/// （与 Kotlin 的命名参数默认值语义一致）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchResult {
    /// 胜者队伍签名。
    pub winner_sig: String,
    /// 败者队伍签名。
    pub loser_sig: String,
    /// 赛事等级（决定奖池分级与线下/线上，见 `TIER_PROFILES`）。
    pub tier: TourneyTier,
    /// 比赛形式，默认跟随 tier。
    pub venue: MatchVenue,
}

impl MatchResult {
    /// 构造比赛结果（`venue` 按 `tier` 推导默认值，等价 Kotlin 默认参数）。
    pub fn new(
        winner_sig: impl Into<String>,
        loser_sig: impl Into<String>,
        tier: TourneyTier,
    ) -> Self {
        Self {
            winner_sig: winner_sig.into(),
            loser_sig: loser_sig.into(),
            tier,
            venue: tier_venue(tier),
        }
    }
}

/// 比赛形式 → 是否线下（物理形式映射，与 `tier_lan` 的 Tier 默认规则正交）。
pub fn venue_lan(venue: MatchVenue) -> bool {
    match venue {
        MatchVenue::Lan | MatchVenue::LanCafe => true, // 线下场馆 / 网吧赛均为线下
        MatchVenue::Online => false,                   // 线上赛
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_default_follows_tier() {
        // T1 及以上线下
        assert_eq!(
            MatchResult::new("A", "B", TourneyTier::Major).venue,
            MatchVenue::Lan
        );
        assert_eq!(
            MatchResult::new("A", "B", TourneyTier::SuperElite).venue,
            MatchVenue::Lan
        );
        assert_eq!(
            MatchResult::new("A", "B", TourneyTier::T1).venue,
            MatchVenue::Lan
        );
        // T2/T3 线上
        assert_eq!(
            MatchResult::new("A", "B", TourneyTier::T2).venue,
            MatchVenue::Online
        );
        assert_eq!(
            MatchResult::new("A", "B", TourneyTier::Qualify).venue,
            MatchVenue::Online
        );
    }

    #[test]
    fn venue_override_is_possible() {
        let mut m = MatchResult::new("A", "B", TourneyTier::T2);
        m.venue = MatchVenue::LanCafe; // 网吧赛显式覆盖（等价 Kotlin 命名参数）
        assert_eq!(m.venue, MatchVenue::LanCafe);
    }

    #[test]
    fn venue_lan_mapping() {
        assert!(venue_lan(MatchVenue::Lan));
        assert!(venue_lan(MatchVenue::LanCafe));
        assert!(!venue_lan(MatchVenue::Online));
    }

    #[test]
    fn serde_roundtrip() {
        let m = MatchResult::new("TeamA", "TeamB", TourneyTier::Major);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""winner_sig":"TeamA""#));
        assert!(json.contains(r#""tier":"MAJOR""#));
        assert!(json.contains(r#""venue":"LAN""#));
        let back: MatchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }
}
