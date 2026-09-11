//! 稳定 ID（新类型）。
//!
//! 转写核心决策（对应 docs/09 B3）：Kotlin 原型用 `playerName`/`team.signature`
//! 做引用键（换人即变、谱系断裂）；Rust 侧一律用**全局递增 ID**——
//! 实体 arena 索引、事件/档案/转会引用、存档序列化都以 ID 为准，
//! `name`/`signature` 降级为展示字段。

use serde::{Deserialize, Serialize};

/// 选手稳定 ID（u32 新类型；`NONE = u32::MAX` 为"未分配"哨兵，
/// `0` 是合法 arena 索引——不要占用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerId(pub u32);

/// 队伍稳定 ID（u32 新类型；`NONE = u32::MAX` 为"未分配"哨兵，
/// `0` 是合法 arena 索引——不要占用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TeamId(pub u32);

impl PlayerId {
    /// 未分配哨兵（**u32::MAX**——不能是 0：World arena 的 id 从 0 起，
    /// 0 是合法 ID；NONE 用最大哨兵值避免冲突）。
    pub const NONE: PlayerId = PlayerId(u32::MAX);
}

impl TeamId {
    /// 未分配哨兵（同 PlayerId：0 是合法 arena 索引）。
    pub const NONE: TeamId = TeamId(u32::MAX);
}

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for TeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 稳定 ID 分配：见 `csc-entities::World`——arena 不收缩，ID 即 Vec 索引，
/// `next_player_id`/`next_team_id`（= players.len()/teams.len()）由 World 直接维护，
/// 因此本模块不提供独立分配器（历史上曾存在共享计数 `IdAllocator`，因与
/// World 的分离计数器语义冲突且无任何引用，已删除）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_sentinels() {
        // NONE = u32::MAX（0 是合法 arena 索引，不能占用）
        assert_eq!(PlayerId::NONE.0, u32::MAX);
        assert_eq!(TeamId::NONE.0, u32::MAX);
        assert_ne!(PlayerId::NONE, PlayerId(0));
    }
}
