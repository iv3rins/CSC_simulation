//! 决策日志（Kotlin `DecisionLog.kt` 转写）——可复现性的第二根支柱。

use serde::{Deserialize, Serialize};

use crate::point::PlayerDecision;

/// 决策日志——记录每次决策（纯值），随 GameState 存档；
/// **重放 = 相同种子 + 相同决策序列 → 完全一致的模拟**。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionLog {
    entries: Vec<PlayerDecision>,
}

impl DecisionLog {
    /// 记录一个决策。
    pub fn record(&mut self, decision: PlayerDecision) {
        self.entries.push(decision);
    }

    /// 批量记录（决策批次 / 比赛级现场）。
    pub fn record_all(&mut self, decisions: Vec<PlayerDecision>) {
        self.entries.extend(decisions);
    }

    /// 全部决策（只读快照）。
    pub fn entries(&self) -> Vec<PlayerDecision> {
        self.entries.clone()
    }

    /// 恢复（读档用）。
    pub fn restore(&mut self, entries: Vec<PlayerDecision>) {
        self.entries = entries;
    }

    /// 决策总数。
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_restore() {
        let mut log = DecisionLog::default();
        log.record(PlayerDecision::new("p1", "AIM"));
        log.record_all(vec![
            PlayerDecision::new("p2", "ACCEPT"),
            PlayerDecision::new("p3", "STAY"),
        ]);
        assert_eq!(log.count(), 3);
        let snap = log.entries();
        let mut log2 = DecisionLog::default();
        log2.restore(snap);
        assert_eq!(log2.count(), 3);
    }

    #[test]
    fn serde_roundtrip() {
        let mut log = DecisionLog::default();
        log.record(PlayerDecision::new("p1", "AIM"));
        let json = serde_json::to_string(&log).unwrap();
        let back: DecisionLog = serde_json::from_str(&json).unwrap();
        assert_eq!(log, back);
    }
}
