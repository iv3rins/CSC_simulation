//! 随机数采样工具（Kotlin `RandomUtils` 的转写：gaussian/pickIndex/bernoulli）。
//!
//! 注：`bernoulli` 曾是无消费方的薄封装（生产路径全部直接调 `rng.roll_bp`），
//! 已于结构拆分时删除。

use crate::rng::Xoshiro256StarStar;

/// 标准正态采样（Box-Muller 变换：两个均匀分布 → 一个标准正态 N(0,1)）。
///
/// = Kotlin `RandomUtils.gaussian`：`u1 = nextDouble().coerceAtLeast(1e-12)`（防 ln(0)）、
/// `u2 = nextDouble()`、`sqrt(-2·ln(u1)) · cos(2π·u2)`。
///
/// 可复现性口径（D2）：`ln/sqrt/cos` 为 libm 超越函数，跨语言可能差最后 1-2 ulp——
/// 语言内位级一致、跨语言统计一致（golden 测试按位模式锁语言内确定性）。
pub fn gaussian(rng: &mut Xoshiro256StarStar) -> f64 {
    let u1 = rng.next_double().max(1e-12); // (0,1] 防 ln(0)
    let u2 = rng.next_double(); // [0,1)
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// 加权轮盘采样：按权重比例返回命中的下标（= Kotlin `RandomUtils.pickIndex`）。
///
/// 在 `[0, weight_sum)` 内取点，落入哪个选项的权重区间即命中。
/// 权重列表须非空且 `weight_sum` 为其和（调用方预计算）。
pub fn pick_index(weights: &[f64], weight_sum: f64, rng: &mut Xoshiro256StarStar) -> usize {
    debug_assert!(!weights.is_empty());
    let r = rng.next_double() * weight_sum; // [0, weightSum) 内取点
    let mut idx = 0;
    let mut acc = weights[0];
    while r > acc {
        idx += 1;
        acc += weights[idx];
    }
    idx
}

/// 按概率 `p` 掷一次伯努利骰（= Kotlin `RandomUtils.bernoulli`）。
///
/// 整数化（D2）：委托 [`Xoshiro256StarStar::roll_bp`]——基点粒度（1/10000）的
/// 整数比较，替代 `next_double() < p` 的浮点分支（跨语言 1 ulp 差异不再导致
/// 事件序列分叉）。生产调用方直接使用 `rng.roll_bp(p)` 即可。
#[cfg(test)]
pub(crate) fn bernoulli(p: f64, rng: &mut Xoshiro256StarStar) -> bool {
    rng.roll_bp(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_shape() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let vals: Vec<f64> = (0..10_000).map(|_| gaussian(&mut rng)).collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64;
        // 均值 ≈ 0（±0.05）、方差 ≈ 1（±0.1）
        assert!(mean.abs() < 0.05, "均值应接近 0: {mean}");
        assert!((var - 1.0).abs() < 0.1, "方差应接近 1: {var}");
    }

    #[test]
    fn pick_index_respects_weights() {
        let mut rng = Xoshiro256StarStar::seed(7);
        let weights = [1.0, 3.0, 6.0];
        let sum: f64 = weights.iter().sum();
        let mut counts = [0usize; 3];
        for _ in 0..10_000 {
            counts[pick_index(&weights, sum, &mut rng)] += 1;
        }
        // 比例约 1:3:6
        assert!(counts[0] < counts[1] && counts[1] < counts[2]);
        assert!((counts[2] as f64 / counts[0] as f64 - 6.0).abs() < 1.0);
    }

    #[test]
    fn bernoulli_probability() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let n = 10_000;
        let hits = (0..n).filter(|_| bernoulli(0.25, &mut rng)).count();
        let p = hits as f64 / n as f64;
        assert!((p - 0.25).abs() < 0.03, "概率应接近 0.25: {p}");
    }
}
