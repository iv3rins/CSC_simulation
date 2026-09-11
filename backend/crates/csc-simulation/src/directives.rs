//! 场内干预指令 + 打法风格 + 失误类型（Kotlin `MatchDirectives.kt` 转写）。

use serde::{Deserialize, Serialize};

/// 打法风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Playstyle {
    /// 激进攻防：高风险高回报（实力加成大，方差大）
    Aggressive,
    /// 平衡
    Balanced,
    /// 保守稳扎：低风险（小加成，方差小）
    Conservative,
}

impl Playstyle {
    /// 中文标签（= Kotlin `label`）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Aggressive => "激进打法",
            Self::Balanced => "平衡打法",
            Self::Conservative => "保守打法",
        }
    }
}

/// 场内干预指令——玩家对比赛的战术输入（纯值，跨引擎传递）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchDirectives {
    /// 当图打法风格
    pub style: Playstyle,
    /// 综合加成（暂停/鼓励/生涯印记修正 → 胜率实力差加成）
    pub bonus: i32,
}

impl Default for MatchDirectives {
    fn default() -> Self {
        Self {
            style: Playstyle::Balanced,
            bonus: 0,
        }
    }
}

impl MatchDirectives {
    /// 无指令（非玩家对局 / 默认；= Kotlin `MatchDirectives.NONE`）。
    pub const NONE: MatchDirectives = MatchDirectives {
        style: Playstyle::Balanced,
        bonus: 0,
    };
}

/// 队友失误类型（"犯罪/打差了"瞬间的叙事分类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlunderKind {
    /// 关键局白给（残局/1v1 失误）
    Choke,
    /// 枪法拉胯（对枪连败，数据垫底）
    Miscarry,
    /// 心态崩（连败后操作变形）
    Tilt,
    /// 注意力不集中（失误频出）
    LackOfFocus,
}

impl BlunderKind {
    /// 中文标签（= Kotlin `label`）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Choke => "关键局白给",
            Self::Miscarry => "枪法拉胯",
            Self::Tilt => "心态崩了",
            Self::LackOfFocus => "注意力涣散",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_kotlin() {
        assert_eq!(Playstyle::Aggressive.label(), "激进打法");
        assert_eq!(BlunderKind::Choke.label(), "关键局白给");
        assert_eq!(BlunderKind::LackOfFocus.label(), "注意力涣散");
    }

    #[test]
    fn none_directives_defaults() {
        assert_eq!(MatchDirectives::NONE, MatchDirectives::default());
        assert_eq!(MatchDirectives::NONE.style, Playstyle::Balanced);
        assert_eq!(MatchDirectives::NONE.bonus, 0);
    }

    #[test]
    fn serde_names_match_kotlin() {
        assert_eq!(
            serde_json::to_string(&Playstyle::Aggressive).unwrap(),
            r#""AGGRESSIVE""#
        );
        assert_eq!(
            serde_json::to_string(&BlunderKind::LackOfFocus).unwrap(),
            r#""LACK_OF_FOCUS""#
        );
    }
}
