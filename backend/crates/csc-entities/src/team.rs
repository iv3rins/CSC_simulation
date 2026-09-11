//! CS 战队实体（Kotlin `Team.kt` 的 arena 形态转写）。

use csc_util::id::{PlayerId, TeamId};
use serde::{Deserialize, Serialize};

use crate::chemistry::TeamChemistry;

/// CS 战队实体。
///
/// 转写差异（对象图 → ID 引用）：
/// - Kotlin `roster: List<PlayerCharacter>`（持有对象引用）→ **`roster_ids: Vec<PlayerId>`**；
/// - Kotlin `Team.with*` 返回新 Team（不可变快照）→ 本结构为**可变实体**，
///   换人经 [`crate::world::World`] 的原子操作（`assign_player_to_team`/`release_player`）
///   同步维护「roster_ids ⟷ 成员 team 字段」双端不变量；
/// - Kotlin `signature`（队名+阵容派生）保留为 `signature()` 方法——供与 VRS 数据
///   （来自 JSON 的真实 roster 名单）匹配，引用/档案一律用 `id`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    /// 稳定 ID（arena 索引）
    pub id: TeamId,
    /// 队伍昵称，如 "Vitality"（展示字段）
    pub name: String,
    /// 阵容（选手 ID 列表，通常 5 名；**不可变快照语义由 World 原子操作保证**）
    pub roster_ids: Vec<PlayerId>,
    /// VRS 世界排名缓存（1 起，越小越强；由 VRS 系统单向刷新）
    pub vrs_ranking: i32,
    /// VRS 积分缓存（由 VRS 系统单向刷新）
    pub vrs_value: i32,
    /// 队伍财政预算（奖金增加、薪资支出；转会候选要求预算 ≥ 目标薪资）
    pub budget: i64,
    /// 队内氛围与关系矩阵（运行时可变状态）
    pub chemistry: TeamChemistry,
    /// 当前进行的赛事名（**队伍级互斥锁**）：Some = 已签约同期赛事，
    /// 排程/邀请阶段必须跳过（由 Engine 锁定/解锁）
    pub current_tournament_id: Option<String>,
}

impl Team {
    /// 构造队伍（roster 为空）。
    pub fn new(id: TeamId, name: impl Into<String>, vrs_ranking: i32, vrs_value: i32) -> Self {
        Self {
            id,
            name: name.into(),
            roster_ids: Vec::new(),
            vrs_ranking,
            vrs_value,
            budget: 0,
            chemistry: TeamChemistry::default(),
            current_tournament_id: None,
        }
    }

    /// 队伍阵容签名（= Kotlin `Signatures.teamSignature`：`队名|roster 选手名排序 join(",")`，
    /// **无空格**——与 VRS 数据层的键严格一致）。
    ///
    /// 注意：签名依赖选手**名字**（须经 World 查名）；本方法接收排序后的名字列表，
    /// 由 [`crate::world::World::signature_of`] 组装调用。
    pub fn signature(&self, player_names: &[String]) -> String {
        csc_util::signatures::team_signature(&self.name, player_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_team_defaults() {
        let t = Team::new(TeamId(1), "Vitality", 3, 1500);
        assert_eq!(t.id, TeamId(1));
        assert!(t.roster_ids.is_empty());
        assert_eq!(t.budget, 0);
        assert_eq!(t.chemistry.cohesion, 50.0);
        assert_eq!(t.current_tournament_id, None);
    }

    #[test]
    fn signature_format_matches_kotlin() {
        // Kotlin Signatures.teamSignature：队名 + "|" + 排序 join（无空格）
        let t = Team::new(TeamId(1), "Vitality", 3, 1500);
        assert_eq!(
            t.signature(&["apEX".into(), "ZywOo".into()]),
            "Vitality|apEX,ZywOo"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let mut t = Team::new(TeamId(1), "Vitality", 3, 1500);
        t.roster_ids = vec![PlayerId(0), PlayerId(1)];
        t.budget = 100_000;
        t.current_tournament_id = Some("IEM Katowice".into());
        let json = serde_json::to_string(&t).unwrap();
        let back: Team = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
