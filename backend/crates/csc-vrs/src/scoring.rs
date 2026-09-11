//! VRS 积分结算 —— 官方 Glicko 退化成的 ELO（第二层：动态演变；Kotlin `VrsScoring.kt` 转写）。

use csc_domain::tourney_tier::TourneyTier;
use csc_time::window::TimeWindow;

use crate::database::VrsTeamState;

/// VRS 积分结算（官方 model/glicko.js：`setFixedRD(75)` 把 RD 固定为 75，退化为纯 ELO）。
///
/// 与旧版固定 K 表的差异：
/// - 积分变化随预期胜率自适应：爆冷（ev 小）时 Δ 大、碾压（ev 大）时 Δ 小
/// - `info` 为信息权重（= 时间衰减），越久远的比赛影响越小
/// - 赛事重要性不体现在 ELO 步长，而体现在 Seed 的奖池曲线（见 `modifiers`）
pub struct VrsScoring;

impl VrsScoring {
    /// 官方 ELO 常数：`Q = ln(10) / 400`（官方 glicko.js）。
    ///
    /// 转写说明：Kotlin 为 `val`（启动时计算一次）；Rust 用函数（每次计算，
    /// `ln(10)/400` 为廉价运算，且不引入 lazy-static 依赖）。
    pub fn q() -> f64 {
        10f64.ln() / 400.0
    }

    /// 固定 RD = 75（官方 ranking.js `setFixedRD(75)`）。
    pub const FIXED_RD: f64 = 75.0;

    /// 对手 RD 缩放因子 g（固定 RD 时为常数；官方 `addPendingMatch`）。
    pub fn g() -> f64 {
        1.0 / (1.0
            + 3.0 * Self::q() * Self::q() * Self::FIXED_RD * Self::FIXED_RD
                / (std::f64::consts::PI * std::f64::consts::PI))
            .sqrt()
    }

    /// 胜者预期胜率（官方 ev 公式；= Kotlin `expected`/`expectedWinRate`）。
    pub fn expected(winner_p: f64, loser_p: f64) -> f64 {
        1.0 / (1.0 + 10f64.powf(Self::g() * (loser_p - winner_p) / 400.0))
    }

    /// 胜者预期胜率（Int 版便捷入口）。
    pub fn expected_win_rate(winner_points: i32, loser_points: i32) -> f64 {
        Self::expected(winner_points as f64, loser_points as f64)
    }

    /// 败者预期胜率 = 1 − 胜者预期（官方两队的 ev 互补）。
    pub fn expected_loser(winner_p: f64, loser_p: f64) -> f64 {
        1.0 - Self::expected(winner_p, loser_p)
    }

    /// 胜者积分变化 Δ（score=1：mAdjRank = g·(1−ev)·info；爆冷时 Δ 大）。
    pub fn win_delta(winner_p: f64, loser_p: f64, info: f64) -> f64 {
        let ev = Self::expected(winner_p, loser_p);
        let adjusted_rd_sq = 1.0
            / (1.0 / (Self::FIXED_RD * Self::FIXED_RD)
                + Self::q() * Self::q() * Self::g() * Self::g() * ev * (1.0 - ev) * info * info);
        Self::q() * adjusted_rd_sq * Self::g() * (1.0 - ev) * info
    }

    /// 败者积分变化 Δ（score=0：mAdjRank = −g·ev·info，恒为负；被爆冷时扣分多）。
    pub fn lose_delta(winner_p: f64, loser_p: f64, info: f64) -> f64 {
        let ev_l = Self::expected_loser(winner_p, loser_p);
        let adjusted_rd_sq = 1.0
            / (1.0 / (Self::FIXED_RD * Self::FIXED_RD)
                + Self::q()
                    * Self::q()
                    * Self::g()
                    * Self::g()
                    * ev_l
                    * (1.0 - ev_l)
                    * info
                    * info);
        -Self::q() * adjusted_rd_sq * Self::g() * ev_l * info
    }

    /// 官方单场结算：按信息权重 `info` 更新双方积分（就地修改；= Kotlin `applyMatch(info)` 版）。
    ///
    /// 四舍五入用 **Kotlin `roundToInt` 语义**（floor(x+0.5)，负半值向 +∞）——loseDelta
    /// 为负，Rust 的 `round()`（half away from zero）会差 1 分。
    pub fn apply_match_with_info(winner: &mut VrsTeamState, loser: &mut VrsTeamState, info: f64) {
        let wp = winner.points as f64;
        let lp = loser.points as f64;
        winner.points += csc_util::math_utils::round_to_int(Self::win_delta(wp, lp, info));
        loser.points += csc_util::math_utils::round_to_int(Self::lose_delta(wp, lp, info));
    }

    /// 便捷入口：由赛事等级 + 可选时间信息结算一场比赛（= Kotlin `applyMatch(tier, ...)` 版）。
    ///
    /// `tier` 仅用于历史记录的奖池/LAN 推导；传了 `timestamp` 与 `window` 才做时间衰减，
    /// 否则按最新信息（info=1）处理。
    pub fn apply_match(
        winner: &mut VrsTeamState,
        loser: &mut VrsTeamState,
        _tier: TourneyTier,
        timestamp: Option<i64>,
        window: Option<&TimeWindow>,
    ) {
        let info = match (timestamp, window) {
            (Some(ts), Some(w)) => w.window_mod(ts), // 官方 calculateMatchInformationContent
            _ => 1.0,                                // 无时间数据 → 按最新比赛处理
        };
        Self::apply_match_with_info(winner, loser, info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(points: i32) -> VrsTeamState {
        VrsTeamState {
            team_name: "T".into(),
            points,
            ranking: 1,
            roster: Vec::new(),
            last_settled_roster: Vec::new(),
            seed_points: 0,
        }
    }

    #[test]
    fn expected_win_rate_symmetry() {
        let p = VrsScoring::expected_win_rate(2000, 1000);
        assert!(p > 0.9, "高分对低分预期胜率应高: {p}");
        let q = VrsScoring::expected_win_rate(1000, 2000);
        assert!((p + q - 1.0).abs() < 1e-9, "互补: {p} + {q}");
        assert!((VrsScoring::expected_win_rate(1500, 1500) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn upset_gains_more_points() {
        // 爆冷（低分胜高分）Δ 应大于碾压（高分胜低分）
        let upset = VrsScoring::win_delta(1000.0, 2000.0, 1.0);
        let stomp = VrsScoring::win_delta(2000.0, 1000.0, 1.0);
        assert!(upset > stomp, "爆冷应得更多分: upset={upset} stomp={stomp}");
        // 败者恒负
        assert!(VrsScoring::lose_delta(1000.0, 2000.0, 1.0) < 0.0);
        assert!(VrsScoring::lose_delta(2000.0, 1000.0, 1.0) < 0.0);
    }

    #[test]
    fn apply_match_with_info_zero_is_noop() {
        let mut w = state(1500);
        let mut l = state(1500);
        VrsScoring::apply_match_with_info(&mut w, &mut l, 0.0);
        assert_eq!(w.points, 1500);
        assert_eq!(l.points, 1500);
    }

    #[test]
    fn apply_match_updates_both_sides() {
        let mut w = state(1500);
        let mut l = state(1500);
        VrsScoring::apply_match_with_info(&mut w, &mut l, 1.0);
        assert!(w.points > 1500);
        assert!(l.points < 1500);
        // 守恒近似：win_delta ≈ -lose_delta（round 后可能差 1）
        assert!(((w.points - 1500) + (l.points - 1500)).abs() <= 1);
    }

    /// 跨语言 golden：Kotlin `VrsScoring` 权威输出（位模式；tools 侧 gen 脚本实证）。
    #[test]
    fn golden_scoring_formulas() {
        assert_eq!(VrsScoring::q().to_bits(), 0x3f779416b2d3a01c);
        assert_eq!(VrsScoring::g().to_bits(), 0x3fef21597cad5365);
        assert_eq!(
            VrsScoring::expected(2000.0, 1000.0).to_bits(),
            0x3fefe1d1a618fedd
        );
        assert_eq!(
            VrsScoring::expected(1500.0, 1500.0).to_bits(),
            0x3fe0000000000000
        ); // 0.5
        assert_eq!(
            VrsScoring::win_delta(1500.0, 1500.0, 1.0).to_bits(),
            0x402e2b68707547bf
        );
        assert_eq!(
            VrsScoring::lose_delta(1500.0, 1500.0, 1.0).to_bits(),
            0xc02e2b68707547bf
        );
        assert_eq!(
            VrsScoring::win_delta(1000.0, 2000.0, 1.0).to_bits(),
            0x403f5d1a525f4460
        );
        assert_eq!(
            VrsScoring::lose_delta(2000.0, 1000.0, 1.0).to_bits(),
            0xbfbdb0b74db44ca3
        );
    }
}
