//! xoshiro256** 确定性随机生成器 —— Kotlin `DeterministicRandom` 的位级直译。
//!
//! 逐位对照表（Kotlin 方法 → Rust 方法，输出位模式完全一致）：
//!
//! | Kotlin | Rust | 说明 |
//! |---|---|---|
//! | `nextLong()` | [`next_u64`] | 核心原语（Kotlin Long 有符号，位模式与 u64 相同） |
//! | `nextBits(n)` | [`next_bits`] | 取高 n 位（`>> (64-n)`） |
//! | `nextInt()` | [`next_i32`] | 低 32 位截断 |
//! | `nextInt(until)` | [`next_i32_bound`] | 2 的幂分支 + 拒绝采样（**逐位复刻**） |
//! | `nextDouble()` | [`next_double`] | 53 位尾数 `[0,1)` |
//! | `nextFloat()` | [`next_float`] | 24 位尾数 `[0,1)` |
//! | `nextBoolean()` | [`next_bool`] | `nextBits(1) != 0` |
//! | `seed(seed)` | [`seed`] | splitmix64 展开 4 字 |
//! | `snapshot()/restore()` | [`snapshot`]/[`from_state`] | 4 × u64 状态 |
//!
//! 关键陷阱（已按 Kotlin 语义处理）：
//! - Kotlin `Long`/`Int` 溢出是补码 wrap → Rust 全部 `wrapping_*`；
//! - `nextInt(until)` 的拒绝条件 `v - v % until + (until - 1) < 0` 中
//!   `v % until` 是 **trunc 余数**（Rust `%` 同语义）、加法可能溢出 wrap；
//! - `rotl` 在 k=0 时 Kotlin 的 `ushr 64` 取模为 `ushr 0` → Rust 需特判。

use serde::{Deserialize, Serialize};

/// xoshiro256** 确定性随机生成器。
///
/// 状态 4 × u64（构造后永不为全 0——splitmix64 种子展开保证）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Xoshiro256StarStar {
    s: [u64; 4],
}

impl Xoshiro256StarStar {
    /// splitmix64 常量（Kotlin `DeterministicRandom` companion 同值）。
    const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
    const SPLITMIX_M1: u64 = 0xBF58_476D_1CE4_E5B9;
    const SPLITMIX_M2: u64 = 0x94D0_49BB_1331_11EB;

    /// 由种子派生初始状态（splitmix64 展开 4 个字，与 Kotlin `seed` 一致）。
    pub fn seed(seed: u64) -> Self {
        let mut state = [0u64; 4];
        let mut z = seed;
        for s in &mut state {
            z = z.wrapping_add(Self::SPLITMIX_GAMMA);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(Self::SPLITMIX_M1);
            x = (x ^ (x >> 27)).wrapping_mul(Self::SPLITMIX_M2);
            *s = x ^ (x >> 31);
        }
        Self { s: state }
    }

    /// 从快照恢复（读档用；状态必须为 4 个字）。
    pub fn from_state(state: [u64; 4]) -> Self {
        Self { s: state }
    }

    /// 快照当前状态（存档用）。
    pub fn snapshot(&self) -> [u64; 4] {
        self.s
    }

    /// 循环左移（= Kotlin `rotl`；`u64::rotate_left` 对 k=0 返回原值、k 取模 64，
    /// 与 Kotlin `ushr (64-k)` 的移位取模语义一致——golden 测试锁死位级一致）。
    #[inline]
    fn rotl(x: u64, k: u32) -> u64 {
        x.rotate_left(k)
    }

    /// 下一个 64 位随机数（= Kotlin `nextLong()` 的位模式）。
    pub fn next_u64(&mut self) -> u64 {
        let result = Self::rotl(self.s[1].wrapping_mul(5), 7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = Self::rotl(self.s[3], 45);
        result
    }

    /// 取高 `bit_count` 位（= Kotlin `nextBits(bitCount)`；`bit_count` ∈ 1..=32）。
    pub fn next_bits(&mut self, bit_count: u32) -> i32 {
        debug_assert!((1..=32).contains(&bit_count));
        (self.next_u64() >> (64 - bit_count)) as i32
    }

    /// 32 位随机整数（= Kotlin `nextInt()`：`nextLong().toInt()` 低 32 位）。
    pub fn next_i32(&mut self) -> i32 {
        self.next_u64() as i32
    }

    /// `[0, until)` 均匀整数（= Kotlin **单参数** `nextInt(until)`，DeterministicRandom 重写版）。
    ///
    /// - `until` 为 2 的幂：`nextLong() and (until-1)`（低 until-1 位）；
    /// - 否则拒绝采样：`v = (nextLong() ushr 1).toInt()`（高 63 位的低 32 位），
    ///   拒绝条件 `v - v % until + (until - 1) < 0` 用 **wrapping 运算**（Kotlin Int 溢出 wrap）。
    ///
    /// **上游 bug 修正（2026-08-13，20 年浸泡测试捕获）**：Kotlin 重写版用 i32 承载
    /// `v`——负 `v` 时 `v % until` 仍为负，且 `v ∈ (-until, 0)` 的负小值会通过拒绝
    /// 判定并被原样返回（`NAME_POOL[-11]` 越界）。Rust 侧改为 **u32 同构运算**：
    /// 接受/拒绝判定与 Kotlin 重写版**位级一致**（cond 的 32 位模式相同），
    /// 仅返回值恒非负——golden 序列不受影响（缺陷命中率 ~1e-9，golden 样本无负 v）。
    ///
    /// ⚠️ 与双参数 [`Self::next_i32_in`] 的底层原语不同（见其文档）——调用点必须
    /// 与 Kotlin 调用点一一对应，混用会破坏跨语言可复现性。
    pub fn next_i32_bound(&mut self, until: i32) -> i32 {
        assert!(until > 0, "bound 必须为正（实际 {until}）");
        if until & (until - 1) == 0 {
            return (self.next_u64() & (until - 1) as u64) as i32;
        }
        let u = until as u32;
        loop {
            // v = nextLong 的 bits 1..32（= Kotlin `(nextLong() ushr 1).toInt()` 位模式）
            let v = (self.next_u64() >> 1) as u32;
            // Kotlin: v - v % until + (until - 1) < 0 —— 补码 wrap 语义下
            // "i32 为负" ⟺ "u32 ≥ 2^31"；拒绝判定两域位级一致
            let cond = v.wrapping_sub(v % u).wrapping_add(u - 1);
            if cond < 0x8000_0000 {
                return (v % u) as i32;
            }
        }
    }

    /// `[from, until)` 均匀整数（= Kotlin **双参数** `nextInt(from, until)`，**stdlib 默认实现**，
    /// 非 DeterministicRandom 重写版——两者底层原语不同，必须区分）。
    ///
    /// Kotlin 2.3 stdlib 实现（`kotlin/random/Random.kt`）：
    /// - `n = until - from` 为 2 的幂：`nextBits(fastLog2(n))`（**取高 bitCount 位**）；
    /// - 否则拒绝采样：`bits = nextInt().ushr(1)`（**32 位原语**，取 nextInt 低 32 位的
    ///   无符号右移——与单参数版的 `(nextLong() ushr 1).toInt()` 不同，
    ///   在 nextLong 高 32 位为奇数时两者取值不同，拒绝行为分叉）。
    ///
    /// 仅支持 `n > 0`（项目内所有调用点 from/until 均为小正数；
    /// Kotlin 的 `n == Int.MIN_VALUE` 溢出分支不转写）。
    pub fn next_i32_in(&mut self, from: i32, until: i32) -> i32 {
        assert!(until > from, "Random range is empty: [{from}, {until}).");
        let n = until.wrapping_sub(from);
        if n > 0 && n & -n == n {
            // 2 的幂：nextBits(fastLog2(n))；fastLog2(n) = 31 - countLeadingZeroBits(n)
            let bit_count = 31 - (n as u32).leading_zeros();
            return from + self.next_bits(bit_count);
        }
        // 拒绝采样：bits = nextInt().ushr(1)（u32 无符号右移）
        loop {
            let bits = (self.next_i32() as u32) >> 1;
            let v = bits % n as u32;
            // Kotlin: bits - v + (n - 1) < 0（Int wrap：数学值 ≥ 2^31 视为负）；
            // n ≤ 2^31-1 时 u64 不会溢出，直接比较。
            let cond = bits as u64 - v as u64 + (n - 1) as u64;
            if cond < (1u64 << 31) {
                return from + v as i32;
            }
        }
    }

    /// 53 位双精度 `[0, 1)`（= Kotlin `nextDouble()`）。
    ///
    /// ⚠️ 仅用于**连续值抽样**（Box-Muller、加权取点、属性生成等无分支判定场景）。
    /// **布尔概率判定**一律走 [`Self::roll_bp`]（整数化，见其文档）——浮点分支
    /// 在不同语言/平台间可能最后 1 ulp 分叉，破坏跨语言事件序列对齐。
    pub fn next_double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// 概率判定（整数化，ARCHITECTURE-MAPPING D2）：以**基点**（1/10000）为粒度的
    /// 伯努利判定——`next_u64() % 10_000 < round(p × 10_000)`。
    ///
    /// 动机：`next_double() < p` 是双浮点比较，p 在跨语言复刻时可能差最后 1 ulp
    /// （libm 差异），导致同一事件序列在两种语言间分叉。整数化后分支只依赖
    /// `p` 四舍五入到基点后的整数阈值——1 ulp 的浮点差几乎不可能改变舍入结果，
    /// 事件序列可跨语言对齐（语言内仍是位级确定）。
    ///
    /// RNG 消耗：恰好 1 个 `next_u64`（与 `next_double` 同次数，序列消耗语义不变）。
    pub fn roll_bp(&mut self, p: f64) -> bool {
        debug_assert!((0.0..=1.0).contains(&p), "概率必须 ∈ [0,1]（实际 {p}）");
        let bp = (p * 10_000.0).round() as u64;
        self.next_u64() % 10_000 < bp
    }

    /// 掷一个基点粒度 `[0, 10000)` 的整数骰（= 取模后的均匀随机），
    /// 供"掷骰值外提 + 纯函数阈值比较"的判定模式（如 [`csc_tournaments::invite_model`]）：
    /// 调用方先掷 `next_bp()`，再交给无状态的纯函数比较整数阈值——
    /// 随机消费与判定解耦，纯函数可脱离 RNG 单独测试。
    ///
    /// RNG 消耗：恰好 1 个 `next_u64`。
    pub fn next_bp(&mut self) -> u64 {
        self.next_u64() % 10_000
    }

    /// 24 位单精度 `[0, 1)`（= Kotlin `nextFloat()`）。
    pub fn next_float(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0f32 / (1u64 << 24) as f32)
    }

    /// 布尔值（= Kotlin `Random.nextBoolean()` 默认实现：`nextBits(1) != 0`）。
    pub fn next_bool(&mut self) -> bool {
        self.next_bits(1) != 0
    }
}

impl Default for Xoshiro256StarStar {
    /// 默认种子（原型默认 `DEFAULT_SEED = 42`，与 Kotlin `Engine.DEFAULT_SEED` 一致）。
    fn default() -> Self {
        Self::seed(42)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kotlin `DeterministicRandom` 生成的 golden 序列（`tools/gen_rng_golden.kt` 输出）。
    /// 一旦跨语言位级一致被破坏，此测试即失败——可复现性契约的第一道锁。
    const GOLDEN_LONG_42: [u64; 10] = [
        0x15780b2e0c2ec716,
        0x6104d9866d113a7e,
        0xae17533239e499a1,
        0xecb8ad4703b360a1,
        0xfde6dc7fe2ec5e64,
        0xc50da53101795238,
        0xb82154855a65ddb2,
        0xd99a2743ebe60087,
        0xc2e96e726e97647e,
        0x9556615f775fbc3d,
    ];
    const GOLDEN_DOUBLE_42_BITS: [u64; 5] = [
        0x3fb5780b2e0c2ec0,
        0x3fd84136619b444e,
        0x3fe5c2ea66473c93,
        0x3fed9715a8e0766c,
        0x3fefbcdb8ffc5d8b,
    ];
    const GOLDEN_FLOAT_42_BITS: [u32; 3] = [0x3dabc058, 0x3ec209b2, 0x3f2e1753];
    const GOLDEN_INT100_42: [i32; 10] = [7, 15, 24, 11, 36, 6, 73, 18, 6, 77];
    const GOLDEN_INT97_42: [i32; 10] = [11, 94, 60, 2, 74, 71, 91, 7, 21, 44];
    const GOLDEN_INT64_42: [i32; 8] = [22, 62, 33, 33, 36, 56, 50, 7];
    const GOLDEN_LONG_7: [u64; 5] = [
        0xb358faf74ef9765a,
        0x475c3d964f482cd2,
        0xd6f1d349952c7996,
        0xfb2938731e807240,
        0xfda904ec7e540318,
    ];
    const GOLDEN_LONG_0: [u64; 3] = [0x99ec5f36cb75f2b4, 0xbf6e1f784956452a, 0x1a5f849d4933e6e0];
    const GOLDEN_SNAPSHOT_AFTER: [u64; 3] =
        [0xae17533239e499a1, 0xecb8ad4703b360a1, 0xfde6dc7fe2ec5e64];
    const GOLDEN_SNAPSHOT_STATE: [u64; 4] = [
        0x9d1c68b67d1ceb43,
        0x23acb714c3a78d01,
        0x9d431c524cad8dc5,
        0x72e82a070b0b9ddb,
    ];

    fn seq(mut rng: Xoshiro256StarStar, n: usize) -> Vec<u64> {
        (0..n).map(|_| rng.next_u64()).collect()
    }

    /// 跨语言对照：Kotlin seed(42) 前 10 个 nextLong 位模式（第一道锁）。
    #[test]
    fn golden_next_long_42() {
        assert_eq!(seq(Xoshiro256StarStar::seed(42), 10), GOLDEN_LONG_42);
    }

    #[test]
    fn golden_next_double_42() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let bits: Vec<u64> = (0..5).map(|_| rng.next_double().to_bits()).collect();
        assert_eq!(bits, GOLDEN_DOUBLE_42_BITS);
    }

    #[test]
    fn golden_next_float_42() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let bits: Vec<u32> = (0..3).map(|_| rng.next_float().to_bits()).collect();
        assert_eq!(bits, GOLDEN_FLOAT_42_BITS);
    }

    #[test]
    fn golden_next_int_bound_42() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let v: Vec<i32> = (0..10).map(|_| rng.next_i32_bound(100)).collect();
        assert_eq!(v, GOLDEN_INT100_42);
        let mut rng = Xoshiro256StarStar::seed(42);
        let v: Vec<i32> = (0..10).map(|_| rng.next_i32_bound(97)).collect();
        assert_eq!(v, GOLDEN_INT97_42);
        let mut rng = Xoshiro256StarStar::seed(42);
        let v: Vec<i32> = (0..8).map(|_| rng.next_i32_bound(64)).collect();
        assert_eq!(v, GOLDEN_INT64_42);
    }

    #[test]
    fn golden_other_seeds() {
        assert_eq!(seq(Xoshiro256StarStar::seed(7), 5), GOLDEN_LONG_7);
        assert_eq!(seq(Xoshiro256StarStar::seed(0), 3), GOLDEN_LONG_0);
    }

    /// 跨语言对照：快照状态与恢复后序列（Kotlin 侧权威值）。
    #[test]
    fn golden_snapshot_restore() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let _ = rng.next_u64();
        let _ = rng.next_u64();
        let snap = rng.snapshot();
        assert_eq!(snap, GOLDEN_SNAPSHOT_STATE, "快照状态位级一致");
        let after: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
        assert_eq!(after, GOLDEN_SNAPSHOT_AFTER);
        let mut restored = Xoshiro256StarStar::from_state(snap);
        let replay: Vec<u64> = (0..3).map(|_| restored.next_u64()).collect();
        assert_eq!(replay, GOLDEN_SNAPSHOT_AFTER, "恢复后序列无缝衔接");
    }

    /// 跨语言 golden：**双参数** `nextInt(from, until)`（stdlib 默认实现，非单参数重写版）。
    /// Kotlin 权威序列（seed=42；`tools/gen_entities_golden.kt` 相关实证脚本）。
    /// 注意：与 `from + nextInt(n)`（单参数）在第 4 次起分叉——底层原语不同
    /// （双参数：`nextInt().ushr(1)`；单参数：`(nextLong() ushr 1).toInt()`）。
    #[test]
    fn golden_next_int_in() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let seq: Vec<i32> = (0..10).map(|_| rng.next_i32_in(15, 41)).collect();
        assert_eq!(seq, vec![28, 16, 23, 17, 19, 35, 34, 18, 36, 27]);

        // 2 的幂路径：nextInt(0, 64) 用 nextBits(6)（与单参数 nextInt(64) 的 nextLong 分支不同）
        let mut rng = Xoshiro256StarStar::seed(42);
        let p2: Vec<i32> = (0..8).map(|_| rng.next_i32_in(0, 64)).collect();
        assert_eq!(p2, vec![5, 24, 43, 59, 63, 49, 46, 54]);

        // 非零起点：nextInt(5, 26)
        let mut rng = Xoshiro256StarStar::seed(42);
        let p3: Vec<i32> = (0..8).map(|_| rng.next_i32_in(5, 26)).collect();
        assert_eq!(p3, vec![11, 7, 7, 20, 18, 21, 24, 24]);
    }

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Xoshiro256StarStar::seed(42);
        let mut b = Xoshiro256StarStar::seed(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn snapshot_restore_continuity() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let _ = rng.next_u64();
        let _ = rng.next_u64();
        let snap = rng.snapshot();
        let expected: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
        let mut restored = Xoshiro256StarStar::from_state(snap);
        let actual: Vec<u64> = (0..3).map(|_| restored.next_u64()).collect();
        assert_eq!(expected, actual, "恢复后序列必须无缝衔接");
        // serde 往返（存档）
        let json = serde_json::to_string(&snap).unwrap();
        let back: [u64; 4] = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Xoshiro256StarStar::seed(1);
        let mut b = Xoshiro256StarStar::seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn power_of_two_bound_matches_kotlin_branch() {
        // until=64：nextLong() and 63（2 的幂分支）
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..20 {
            let v = rng.next_i32_bound(64);
            assert!((0..64).contains(&v));
        }
    }

    #[test]
    fn bound_one_always_zero() {
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..100 {
            assert_eq!(rng.next_i32_bound(1), 0);
        }
    }

    #[test]
    fn next_double_in_range() {
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..1000 {
            let d = rng.next_double();
            assert!((0.0..1.0).contains(&d));
        }
    }

    #[test]
    fn roll_bp_boundaries_and_consumption() {
        // 确定性：p=0 必 false，p=1 必 true（不消费 RNG）
        let mut rng = Xoshiro256StarStar::seed(42);
        assert!(!rng.roll_bp(0.0));
        assert!(rng.roll_bp(1.0));
        // 消耗语义：roll_bp 恰好 1 个 next_u64（与 next_double 同）
        let mut a = Xoshiro256StarStar::seed(7);
        let mut b = Xoshiro256StarStar::seed(7);
        let _ = a.roll_bp(0.5);
        let _ = b.next_u64();
        assert_eq!(a.snapshot(), b.snapshot(), "roll_bp 消耗 1 个 next_u64");
        // 边界值不 panic
        let mut rng = Xoshiro256StarStar::seed(42);
        for p in [1e-9, 0.0001, 0.5, 0.9999, 1.0] {
            let _ = rng.roll_bp(p);
        }
    }

    #[test]
    fn roll_bp_distribution_roughly_matches_p() {
        // 粗验证：p=0.3 时命中率在 3σ 内（n=10000，σ≈0.0046）
        let mut rng = Xoshiro256StarStar::seed(7);
        let n = 10_000;
        let hits = (0..n).filter(|_| rng.roll_bp(0.3)).count();
        let ratio = hits as f64 / n as f64;
        assert!((ratio - 0.3).abs() < 0.015, "命中率偏离 p：{ratio}");
    }

    #[test]
    fn next_int_bound_rejection_sampling_distribution() {
        // 非 2 幂 bound：拒绝采样路径；粗验证分布均匀（±10% 容忍）
        let mut rng = Xoshiro256StarStar::seed(7);
        let n = 10_000;
        let bound = 97;
        let mut counts = vec![0u32; bound as usize];
        for _ in 0..n {
            counts[rng.next_i32_bound(bound) as usize] += 1;
        }
        let expected = n / bound as usize;
        for c in &counts {
            // ±30% 容忍（n=10K、97 桶时 σ≈10，2.5σ 波动属正常）
            assert!(
                (*c as i32 - expected as i32).abs() < (expected as i32 * 3 / 10).max(10),
                "分布严重不均：{counts:?}"
            );
        }
    }

    /// 白盒回归（上游 bug 修正）：构造状态使首抽 `v = 0xFFFFFFF5`（i32 = -11）。
    /// 旧实现会返回 **-11**（负随机数，20 年浸泡测试以 `NAME_POOL[-11]` 越界暴露）；
    /// 修正后负 v 被 u32 域拒绝判定淘汰，返回值恒在 `[0, until)`。
    #[test]
    fn next_int_bound_never_returns_negative_for_negative_v_state() {
        // 状态推导：s1 = x 使 first next_u64 = 0x1FFFFFFEA → v = 0xFFFFFFF5（i32: -11）
        let mut rng = Xoshiro256StarStar::from_state([0, 15_513_199_356_433_621_174, 0, 0]);
        for _ in 0..100 {
            let v = rng.next_i32_bound(40);
            assert!((0..40).contains(&v), "返回值必须在 [0, 40)：{v}");
        }
        // 大规模非负性回归（跨多种 bound 与种子）
        for seed in [0u64, 1, 7, 42, 123456] {
            let mut rng = Xoshiro256StarStar::seed(seed);
            for bound in [3, 40, 97, 200] {
                for _ in 0..50_000 {
                    let v = rng.next_i32_bound(bound);
                    assert!(
                        (0..bound).contains(&v),
                        "seed={seed} bound={bound} 返回越界值 {v}"
                    );
                }
            }
        }
    }

    #[test]
    fn default_seed_is_42() {
        assert_eq!(
            Xoshiro256StarStar::default().snapshot(),
            Xoshiro256StarStar::seed(42).snapshot()
        );
    }
}
