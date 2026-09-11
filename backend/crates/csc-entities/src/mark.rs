//! 生涯印记（Career Mark / Flag）—— 玩家场内操作与决策留下的**长期影响标记**
//! （Kotlin `CareerMark.kt`）。

use serde::{Deserialize, Serialize};

/// 印记类型（10 种；消费端统一收敛在 `csc-simulation::mark_effects`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CareerMarkType {
    /// 打法风格：激进攻防（场内多次选 AGGRESSIVE 累积）
    AggressivePlaystyle,
    /// 打法风格：保守稳扎（多次选 CONSERVATIVE 累积）
    ConservativePlaystyle,
    /// 场内领导：常叫暂停/战术调整，队友信服
    TacticalLeader,
    /// 队内角色：支持型队友（鼓励、补士气）
    SupportiveTeammate,
    /// 队内角色：毒瘤（指责、甩锅，损害凝聚力）
    Toxic,
    /// 心理属性：大心脏（关键局敢打，胜率修正）
    ClutchSpecialist,
    /// 职业素养：训练狂（薪资溢价）
    HardWorker,
    /// 职业素养：训练懒散（薪资折价）
    Slacker,
    /// 伤病史：玻璃人（多次伤病后累积，受伤概率上升）
    InjuryProne,
    /// 经济：代言明星（长期高声誉 + 代言收入）
    SponsorMagnet,
}

/// 一条印记。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CareerMark {
    pub r#type: CareerMarkType,
    /// 产生年份
    pub year: i32,
    /// 来源（决策点 id / 系统名），可追溯
    pub source: String,
    /// 强度（同类叠加）
    pub strength: i32,
}

/// 印记集合工具（纯函数，无状态）。
pub struct CareerMarks;

impl CareerMarks {
    /// 叠加一条印记：同类合并强度（保留最早年份与来源描述），
    /// 避免同一类型反复 append 导致列表膨胀。
    pub fn apply(
        marks: &mut Vec<CareerMark>,
        r#type: CareerMarkType,
        year: i32,
        source: &str,
        strength: i32,
    ) {
        if let Some(idx) = marks.iter().position(|m| m.r#type == r#type) {
            marks[idx].strength += strength;
        } else {
            marks.push(CareerMark {
                r#type,
                year,
                source: source.to_string(),
                strength,
            });
        }
    }

    /// 某类型印记的总强度（0 = 无）。
    pub fn strength_of(marks: &[CareerMark], r#type: CareerMarkType) -> i32 {
        marks
            .iter()
            .filter(|m| m.r#type == r#type)
            .map(|m| m.strength)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_merges_same_type_keeping_first_year() {
        let mut marks = Vec::new();
        CareerMarks::apply(
            &mut marks,
            CareerMarkType::AggressivePlaystyle,
            2026,
            "decision-1",
            1,
        );
        CareerMarks::apply(
            &mut marks,
            CareerMarkType::AggressivePlaystyle,
            2027,
            "decision-2",
            1,
        );
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].year, 2026, "保留最早年份");
        assert_eq!(marks[0].source, "decision-1", "保留最早来源");
        assert_eq!(marks[0].strength, 2, "强度合并");
    }

    #[test]
    fn apply_distinct_types_append() {
        let mut marks = Vec::new();
        CareerMarks::apply(&mut marks, CareerMarkType::Toxic, 2026, "blunder", 1);
        CareerMarks::apply(
            &mut marks,
            CareerMarkType::ClutchSpecialist,
            2026,
            "clutch",
            2,
        );
        assert_eq!(marks.len(), 2);
        assert_eq!(CareerMarks::strength_of(&marks, CareerMarkType::Toxic), 1);
        assert_eq!(
            CareerMarks::strength_of(&marks, CareerMarkType::ClutchSpecialist),
            2
        );
        assert_eq!(CareerMarks::strength_of(&marks, CareerMarkType::Slacker), 0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut marks = Vec::new();
        CareerMarks::apply(&mut marks, CareerMarkType::InjuryProne, 2026, "injury", 3);
        let json = serde_json::to_string(&marks).unwrap();
        assert!(json.contains(r#""INJURY_PRONE""#));
        let back: Vec<CareerMark> = serde_json::from_str(&json).unwrap();
        assert_eq!(marks, back);
    }
}
