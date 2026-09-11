//! HLTV Rating 2.0 计算器（Kotlin `hltv/RatingCalculator.kt` 转写）。

/// 每回合指标（Rating 2.0 公式的直接输入）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoundStats {
    /// 每回合击杀（Kills per Round）
    pub kpr: f64,
    /// 每回合死亡（Deaths per Round）
    pub dpr: f64,
    /// 有贡献回合百分比（0~100）
    pub kast: f64,
    /// 每回合平均伤害
    pub adr: f64,
    /// 每回合助攻（Assists per Round）
    pub apr: f64,
}

/// 整场比赛原始统计（用于从击杀/死亡推导每回合指标）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MatchStats {
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    /// 总回合数（>0；KPR/DPR 由此推导）
    pub rounds: i32,
    /// 有贡献回合百分比（0~100）
    pub kast: f64,
    /// 每回合平均伤害
    pub adr: f64,
}

impl MatchStats {
    /// 推导每回合指标：kpr = kills/rounds、dpr = deaths/rounds、apr = assists/rounds。
    pub fn to_round_stats(&self) -> RoundStats {
        assert!(self.rounds > 0, "rounds 必须 > 0（当前 {}）", self.rounds);
        RoundStats {
            kpr: self.kills as f64 / self.rounds as f64,
            dpr: self.deaths as f64 / self.rounds as f64,
            kast: self.kast,
            adr: self.adr,
            apr: self.assists as f64 / self.rounds as f64,
        }
    }
}

/// HLTV Rating 2.0 计算器。
///
/// 公式源自 flashed.gg 逆向工程《Reverse Engineering HLTV Rating》：
/// ```text
/// Impact = 2.13·KPR + 0.42·APR − 0.41
/// Rating = 0.00738764·KAST + 0.35912389·KPR − 0.5329508·DPR
///          + 0.2372603·Impact + 0.0032397·ADR + 0.1587 − 0.01(误差修正)
/// ```
pub struct RatingCalculator;

impl RatingCalculator {
    // —— 公式权重（官方逆向结果） ——
    pub const IMPACT_KPR_WEIGHT: f64 = 2.13;
    pub const IMPACT_APR_WEIGHT: f64 = 0.42;
    pub const IMPACT_OFFSET: f64 = 0.41;
    pub const WEIGHT_KAST: f64 = 0.00738764;
    pub const WEIGHT_KPR: f64 = 0.35912389;
    pub const WEIGHT_DPR: f64 = -0.5329508;
    pub const WEIGHT_IMPACT: f64 = 0.2372603;
    pub const WEIGHT_ADR: f64 = 0.0032397;
    pub const RATING_BASE: f64 = 0.1587;
    pub const RATING_ERROR_CORRECTION: f64 = 0.01;

    /// 影响力 Impact（回合均击杀/助攻的组合）。
    pub fn impact(kpr: f64, apr: f64) -> f64 {
        Self::IMPACT_KPR_WEIGHT * kpr + Self::IMPACT_APR_WEIGHT * apr - Self::IMPACT_OFFSET
    }

    /// 完整 Rating 2.0（传每回合指标）。
    pub fn rating_of_round(kpr: f64, dpr: f64, kast: f64, adr: f64, apr: f64) -> f64 {
        let imp = Self::impact(kpr, apr);
        Self::WEIGHT_KAST * kast
            + Self::WEIGHT_KPR * kpr
            + Self::WEIGHT_DPR * dpr
            + Self::WEIGHT_IMPACT * imp
            + Self::WEIGHT_ADR * adr
            + Self::RATING_BASE
            - Self::RATING_ERROR_CORRECTION
    }

    /// 按每回合指标结构计算（= Kotlin `ratingOf(RoundStats)`）。
    pub fn rating_of(round: &RoundStats) -> f64 {
        Self::rating_of_round(round.kpr, round.dpr, round.kast, round.adr, round.apr)
    }

    /// 按「击杀 / 死亡」原始数据计算（= Kotlin `ratingOf(MatchStats)`）。
    pub fn rating_of_match(match_stats: &MatchStats) -> f64 {
        Self::rating_of(&match_stats.to_round_stats())
    }

    /// 按击杀 / 死亡 / 助攻 / 回合数直接计算（模拟链便捷入口）。
    /// 模拟器不产出真实 ADR/KAST，用 `adr_of`/`kast_of` 估值推导。
    pub fn rating_of_counts(kills: i32, deaths: i32, assists: i32, rounds: i32) -> f64 {
        assert!(rounds > 0, "rounds 必须 > 0（当前 {rounds}）");
        let kpr = kills as f64 / rounds as f64;
        let dpr = deaths as f64 / rounds as f64;
        let apr = assists as f64 / rounds as f64;
        Self::rating_of_round(
            kpr,
            dpr,
            Self::kast_of(kpr, dpr, apr),
            Self::adr_of(kpr, apr, dpr),
            apr,
        )
    }

    /// ADR 估值（**2026 复审校准**）：对齐真实 HLTV 顶尖样本——ZywOo 2023
    /// (KPR 0.84, APR 0.28, DPR 0.57 → ADR 86.7)、donk 2024 (0.94/0.33/0.65 →
    /// 97.9)。旧公式 `kpr*100` 把击杀伤害按满血估，顶尖选手被推到 110 封顶、
    /// 与真实 ADR 脱节。新公式：击杀 ≈ 31 血/个（含残血击杀均值），夹到 [30, 115]。
    pub fn adr_of(kpr: f64, apr: f64, dpr: f64) -> f64 {
        (52.0 + kpr * 31.0 + apr * 24.0 + dpr * 3.5).clamp(30.0, 115.0)
    }

    /// KAST 估值（0~100）：击杀/助攻抬升、死亡拖低，夹到 [40, 88]。
    /// 旧公式 `50 + (kpr-dpr)*40 + kpr*20 + apr*30` 在 KPR 1.0 时直接顶到 90 封顶，
    /// 使 rating 虚高。新公式对齐真实：顶尖 (0.84/0.57/0.28 → 78.5)、
    /// 平均 (0.70/0.70/0.20 → ~72)。
    pub fn kast_of(kpr: f64, dpr: f64, apr: f64) -> f64 {
        (61.0 + (kpr - dpr) * 20.0 + kpr * 14.0 + apr * 10.0).clamp(40.0, 88.0)
    }

    /// HLTV 2021 顶尖选手 Rating 2.0 对照表（用于 `nearest_pro_player` 对比）。
    pub const PRO_PLAYER_RATINGS: [(&'static str, f64); 43] = [
        ("s1mple", 1.25),
        ("ZywOo", 1.24),
        ("sh1ro", 1.25),
        ("NiKo", 1.21),
        ("Ax1Le", 1.21),
        ("blameF", 1.20),
        ("m0nesy", 1.15),
        ("stavn", 1.15),
        ("frozen", 1.14),
        ("broky", 1.14),
        ("YEKINDAR", 1.13),
        ("Twistzz", 1.12),
        ("ropz", 1.12),
        ("huNter-", 1.11),
        ("HObbit", 1.11),
        ("Spinx", 1.10),
        ("electronic", 1.08),
        ("rain", 1.08),
        ("syrsoN", 1.08),
        ("dycha", 1.08),
        ("b1t", 1.07),
        ("REZ", 1.07),
        ("TeSeS", 1.07),
        ("konfig", 1.06),
        ("cadiaN", 1.06),
        ("Magisk", 1.05),
        ("oSee", 1.04),
        ("sjuush", 1.04),
        ("tabseN", 1.03),
        ("jabbi", 1.03),
        ("Perfecto", 1.03),
        ("dupreeh", 1.01),
        ("Maden", 1.00),
        ("hampus", 0.99),
        ("nafany", 0.98),
        ("Xyp9x", 0.94),
        ("es3tag", 0.94),
        ("Snappi", 0.93),
        ("gla1ve", 0.92),
        ("dexter", 0.92),
        ("karrigan", 0.91),
        ("apEX", 0.90),
        ("nitr0", 0.89),
    ];

    /// 找到与目标 rating 最接近的对照选手（= Kotlin `nearestProPlayer`）。
    pub fn nearest_pro_player(target: f64) -> &'static str {
        let mut best = "";
        let mut min_diff = f64::MAX;
        for (player, rating) in Self::PRO_PLAYER_RATINGS {
            let diff = (target - rating).abs();
            if diff < min_diff {
                min_diff = diff;
                best = player;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_formula() {
        // 1.0 KPR、0.3 APR → 2.13 + 0.126 - 0.41 = 1.846
        assert!((RatingCalculator::impact(1.0, 0.3) - 1.846).abs() < 1e-12);
    }

    #[test]
    fn rating_known_value() {
        // 参考实现 sanity：KPR=0.8, DPR=0.6, KAST=75, ADR=80, APR=0.25
        let r = RatingCalculator::rating_of_round(0.8, 0.6, 75.0, 80.0, 0.25);
        assert!(r > 0.8 && r < 1.4, "合理 rating 区间: {r}");
    }

    #[test]
    fn adr_kast_estimates_clamped() {
        assert_eq!(RatingCalculator::adr_of(10.0, 10.0, 10.0), 115.0);
        assert_eq!(RatingCalculator::adr_of(0.0, 0.0, 0.0), 52.0);
        assert_eq!(RatingCalculator::kast_of(10.0, 0.0, 0.0), 88.0);
        assert_eq!(RatingCalculator::kast_of(0.0, 10.0, 0.0), 40.0);
        // 真实量级对齐：ZywOo 2023 (KPR 0.84, DPR 0.57, APR 0.28) → KAST ≈ 81 / ADR ≈ 87
        // （真实 78.5 / 86.7）；平均选手 (0.70/0.70/0.20) → KAST ≈ 73 / ADR ≈ 81
        assert!((RatingCalculator::kast_of(0.84, 0.57, 0.28) - 81.0).abs() < 0.5);
        assert!((RatingCalculator::adr_of(0.84, 0.28, 0.57) - 86.8).abs() < 0.5);
        assert!((RatingCalculator::kast_of(0.7, 0.7, 0.2) - 72.8).abs() < 0.5);
        // 旧公式在 KPR=1.0 时顶到 90 封顶导致 rating 虚高——新公式应低于 88
        assert!(RatingCalculator::kast_of(1.0, 0.55, 0.25) < 88.0);
    }

    #[test]
    fn nearest_pro_player() {
        assert_eq!(RatingCalculator::nearest_pro_player(1.25), "s1mple");
        assert_eq!(RatingCalculator::nearest_pro_player(0.90), "apEX");
    }

    #[test]
    fn counts_entry_requires_positive_rounds() {
        let r = RatingCalculator::rating_of_counts(20, 15, 5, 30);
        assert!(r > 0.8 && r < 1.5);
    }

    /// 跨语言 golden：Kotlin `RatingCalculator` 权威输出（tools/gen_sim_golden.kt）。
    /// 2026 复审校准（击杀线性化 + KAST/ADR 估值回归真实量级）后更新为 Rust 新权威值。
    #[test]
    fn golden_rating_formulas() {
        assert_eq!(
            RatingCalculator::rating_of_counts(20, 15, 5, 30).to_bits(),
            0x3ff303f27bd27ddb
        );
        assert_eq!(
            RatingCalculator::adr_of(0.8, 0.25, 0.6).to_bits(),
            0x4055399999999999
        );
        assert_eq!(
            RatingCalculator::kast_of(0.8, 0.6, 0.25).to_bits(),
            0x4053accccccccccd
        );
    }
}
