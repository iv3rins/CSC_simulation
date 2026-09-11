//! 体况模型（Kotlin `ConditionModel.kt` 转写）——伤病/体能规则的**纯函数层**。

use csc_entities::character::PlayerCharacter;
use csc_entities::injury::{Injury, InjuryKind, InjurySeverity};
use csc_util::rng::Xoshiro256StarStar;

/// 体况模型——疲劳/伤病对实力的折扣、恢复速率、受伤概率全部收敛于此；
/// 触发与结算在 `csc-systems::injury`，实体只携带状态（fatigue / injury）。
pub struct ConditionModel;

impl ConditionModel {
    // —— 疲劳 ——

    /// 每张地图的疲劳消耗（线下 +1.5：旅途/舞台压力更大）
    pub const FATIGUE_PER_MAP: f64 = 3.0;
    pub const FATIGUE_LAN_BONUS: f64 = 1.5;
    /// 疲劳上限
    pub const FATIGUE_MAX: f64 = 100.0;

    /// 一场系列赛的疲劳消耗。
    pub fn fatigue_cost(maps: i32, lan: bool) -> f64 {
        maps as f64 * (Self::FATIGUE_PER_MAP + if lan { Self::FATIGUE_LAN_BONUS } else { 0.0 })
    }

    /// 月度自然恢复（未参赛月恢复约 60%；休养决策额外加成由 InjuryEngine 处理）。
    pub fn monthly_recovery() -> f64 {
        60.0
    }

    /// 疲劳对实力的折扣乘数：0 → 1.0，100 → 0.82（线性）。
    pub fn fatigue_penalty(fatigue: f64) -> f64 {
        1.0 - 0.18 * (fatigue.clamp(0.0, Self::FATIGUE_MAX) / Self::FATIGUE_MAX)
    }

    /// 伤病对实力的折扣乘数（休养状态半效，见 InjuryEngine）。
    pub fn injury_penalty(injury: Option<&Injury>, resting: bool) -> f64 {
        match injury {
            None => 1.0,
            Some(i) => 1.0 - i.severity.penalty() * if resting { 0.5 } else { 1.0 },
        }
    }

    /// 综合实力折扣（疲劳 + 伤病）。
    pub fn effective_power(pc: &PlayerCharacter, raw_power: f64, resting: bool) -> f64 {
        raw_power
            * Self::fatigue_penalty(pc.fatigue)
            * Self::injury_penalty(pc.injury.as_ref(), resting)
    }

    // —— 伤病概率 ——

    /// 基础受伤概率（健康 90+、年轻 → 低）。
    pub fn injury_chance(age: i32, health: i32, fatigue: f64, injury_prone_strength: i32) -> f64 {
        let health_factor = 0.09 - 0.06 * (health.clamp(0, 100) as f64 / 100.0); // 健康 100 → 3%，健康 50 → 6%
        let age_factor = if age >= 28 { 0.02 } else { 0.0 }; // 老将 +2%
        let fatigue_factor = 0.05 * (fatigue / Self::FATIGUE_MAX); // 疲劳 100 → +5%
        let prone_factor = 0.02 * injury_prone_strength as f64; // 玻璃人印记 +2%/层
        (health_factor + age_factor + fatigue_factor + prone_factor).clamp(0.0, 0.35)
    }

    /// 受伤时按权重掷部位（= Kotlin `rollKind`：`nextInt(10)` 单参数原语）。
    pub fn roll_kind(rng: &mut Xoshiro256StarStar) -> InjuryKind {
        match rng.next_i32_bound(10) {
            0..=2 => InjuryKind::Wrist,
            3..=4 => InjuryKind::Finger,
            5 => InjuryKind::Shoulder,
            6 => InjuryKind::Back,
            7 => InjuryKind::Leg,
            8 => InjuryKind::EyeStrain,
            _ => InjuryKind::Illness,
        }
    }

    /// 受伤时掷严重度（= Kotlin `rollSeverity`；轻伤为主，重伤罕见；
    /// 整数化判定 D2：roll_bp 基点比较）。
    pub fn roll_severity(rng: &mut Xoshiro256StarStar) -> InjurySeverity {
        if rng.roll_bp(0.65) {
            InjurySeverity::Minor
        } else if rng.roll_bp(0.85) {
            InjurySeverity::Moderate
        } else {
            InjurySeverity::Severe
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::injury::Injury;

    #[test]
    fn fatigue_penalty_linear() {
        assert_eq!(ConditionModel::fatigue_penalty(0.0), 1.0);
        // 0.82 的 IEEE754 表示为 0.8200000000000001（Kotlin 同值）——用近似断言
        assert!((ConditionModel::fatigue_penalty(100.0) - 0.82).abs() < 1e-12);
        assert!((ConditionModel::fatigue_penalty(50.0) - 0.91).abs() < 1e-12);
        // clamp：超出上限按 100 计
        assert!((ConditionModel::fatigue_penalty(150.0) - 0.82).abs() < 1e-12);
    }

    #[test]
    fn fatigue_cost_lan_bonus() {
        assert_eq!(ConditionModel::fatigue_cost(3, false), 9.0);
        assert_eq!(ConditionModel::fatigue_cost(3, true), 13.5);
    }

    #[test]
    fn injury_penalty_resting_half() {
        let injury = Injury {
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Severe,
            days_left: 2,
            sustained_date: "2026-06-08".into(),
            source: "t".into(),
            ticked: false,
        };
        // SEVERE：-45%；休养半效：-22.5%
        assert!((ConditionModel::injury_penalty(Some(&injury), false) - 0.55).abs() < 1e-12);
        assert!((ConditionModel::injury_penalty(Some(&injury), true) - 0.775).abs() < 1e-12);
        assert_eq!(ConditionModel::injury_penalty(None, false), 1.0);
    }

    #[test]
    fn injury_chance_factors() {
        // 健康 100 年轻无疲劳无印记
        assert!((ConditionModel::injury_chance(20, 100, 0.0, 0) - 0.03).abs() < 1e-12);
        // 健康 50：6%
        assert!((ConditionModel::injury_chance(20, 50, 0.0, 0) - 0.06).abs() < 1e-12);
        // 老将 +2%
        assert!((ConditionModel::injury_chance(30, 100, 0.0, 0) - 0.05).abs() < 1e-12);
        // 疲劳 100 +5%
        assert!((ConditionModel::injury_chance(20, 100, 100.0, 0) - 0.08).abs() < 1e-12);
        // 玻璃人 3 层 +6%
        assert!((ConditionModel::injury_chance(20, 100, 0.0, 3) - 0.09).abs() < 1e-12);
        // clamp 上限 35%
        assert!(ConditionModel::injury_chance(30, 0, 100.0, 50) <= 0.35);
    }

    #[test]
    fn roll_kind_and_severity() {
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..100 {
            let k = ConditionModel::roll_kind(&mut rng);
            let s = ConditionModel::roll_severity(&mut rng);
            // 枚举成员合法即可（分布测试在统计口径）
            let _ = (k, s);
        }
        // 严重度分布粗验：MINOR 应占多数
        let mut rng = Xoshiro256StarStar::seed(7);
        let mut minor = 0;
        for _ in 0..1000 {
            if ConditionModel::roll_severity(&mut rng) == InjurySeverity::Minor {
                minor += 1;
            }
        }
        assert!(minor > 550, "MINOR 应占多数: {minor}/1000");
    }
}
