//! 伤病领域模型 —— 伤病/体能系统的值对象（Kotlin `Injury.kt`）。

use serde::{Deserialize, Serialize};

/// 伤病严重度（决定实力折扣与恢复时长；时长按月计，月度 tick 扣减）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InjurySeverity {
    /// 轻伤（手腕疲劳、肌肉酸痛）：实力 -10%，约 1 个月恢复。
    Minor,
    /// 中等（肌肉拉伤、背伤）：实力 -25%，约 2 个月。
    Moderate,
    /// 重伤（骨折、手术）：实力 -45%，约 4 个月。
    Severe,
}

impl InjurySeverity {
    /// 实力折扣（0..1；`penaltyFactor = 1 - penalty`）。
    pub const fn penalty(self) -> f64 {
        match self {
            Self::Minor => 0.10,
            Self::Moderate => 0.25,
            Self::Severe => 0.45,
        }
    }

    /// 恢复月数。
    pub const fn recovery_days(self) -> i32 {
        match self {
            Self::Minor => 1,
            Self::Moderate => 2,
            Self::Severe => 4,
        }
    }

    /// 中文标签（叙事用）。
    pub const fn label_cn(self) -> &'static str {
        match self {
            Self::Minor => "轻伤",
            Self::Moderate => "中等伤势",
            Self::Severe => "重伤",
        }
    }
}

/// 伤病部位/类型（影响叙事与恢复，信息用途为主）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InjuryKind {
    Wrist,
    Finger,
    Shoulder,
    Back,
    Leg,
    EyeStrain,
    Illness,
}

impl InjuryKind {
    /// 中文标签（叙事用；= Kotlin `label` 字段）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wrist => "手腕",
            Self::Finger => "手指",
            Self::Shoulder => "肩膀",
            Self::Back => "背部",
            Self::Leg => "腿部",
            Self::EyeStrain => "视力疲劳",
            Self::Illness => "疾病",
        }
    }
}

/// 一条伤病（纯值，随实体状态演化）。
///
/// `days_left` 为剩余恢复**月数**（月度 tick 扣减；`healed` 后由 InjuryEngine 清除；
/// 受伤当月不恢复——首个 tick 只置 `ticked`，决策生效后从次月扣减）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Injury {
    pub kind: InjuryKind,
    pub severity: InjurySeverity,
    pub days_left: i32,
    /// 受伤日期（dateLabel，如 `2026-06-08`）
    pub sustained_date: String,
    /// 来源（决策点 id / 系统名），可追溯
    pub source: String,
    /// 是否已过首个恢复 tick（受伤当月保护，防当月即愈）
    pub ticked: bool,
}

impl Injury {
    /// 带伤月度扣减。
    pub fn tick(&self) -> Self {
        Self {
            days_left: (self.days_left - 1).max(0),
            ticked: true,
            ..self.clone()
        }
    }

    /// 休养月度扣减（双倍恢复）。
    pub fn tick_resting(&self) -> Self {
        Self {
            days_left: (self.days_left - 2).max(0),
            ticked: true,
            ..self.clone()
        }
    }

    /// 是否已痊愈（≤ 0 天）。
    pub fn healed(&self) -> bool {
        self.days_left <= 0
    }

    /// 实力折扣乘数（0..1，1 = 无影响）。
    pub fn penalty_factor(&self) -> f64 {
        1.0 - self.severity.penalty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn injury(days: i32) -> Injury {
        Injury {
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Minor,
            days_left: days,
            sustained_date: "2026-06-08".into(),
            source: "test".into(),
            ticked: false,
        }
    }

    #[test]
    fn severity_penalty_and_recovery_match_kotlin() {
        assert_eq!(InjurySeverity::Minor.penalty(), 0.10);
        assert_eq!(InjurySeverity::Moderate.penalty(), 0.25);
        assert_eq!(InjurySeverity::Severe.penalty(), 0.45);
        assert_eq!(InjurySeverity::Minor.recovery_days(), 1);
        assert_eq!(InjurySeverity::Moderate.recovery_days(), 2);
        assert_eq!(InjurySeverity::Severe.recovery_days(), 4);
        // penalty_factor 是 Injury 的方法（1 - severity.penalty）
        assert_eq!(1.0 - InjurySeverity::Severe.penalty(), 0.55);
    }

    #[test]
    fn tick_and_healed_semantics() {
        let i = injury(1);
        assert!(!i.healed());
        let t = i.tick();
        assert!(t.healed());
        assert!(t.ticked);
        // 不会扣成负数
        let t2 = t.tick();
        assert_eq!(t2.days_left, 0);
    }

    #[test]
    fn tick_resting_double_recovery() {
        let i = injury(3);
        assert_eq!(i.tick_resting().days_left, 1);
        let i = injury(1);
        assert_eq!(i.tick_resting().days_left, 0);
    }

    #[test]
    fn first_tick_protection_flag() {
        let i = injury(2);
        assert!(!i.ticked);
        assert!(i.tick().ticked);
    }

    #[test]
    fn serde_roundtrip() {
        let i = injury(2);
        let json = serde_json::to_string(&i).unwrap();
        let back: Injury = serde_json::from_str(&json).unwrap();
        assert_eq!(i, back);
    }
}
