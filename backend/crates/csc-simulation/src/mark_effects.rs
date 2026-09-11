//! 生涯印记消费端（Kotlin `MarkEffects.kt` 转写）—— **flag 影响深远的唯一出口**。

use csc_entities::mark::{CareerMark, CareerMarkType, CareerMarks};

use crate::directives::Playstyle;

/// 生涯印记消费端——印记只在这里被消费（胜率 / 薪资 / 伤病 / 默认打法风格），
/// 业务代码不直接遍历 marks：新增印记类型只需改本文件 + 产生端。
///
/// 间接渠道：TOXIC / SUPPORTIVE_TEAMMATE 通过 ChemistryEngine 事件影响凝聚力，
/// 再经 `ChemistryModel::cohesion_factor` 进入胜率——避免同一印记被双重计算。
pub struct MarkEffects;

impl MarkEffects {
    /// 全部印记类型（= Kotlin `CareerMarkType.entries`，供遍历）。
    pub const ALL_TYPES: [CareerMarkType; 10] = [
        CareerMarkType::AggressivePlaystyle,
        CareerMarkType::ConservativePlaystyle,
        CareerMarkType::TacticalLeader,
        CareerMarkType::SupportiveTeammate,
        CareerMarkType::Toxic,
        CareerMarkType::ClutchSpecialist,
        CareerMarkType::HardWorker,
        CareerMarkType::Slacker,
        CareerMarkType::InjuryProne,
        CareerMarkType::SponsorMagnet,
    ];

    /// 每点印记的胜率实力加成（打法 + 大心脏）。
    fn power_bonus(r#type: CareerMarkType) -> f64 {
        match r#type {
            CareerMarkType::AggressivePlaystyle => 0.8,
            CareerMarkType::ConservativePlaystyle => 0.4,
            CareerMarkType::ClutchSpecialist => 1.2,
            _ => 0.0,
        }
    }

    /// 玩家印记 → 胜率实力加成（叠加后由 WinRateCalculator 消费）。
    pub fn win_rate_modifier(marks: &[CareerMark]) -> f64 {
        Self::ALL_TYPES
            .iter()
            .map(|t| Self::power_bonus(*t) * CareerMarks::strength_of(marks, *t) as f64)
            .sum()
    }

    /// 印记 → 薪资乘数（HARD_WORKER 溢价 / SLACKER 折价 / 明星效应）。
    pub fn salary_multiplier(marks: &[CareerMark]) -> f64 {
        let hard = CareerMarks::strength_of(marks, CareerMarkType::HardWorker) as f64;
        let slack = CareerMarks::strength_of(marks, CareerMarkType::Slacker) as f64;
        let magnet = CareerMarks::strength_of(marks, CareerMarkType::SponsorMagnet) as f64;
        1.0 + hard * 0.02 - slack * 0.03 + magnet * 0.05
    }

    /// 印记 → 受伤概率加成（INJURY_PRONE 每层 +2%，由 InjuryEngine 消费）。
    pub fn injury_prone_strength(marks: &[CareerMark]) -> i32 {
        CareerMarks::strength_of(marks, CareerMarkType::InjuryProne)
    }

    /// 印记 → 默认打法风格（场内干预选项第 0 项；多次激进 → 激进基线）。
    pub fn default_playstyle(marks: &[CareerMark]) -> Playstyle {
        let aggressive = CareerMarks::strength_of(marks, CareerMarkType::AggressivePlaystyle);
        let conservative = CareerMarks::strength_of(marks, CareerMarkType::ConservativePlaystyle);
        if aggressive > conservative {
            Playstyle::Aggressive
        } else if conservative > aggressive {
            Playstyle::Conservative
        } else {
            Playstyle::Balanced
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> Vec<CareerMark> {
        let mut m = Vec::new();
        CareerMarks::apply(&mut m, CareerMarkType::AggressivePlaystyle, 2026, "d1", 2);
        CareerMarks::apply(&mut m, CareerMarkType::ClutchSpecialist, 2026, "d2", 1);
        CareerMarks::apply(&mut m, CareerMarkType::HardWorker, 2026, "t", 1);
        CareerMarks::apply(&mut m, CareerMarkType::Slacker, 2026, "t", 1);
        CareerMarks::apply(&mut m, CareerMarkType::SponsorMagnet, 2026, "s", 1);
        CareerMarks::apply(&mut m, CareerMarkType::InjuryProne, 2026, "i", 3);
        m
    }

    #[test]
    fn win_rate_modifier_sums_bonuses() {
        // 2×AGGRESSIVE(0.8) + 1×CLUTCH(1.2) = 2.8；其它印记无加成
        assert!((MarkEffects::win_rate_modifier(&marks()) - 2.8).abs() < 1e-9);
        assert_eq!(MarkEffects::win_rate_modifier(&[]), 0.0);
    }

    #[test]
    fn salary_multiplier_combines() {
        // 1×HARD(+0.02) - 1×SLACK(-0.03) + 1×MAGNET(+0.05) = 1.04
        assert!((MarkEffects::salary_multiplier(&marks()) - 1.04).abs() < 1e-12);
        assert_eq!(MarkEffects::salary_multiplier(&[]), 1.0);
    }

    #[test]
    fn injury_prone_strength() {
        assert_eq!(MarkEffects::injury_prone_strength(&marks()), 3);
        assert_eq!(MarkEffects::injury_prone_strength(&[]), 0);
    }

    #[test]
    fn default_playstyle_by_marks() {
        assert_eq!(
            MarkEffects::default_playstyle(&marks()),
            Playstyle::Aggressive
        );
        // 保守 > 激进
        let mut m = Vec::new();
        CareerMarks::apply(&mut m, CareerMarkType::ConservativePlaystyle, 2026, "d", 3);
        assert_eq!(MarkEffects::default_playstyle(&m), Playstyle::Conservative);
        // 平手 → 平衡
        assert_eq!(MarkEffects::default_playstyle(&[]), Playstyle::Balanced);
    }
}
