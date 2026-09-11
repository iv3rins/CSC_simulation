//! 胜率计算器（Kotlin `WinRateCalculator.kt` 转写）。

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::character::PlayerCharacter;
use csc_util::rng::Xoshiro256StarStar;
use csc_util::{gaussian, sigmoid};

use crate::condition::ConditionModel;
use crate::directives::{MatchDirectives, Playstyle};
use crate::volatility::VolatilityModel;

/// 胜率计算器。
///
/// 胜率 = f(我方当场合力, 对手当场合力, 赛事等级, 心态, 场内指令, 体况, 凝聚力)。
/// 差异化设计：对每名选手按其发挥标准差 σ 采样「单场实际发挥」
/// （`P_actual = clamp(P_base + N(0,1) × σ)`），神经刀波动大、稳健指挥几乎稳定。
///
/// 随机消耗顺序（跨语言可复现）：我方每选手 1 个 gaussian（=2 个 nextDouble）
/// → 对手每选手 1 个 gaussian。无其他随机消耗。
pub struct WinRateCalculator;

impl WinRateCalculator {
    /// sigmoid 曲线的陡峭度基准
    pub const K: f64 = 0.030;
    /// 心态（抗压/自信/士气）在胜率中的权重
    pub const MENTALITY_WEIGHT: f64 = 0.20;
    /// 结构因子差 → 等效实力差的放大系数（磨合/角色互补/队内关系）。
    /// 结构因子范围约 0.85~1.25 → 因子差 ±0.4 → ±20 等效实力差（心态差同级）。
    pub const STRUCTURE_SCALE: f64 = 50.0;

    /// 不同赛事等级对实力差的放大系数（等级越高系数越大 → 强队更难被爆冷）。
    pub fn tier_factor(tier: TourneyTier) -> f64 {
        match tier {
            TourneyTier::Major => 1.35,      // Major：实力差最被放大
            TourneyTier::SuperElite => 1.20, // 卡托维兹/科隆
            TourneyTier::Elite => 1.10,      // EPL/BLAST 大型 Premier
            TourneyTier::T1 => 1.05,         // 一级国际赛
            TourneyTier::T2 => 0.90,         // 二级赛
            TourneyTier::Qualify => 0.75,    // 预选赛：最易爆冷
        }
    }

    /// 计算「我方」对阵「对手」的胜率（0..1）。
    ///
    /// @param self     我方选手列表（含玩家与队友）
    /// @param opponent 对手选手列表
    /// @param tier     赛事等级
    /// @param rng      随机源（单场发挥采样）
    /// @param directives_self  我方场内指令（None = 无指令）
    /// @param directives_opp   对手场内指令（None = 无指令）
    /// @param structure_self   我方队伍结构因子（角色互补×磨合×队内关系；1.0 中性）
    /// @param structure_opp    对手队伍结构因子
    /// 参数分四组（双方 roster / 赛事与随机 / 指令对 / 结构对）——每组对偶出现，
    /// 拆结构体反而不直观（与 Kotlin 转写对齐）；见 ENGINEERING-NOTES。
    #[allow(clippy::too_many_arguments)]
    pub fn win_rate(
        self_roster: &[&PlayerCharacter],
        opponent: &[&PlayerCharacter],
        tier: TourneyTier,
        rng: &mut Xoshiro256StarStar,
        directives_self: Option<MatchDirectives>,
        directives_opp: Option<MatchDirectives>,
        structure_self: f64,
        structure_opp: f64,
    ) -> f64 {
        let self_power = match_power(self_roster, rng) + Self::style_bonus(directives_self);
        let opp_power = match_power(opponent, rng) + Self::style_bonus(directives_opp);
        let power_diff = self_power - opp_power; // 当场合力差
        let mentality_diff = Self::mentality(self_roster) - Self::mentality(opponent); // 双方心态差

        // 综合差异：当场合力差 + 心态差 + 队伍结构差（磨合/角色互补/队内关系）。
        // 结构走**差异通道**：双方结构同构（因子相等）时该项为 0——旧行为不变，
        // 跨语言 golden 位模式不受影响。
        let structure_diff = (structure_self - structure_opp) * Self::STRUCTURE_SCALE;
        let diff = power_diff + Self::MENTALITY_WEIGHT * mentality_diff + structure_diff;
        let scaled = Self::tier_factor(tier) * diff;

        // sigmoid：把差异映射到 0..1 胜率
        let raw = sigmoid(Self::K * scaled);
        // 夹到 5%..95%，保证总有爆冷概率与翻盘希望
        raw.clamp(0.05, 0.95)
    }

    /// 场内指令 → 实力加成（风格差异 + 暂停/印记 bonus）。
    fn style_bonus(directives: Option<MatchDirectives>) -> f64 {
        match directives {
            None => 0.0,
            Some(d) => {
                let style = match d.style {
                    Playstyle::Aggressive => 1.5,   // 激进：高风险高回报
                    Playstyle::Conservative => 0.5, // 保守：小幅稳健加成
                    Playstyle::Balanced => 0.0,
                };
                style + d.bonus as f64
            }
        }
    }

    /// 一方的综合心态：抗压 / 自信 / 士气 的均值。
    fn mentality(roster: &[&PlayerCharacter]) -> f64 {
        if roster.is_empty() {
            return 0.0;
        }
        roster
            .iter()
            .map(|pc| pc.pro.mentality + pc.pro.confidence + pc.pro.morale)
            .sum::<i32>() as f64
            / roster.len() as f64
            / 3.0
    }
}

/// 队伍当场合力：每名选手按其发挥标准差 σ 采样单场实际发挥后求和，
/// 再经体况（疲劳/伤病）折扣（= Kotlin `matchPower`）。
pub fn match_power(roster: &[&PlayerCharacter], rng: &mut Xoshiro256StarStar) -> f64 {
    if roster.is_empty() {
        return 0.0; // 空队
    }
    roster
        .iter()
        .map(|pc| {
            let raw = VolatilityModel::actual_power(pc, gaussian(rng));
            ConditionModel::effective_power(
                pc,
                raw,
                pc.is_player() && pc.career.as_ref().is_some_and(|c| c.resting),
            )
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::attributes::{
        BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
    };
    use csc_entities::role::Role;

    fn squad(power_level: i32, role: Role) -> Vec<PlayerCharacter> {
        (0..5)
            .map(|i| PlayerCharacter {
                id: csc_util::id::PlayerId(i),
                name: format!("p{i}"),
                age: 24,
                role,
                base: BaseAttributes {
                    reaction: power_level,
                    stability: power_level,
                    endurance: power_level,
                    stamina: power_level,
                    health: power_level,
                },
                skill: SkillAttributes {
                    aim: power_level,
                    leader: power_level,
                    communication: power_level,
                    clutch: power_level,
                },
                pro: ProAttributes {
                    mentality: power_level,
                    confidence: power_level,
                    team_spirit: power_level,
                    loyalty: power_level,
                    morale: power_level,
                },
                weapon: WeaponAttributes {
                    position: role,
                    ak: power_level,
                    awp: power_level,
                    pistol: power_level,
                    smoke: power_level,
                    utility: power_level,
                },
                potential: 100,
                fatigue: 0.0,
                injury: None,
                career: None,
                team: None,
                retired: false,
            })
            .collect()
    }

    #[test]
    fn tier_factors_match_kotlin() {
        assert_eq!(WinRateCalculator::tier_factor(TourneyTier::Major), 1.35);
        assert_eq!(
            WinRateCalculator::tier_factor(TourneyTier::SuperElite),
            1.20
        );
        assert_eq!(WinRateCalculator::tier_factor(TourneyTier::T1), 1.05);
        assert_eq!(WinRateCalculator::tier_factor(TourneyTier::T2), 0.90);
        assert_eq!(WinRateCalculator::tier_factor(TourneyTier::Qualify), 0.75);
    }

    #[test]
    fn stronger_team_wins_more_often() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let strong = squad(90, Role::Rifler);
        let weak = squad(60, Role::Rifler);
        let s: Vec<&PlayerCharacter> = strong.iter().collect();
        let w: Vec<&PlayerCharacter> = weak.iter().collect();
        let mut wins = 0;
        let n = 500;
        for _ in 0..n {
            let p = WinRateCalculator::win_rate(
                &s,
                &w,
                TourneyTier::T1,
                &mut rng,
                None,
                None,
                1.0,
                1.0,
            );
            if rng.roll_bp(p) {
                wins += 1;
            }
        }
        let rate = wins as f64 / n as f64;
        assert!(rate > 0.7, "强队胜率应显著高于 50%: {rate}");
    }

    #[test]
    fn win_rate_clamped_5_95() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let god = squad(99, Role::Rifler);
        let fodder = squad(20, Role::Rifler);
        let g: Vec<&PlayerCharacter> = god.iter().collect();
        let f: Vec<&PlayerCharacter> = fodder.iter().collect();
        for _ in 0..50 {
            let p = WinRateCalculator::win_rate(
                &g,
                &f,
                TourneyTier::T1,
                &mut rng,
                None,
                None,
                1.0,
                1.0,
            );
            assert!((0.05..=0.95).contains(&p), "胜率必须 clamp 在 5%~95%: {p}");
        }
    }

    #[test]
    fn directives_add_bonus() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let a = squad(80, Role::Rifler);
        let b = squad(80, Role::Rifler);
        let av: Vec<&PlayerCharacter> = a.iter().collect();
        let bv: Vec<&PlayerCharacter> = b.iter().collect();
        let base =
            WinRateCalculator::win_rate(&av, &bv, TourneyTier::T1, &mut rng, None, None, 1.0, 1.0);
        let mut rng2 = Xoshiro256StarStar::seed(42);
        let with_bonus = WinRateCalculator::win_rate(
            &av,
            &bv,
            TourneyTier::T1,
            &mut rng2,
            Some(MatchDirectives {
                style: Playstyle::Aggressive,
                bonus: 5,
            }),
            None,
            1.0,
            1.0,
        );
        assert!(
            with_bonus > base,
            "指令加成应提高胜率: {base} -> {with_bonus}"
        );
    }

    /// 结构因子（磨合/角色互补/队内关系）经差异通道影响胜率：
    /// 同阵容下结构更优的一方胜率更高；双方同构时与旧行为一致。
    #[test]
    fn structure_advantage_raises_win_rate() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let a = squad(80, Role::Rifler);
        let b = squad(80, Role::Rifler);
        let av: Vec<&PlayerCharacter> = a.iter().collect();
        let bv: Vec<&PlayerCharacter> = b.iter().collect();
        // 同构（结构因子相等）→ 与结构差为 0 完全一致
        let neutral_a =
            WinRateCalculator::win_rate(&av, &bv, TourneyTier::T1, &mut rng, None, None, 1.0, 1.0);
        let mut rng2 = Xoshiro256StarStar::seed(42);
        let neutral_b = WinRateCalculator::win_rate(
            &av,
            &bv,
            TourneyTier::T1,
            &mut rng2,
            None,
            None,
            1.15,
            1.15,
        );
        assert!(
            (neutral_a - neutral_b).abs() < 1e-12,
            "双方同构 → 结构项相消: {neutral_a} vs {neutral_b}"
        );
        // 我方结构优（磨合好）→ 胜率上升
        let mut rng3 = Xoshiro256StarStar::seed(42);
        let adv = WinRateCalculator::win_rate(
            &av,
            &bv,
            TourneyTier::T1,
            &mut rng3,
            None,
            None,
            1.15,
            1.0,
        );
        assert!(adv > neutral_a, "结构优势应提高胜率: {neutral_a} -> {adv}");
        // 我方结构劣（位置重叠/氛围差）→ 胜率下降
        let mut rng4 = Xoshiro256StarStar::seed(42);
        let dis =
            WinRateCalculator::win_rate(&av, &bv, TourneyTier::T1, &mut rng4, None, None, 0.9, 1.0);
        assert!(dis < neutral_a, "结构劣势应降低胜率: {neutral_a} -> {dis}");
    }

    #[test]
    fn empty_roster_zero_power() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let a = squad(80, Role::Rifler);
        let av: Vec<&PlayerCharacter> = a.iter().collect();
        let p =
            WinRateCalculator::win_rate(&av, &[], TourneyTier::T1, &mut rng, None, None, 1.0, 1.0);
        assert!((p - 0.95).abs() < 1e-9, "对空队应接近胜率上限: {p}");
    }

    /// 跨语言 golden：Kotlin `WinRateCalculator.winRate` 权威输出
    /// （A 队 5×RIFLER 全 80 vs B 队 5×RIFLER 全 70、T1、seed=42 连续 5 次；
    /// tools/gen_sim_golden.kt）。位模式断言——gaussian 的 ln/sqrt/cos 为 libm
    /// 函数，此 golden 验证 Windows/JVM 与 Rust 实测一致；若跨平台出现 1-2 ulp
    /// 差异属统计口径允许，可改统计断言。
    #[test]
    fn golden_win_rate_80_vs_70() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let a = squad(80, Role::Rifler);
        let b = squad(70, Role::Rifler);
        let av: Vec<&PlayerCharacter> = a.iter().collect();
        let bv: Vec<&PlayerCharacter> = b.iter().collect();
        let seq: Vec<u64> = (0..5)
            .map(|_| {
                WinRateCalculator::win_rate(
                    &av,
                    &bv,
                    TourneyTier::T1,
                    &mut rng,
                    None,
                    None,
                    1.0,
                    1.0,
                )
                .to_bits()
            })
            .collect();
        assert_eq!(
            seq,
            vec![
                0x3fea171b1c0562d1,
                0x3fe96ba9c9c17646,
                0x3fec5812ba2159fe,
                0x3fea322872b018f5,
                0x3fe80a6a159709af
            ]
        );
    }
}
