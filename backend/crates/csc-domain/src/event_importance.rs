//! 赛事体验分层：世界赛事完整运行，但玩家只在高价值节点进入 LIVE。

use serde::{Deserialize, Serialize};

/// 赛事对职业生涯的展示重要性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventImportance {
    /// T2/T3、普通资格赛和背景赛事：自动模拟。
    #[default]
    Background,
    /// 对排名、资格或职业声誉有明显影响的赛事。
    Important,
    /// Major 及其关键资格阶段。
    Major,
    /// Major 淘汰赛、决赛等冠军节点。
    Championship,
}

impl EventImportance {
    /// 是否应进入玩家可操作的 LIVE 体验。
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Major | Self::Championship)
    }

    /// 是否应在首页倒计时和职业节点中突出显示。
    pub const fn is_career_milestone(self) -> bool {
        !matches!(self, Self::Background)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_high_value_events_are_live() {
        assert!(!EventImportance::Background.is_live());
        assert!(!EventImportance::Important.is_live());
        assert!(EventImportance::Major.is_live());
        assert!(EventImportance::Championship.is_live());
    }
}
