//! 赛事直邀拒绝模型（Kotlin `InviteModel.kt` 转写）——中心分布函数。

use csc_util::math_utils::sigmoid;

/// 赛事直邀拒绝模型：VRS 积分越高的队伍越可能拒绝直邀（档期冲突/轮换休息），
/// 但拒绝概率上限为 10%（至少九成接受）。
pub struct InviteModel;

impl InviteModel {
    /// 拒绝直邀的概率上限（10% → 至少九成接受）
    pub const MAX_DECLINE_RATE: f64 = 0.10;
    /// Sigmoid 陡峭度（单位：积分）
    pub const SIGMOID_STEEPNESS: f64 = 150.0;

    /// 计算队伍拒绝直邀的概率（0..MAX_DECLINE_RATE）。
    ///
    /// @param points 队伍当前 VRS 积分
    /// @param invite_center 邀请池队伍的积分均值（中心 μ）
    pub fn decline_rate(points: i32, invite_center: f64) -> f64 {
        let z = (points as f64 - invite_center) / Self::SIGMOID_STEEPNESS;
        Self::MAX_DECLINE_RATE * sigmoid(z)
    }

    /// 依据中心分布判定队伍是否拒绝本次直邀（**整数化**，D2）。
    ///
    /// @param roll_bp `[0, 10000)` 整数骰（= 引擎 rng.next_bp()，掷骰外提使本函数
    ///        保持纯函数、可脱离 RNG 单独测试）
    /// @return true = 拒绝直邀
    pub fn declines(points: i32, invite_center: f64, roll_bp: u64) -> bool {
        let threshold = (Self::decline_rate(points, invite_center) * 10_000.0).round() as u64;
        roll_bp < threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decline_rate_bounded_and_monotonic() {
        // 中心点（z=0）：0.10 × sigmoid(0) = 0.05
        assert!((InviteModel::decline_rate(1000, 1000.0) - 0.05).abs() < 1e-12);
        // 高积分 → 拒绝率上升但封顶 10%
        let high = InviteModel::decline_rate(3000, 1000.0);
        assert!(high > 0.05 && high <= 0.10);
        // 低积分 → 拒绝率趋近 0
        let low = InviteModel::decline_rate(0, 1000.0);
        assert!(
            low < 0.001,
            "低积分拒绝率趋近 0（Kotlin 同值 ≈0.00013）: {low}"
        );
    }

    #[test]
    fn declines_follows_probability() {
        // roll_bp < rate×10000 → 拒绝（3000 分、中心 1000 → 拒绝率约 0.1 → 阈值约 1000 bp）
        assert!(InviteModel::declines(3000, 1000.0, 1));
        assert!(!InviteModel::declines(3000, 1000.0, 9_999));
        // 低积分：阈值 ≈ 1 bp（0.1×sigmoid(-6.67)≈0.00013）→ roll=1 不拒绝、roll=0 拒绝
        assert!(!InviteModel::declines(0, 1000.0, 1));
        assert!(InviteModel::declines(0, 1000.0, 0));
    }
}
