//! 单场发挥波动模型（Kotlin `VolatilityModel.kt` 转写）。

use csc_entities::character::PlayerCharacter;
use csc_entities::power::PowerCalculator;
use csc_entities::role_profile::RoleProfiles;

/// 单场发挥波动模型（正态分布波动）。
///
/// 选手单场实际发挥：
/// ```text
/// P_actual = clamp(P_base + N(0,1) × σ_player, P_min, P_max)
/// ```
pub struct VolatilityModel;

impl VolatilityModel {
    /// 基础波动常量（控制 σ 的整体量级，可调）。
    pub const BASE_VOLATILITY: f64 = 30.0;

    /// 单场发挥的下限 / 上限（防止数值崩坏）。
    pub const P_MIN: f64 = 0.0;
    pub const P_MAX: f64 = 100.0;

    /// 选手的发挥标准差 σ_player。
    ///
    /// 由角色固有波动 × 状态不稳定系数 × 年龄修正共同决定：
    /// 稳定性/心态/自信越低 → (1−x/100) 越大 → 越不稳定。
    pub fn sigma_of(pc: &PlayerCharacter) -> f64 {
        let role_vol = RoleProfiles::of(pc.role).volatility; // 角色固有波动（ENTRY 大、IGL 小）
        let instability = role_vol
            * (1.0
                + 0.8 * (1.0 - pc.base.stability as f64 / 100.0) // 稳定性低 → 波动大（权重 0.8）
                + 0.6 * (1.0 - pc.pro.mentality as f64 / 100.0) // 抗压低 → 波动大（权重 0.6）
                + 0.4 * (1.0 - pc.pro.confidence as f64 / 100.0)) // 自信低 → 波动大（权重 0.4）
            * Self::age_factor(pc.age); // 年龄修正
        Self::BASE_VOLATILITY * instability // 映射到实际 σ 量级
    }

    /// 采样一次单场实际发挥：`P_actual = clamp(P_base + N(0,1) × σ_player, P_MIN, P_MAX)`。
    ///
    /// @param normal 标准正态随机数 N(0,1)（由调用方经 `csc-util::sampling::gaussian` 采样）
    pub fn actual_power(pc: &PlayerCharacter, normal: f64) -> f64 {
        (PowerCalculator::player_power(pc) + normal * Self::sigma_of(pc))
            .clamp(Self::P_MIN, Self::P_MAX)
    }

    /// 年龄对波动的修正：年轻毛躁 / 巅峰稳定 / 老将起伏（= Kotlin `ageFactor`）。
    pub fn age_factor(age: i32) -> f64 {
        match age {
            a if a < 23 => 1.25, // 18~22：年轻手感起伏大
            23..=25 => 1.0,      // 23~25：巅峰期最稳
            _ => 1.2,            // 26+：老将状态波动回升
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(stability: i32, mentality: i32, confidence: i32) -> PlayerCharacter {
        csc_entities::character::PlayerCharacter {
            id: csc_util::id::PlayerId(0),
            name: "t".into(),
            age: 24,
            role: csc_entities::role::Role::Rifler,
            base: csc_entities::attributes::BaseAttributes {
                reaction: 80,
                stability,
                endurance: 80,
                stamina: 80,
                health: 80,
            },
            skill: csc_entities::attributes::SkillAttributes {
                aim: 80,
                leader: 50,
                communication: 50,
                clutch: 70,
            },
            pro: csc_entities::attributes::ProAttributes {
                mentality,
                confidence,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: csc_entities::attributes::WeaponAttributes {
                position: csc_entities::role::Role::Rifler,
                ak: 80,
                awp: 50,
                pistol: 70,
                smoke: 50,
                utility: 50,
            },
            potential: 90,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        }
    }

    #[test]
    fn age_factor_matches_kotlin() {
        assert_eq!(VolatilityModel::age_factor(18), 1.25);
        assert_eq!(VolatilityModel::age_factor(22), 1.25);
        assert_eq!(VolatilityModel::age_factor(23), 1.0);
        assert_eq!(VolatilityModel::age_factor(25), 1.0);
        assert_eq!(VolatilityModel::age_factor(26), 1.2);
        assert_eq!(VolatilityModel::age_factor(35), 1.2);
    }

    #[test]
    fn low_stability_increases_sigma() {
        let stable = pc(90, 90, 90);
        let neuro = pc(10, 10, 10); // 神经刀：稳定性/心态/自信全低
        assert!(
            VolatilityModel::sigma_of(&neuro) > VolatilityModel::sigma_of(&stable),
            "低稳定/心态应放大波动"
        );
    }

    #[test]
    fn actual_power_clamped() {
        let p = pc(50, 50, 50);
        // 极端正态值 → clamp 到 [0, 100]
        assert_eq!(VolatilityModel::actual_power(&p, 1e6), 100.0);
        assert_eq!(VolatilityModel::actual_power(&p, -1e6), 0.0);
        // 零波动 → 基础实力
        let normal0 = VolatilityModel::actual_power(&p, 0.0);
        assert!((normal0 - csc_entities::power::PowerCalculator::player_power(&p)).abs() < 1e-9);
    }
}
