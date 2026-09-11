//! 赛季目标（2026 产品层）：玩家为当前赛季选择一个主目标。
//!
//! 目标不只是展示标签——训练系统会读取它做差异化调制（例如冲 Rating 的
//! 专项训练效率提升、恢复状态目标让休息恢复更多）。

use serde::{Deserialize, Serialize};

/// 赛季主目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SeasonGoal {
    /// 争夺冠军（Major/T1 团队荣誉）
    WinTitle,
    /// 冲进年度 TOP20
    Top20,
    /// 提升个人 Rating
    ImproveRating,
    /// 稳固首发地位
    SecureStarting,
    /// 寻求转会
    SeekTransfer,
    /// 恢复身体状态
    RecoverForm,
}

impl SeasonGoal {
    pub const fn name(self) -> &'static str {
        match self {
            Self::WinTitle => "WIN_TITLE",
            Self::Top20 => "TOP20",
            Self::ImproveRating => "IMPROVE_RATING",
            Self::SecureStarting => "SECURE_STARTING",
            Self::SeekTransfer => "SEEK_TRANSFER",
            Self::RecoverForm => "RECOVER_FORM",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "WIN_TITLE" => Some(Self::WinTitle),
            "TOP20" => Some(Self::Top20),
            "IMPROVE_RATING" => Some(Self::ImproveRating),
            "SECURE_STARTING" => Some(Self::SecureStarting),
            "SEEK_TRANSFER" => Some(Self::SeekTransfer),
            "RECOVER_FORM" => Some(Self::RecoverForm),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_and_names_roundtrip() {
        for goal in [
            SeasonGoal::WinTitle,
            SeasonGoal::Top20,
            SeasonGoal::ImproveRating,
            SeasonGoal::SecureStarting,
            SeasonGoal::SeekTransfer,
            SeasonGoal::RecoverForm,
        ] {
            assert_eq!(
                serde_json::to_string(&goal).unwrap(),
                format!("\"{}\"", goal.name())
            );
            assert_eq!(SeasonGoal::from_name(goal.name()), Some(goal));
        }
        assert_eq!(SeasonGoal::from_name("NOPE"), None);
    }
}
