//! 击杀/死亡按权重轮盘分配（精确守恒）——**唯一事实源**。
//!
//! 历史上 `LiveMatchEngine`（逐回合 LIVE）与 `MatchSimulator`（快进模拟）各自
//! 内嵌一份逐字符相同的 `allocate_kills` 拷贝（Jaccard=1.00）——一旦未来单边修改，
//! LIVE 与快进模式的同一场比赛会出现击杀分配规则漂移（同种子不同结果）。
//! 提取本模块后，两个引擎共享同一实现。

use csc_util::pick_index;
use csc_util::rng::Xoshiro256StarStar;

/// 把 `total` 个击杀按权重轮盘分配给各选手（精确守恒）。
///
/// - 权重和为 0 / 总数 ≤ 0 / 权重空 → 全 0 数组（防御）；
/// - 每个击杀独立按 `pick_index` 轮盘（与 Kotlin 位级一致）；
/// - RNG 消费序 = 击杀总数（确定性铁律）。
pub fn allocate_kills(total: i32, weights: &[f64], rng: &mut Xoshiro256StarStar) -> Vec<i32> {
    let mut counts = vec![0; weights.len()];
    if total <= 0 || weights.is_empty() {
        return counts;
    }
    let weight_sum: f64 = weights.iter().sum();
    for _ in 0..total {
        counts[pick_index(weights, weight_sum, rng)] += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conserves_total() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let weights = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let counts = allocate_kills(20, &weights, &mut rng);
        assert_eq!(counts.iter().sum::<i32>(), 20);
        assert_eq!(counts.len(), 5);
    }

    #[test]
    fn empty_or_zero_returns_zeros() {
        let mut rng = Xoshiro256StarStar::seed(1);
        assert_eq!(allocate_kills(0, &[1.0, 2.0], &mut rng), vec![0, 0]);
        assert_eq!(allocate_kills(5, &[], &mut rng), Vec::<i32>::new());
        assert_eq!(allocate_kills(-1, &[1.0], &mut rng), vec![0]);
    }

    #[test]
    fn deterministic_per_seed() {
        let a = allocate_kills(30, &[1.0, 1.0, 1.0], &mut Xoshiro256StarStar::seed(7));
        let b = allocate_kills(30, &[1.0, 1.0, 1.0], &mut Xoshiro256StarStar::seed(7));
        assert_eq!(a, b, "同种子同结果（可复现）");
    }
}
