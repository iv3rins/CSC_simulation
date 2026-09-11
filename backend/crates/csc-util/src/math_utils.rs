//! 通用数学工具（Kotlin `MathUtils` 的转写）。

/// Sigmoid 函数：把任意实数映射到 (0,1)（= Kotlin `MathUtils.sigmoid`）。
///
/// 收敛重复点：WinRateCalculator（胜率 sigmoid）与 InviteModel（直邀拒绝率）共用。
/// 可复现性口径：`exp` 为 libm 超越函数——语言内位级一致、跨语言统计一致。
#[inline]
pub fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// Kotlin `Double.roundToInt()` 语义（= Java `Math.round`）：**floor(x + 0.5)**，
/// 正负半值都向 +∞ 舍入（round(-2.5) = -2）。
///
/// ⚠️ 与 Rust `f64::round()`（half away from zero，round(-2.5) = -3）不同——
/// 涉及负值的公式（如 GrowthModel 的成长 delta）必须用本函数。
#[inline]
pub fn round_to_int(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_properties() {
        assert_eq!(sigmoid(0.0), 0.5);
        assert!(sigmoid(-100.0) > 0.0 && sigmoid(-100.0) < 1e-40);
        assert!((sigmoid(100.0) - 1.0).abs() < 1e-40);
        assert!(sigmoid(1.0) > 0.5);
        assert!(sigmoid(-1.0) < 0.5);
        // 单调
        assert!(sigmoid(1.0) > sigmoid(0.5) && sigmoid(0.5) > sigmoid(0.0));
    }
}
