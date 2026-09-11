//! 队内氛围与关系矩阵（Kotlin `TeamChemistry.kt`）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 队内氛围与关系矩阵（队伍实体携带的**运行时状态**）。
///
/// - `cohesion`：队内凝聚力 0..100（由关系矩阵派生，矛盾/鼓励事件直接修正）；
/// - `relations`：选手名 → (目标名 → 0..100 关系值，默认 50)。
///   关系同时存在于 NPC 与玩家之间（NPC 也有立场：被指责会记仇）。
///
/// 与 Kotlin 一致：关系针对"名字"而非对象，新队员关系默认 50（`relation_of` 兜底）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamChemistry {
    /// 队内凝聚力（0..100；50 = 中性）
    pub cohesion: f64,
    /// 关系矩阵（选手名 → 目标名 → 关系值）
    pub relations: HashMap<String, HashMap<String, f64>>,
}

impl Default for TeamChemistry {
    fn default() -> Self {
        Self {
            cohesion: 50.0,
            relations: HashMap::new(),
        }
    }
}

impl TeamChemistry {
    /// a 对 b 的关系（未建立则默认 50）。
    pub fn relation_of(&self, a: &str, b: &str) -> f64 {
        self.relations
            .get(a)
            .and_then(|m| m.get(b))
            .copied()
            .unwrap_or(50.0)
    }

    /// 设置 a 对 b 的关系（clamp 0..100）。
    pub fn set_relation(&mut self, a: &str, b: &str, value: f64) {
        self.relations
            .entry(a.to_string())
            .or_default()
            .insert(b.to_string(), value.clamp(0.0, 100.0));
    }

    /// 双向调整（对称关系：a↔b 同步变动；delta 可正可负）。
    pub fn adjust(&mut self, a: &str, b: &str, delta: f64) {
        self.set_relation(a, b, self.relation_of(a, b) + delta);
        self.set_relation(b, a, self.relation_of(b, a) + delta);
    }

    /// 队伍内全部选手间的平均关系（cohesion 的派生源）。
    /// roster 名用于限定"现役成员"的关系（退役/转会者不计入）。
    pub fn average_relation(&self, roster_names: &[String]) -> f64 {
        let mut sum = 0.0;
        let mut count = 0;
        for a in roster_names {
            for b in roster_names {
                if a == b {
                    continue;
                }
                sum += self.relation_of(a, b);
                count += 1;
            }
        }
        if count == 0 { 50.0 } else { sum / count as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_defaults_to_50() {
        let c = TeamChemistry::default();
        assert_eq!(c.relation_of("a", "b"), 50.0);
        assert_eq!(c.cohesion, 50.0);
    }

    #[test]
    fn set_relation_clamps() {
        let mut c = TeamChemistry::default();
        c.set_relation("a", "b", 150.0);
        assert_eq!(c.relation_of("a", "b"), 100.0);
        c.set_relation("a", "b", -10.0);
        assert_eq!(c.relation_of("a", "b"), 0.0);
    }

    #[test]
    fn adjust_is_symmetric() {
        let mut c = TeamChemistry::default();
        c.adjust("a", "b", 10.0);
        assert_eq!(c.relation_of("a", "b"), 60.0);
        assert_eq!(c.relation_of("b", "a"), 60.0);
        c.adjust("a", "b", -25.0);
        assert_eq!(c.relation_of("a", "b"), 35.0);
        assert_eq!(c.relation_of("b", "a"), 35.0);
    }

    #[test]
    fn average_relation_uses_roster_names_only() {
        let mut c = TeamChemistry::default();
        c.adjust("a", "b", 10.0); // 60
        c.set_relation("a", "c", 90.0); // c 已离队，不应计入
        let roster = vec!["a".to_string(), "b".to_string()];
        assert_eq!(c.average_relation(&roster), 60.0);
        // 单人不计（count=0 → 50 兜底）
        assert_eq!(c.average_relation(&["a".to_string()]), 50.0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut c = TeamChemistry::default();
        c.adjust("a", "b", 5.0);
        c.cohesion = 60.0;
        let json = serde_json::to_string(&c).unwrap();
        let back: TeamChemistry = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
