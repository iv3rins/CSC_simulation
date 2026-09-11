//! 转会候选（Kotlin `TransferOffer.kt` 转写）。
//!
//! 转写差异：Kotlin 持 `Player`/`Team` 实体引用 → Rust 用 **ID + 签名**（纯值，可序列化）。

use serde::{Deserialize, Serialize};

use csc_util::id::{PlayerId, TeamId};

/// 转会候选目标（决策上下文）：队伍 + VRS 排名 + 实力提升量 + 报价薪资。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferTarget {
    /// 目标队伍 ID
    pub team_id: TeamId,
    /// 目标队伍签名（决策选项 id 用；= Kotlin `Team.signature`）
    pub team_signature: String,
    /// 目标队 VRS 排名（越小越强）
    pub ranking: i32,
    /// 玩家实力 - 目标队最弱 NPC 实力（>0 = 入队即提升；执行转会时替换的正是该 NPC）
    pub power_delta: f64,
    /// 本报价年薪（市场价 salary_of × 系数：全价 1.0 / 降薪 SALARY_CUT_FACTOR=0.6）。
    /// 同一 offer 的所有候选薪资一致（玩家市场价，非按队报价）。
    /// `#[serde(default)]`：旧存档 TransferTarget 无此字段 → 反序列化为 0，兼容不破坏存档往返。
    #[serde(default)]
    pub salary: i64,
}

/// 一名玩家的转会候选：当前队伍 + 可转会目标清单（按排名升序，最优在前）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferOffer {
    /// 决策点 id（确定性生成：`日期|transfer|玩家ID`）
    pub point_id: String,
    /// 玩家 ID
    pub player_id: PlayerId,
    /// 玩家昵称（展示）
    pub player_name: String,
    /// 当前队伍 ID（None = 自由身——Kotlin FreeAgentTeam 单例的 Rust 替代）
    pub from_team_id: Option<TeamId>,
    /// 当前队伍签名（None = 自由身）
    pub from_team_signature: Option<String>,
    /// 可转会目标清单（按排名升序）
    pub candidates: Vec<TransferTarget>,
    /// **降薪预期**（2026 合同状态机）：全价无队可签时按 [`crate::offer::SALARY_CUT_FACTOR`]
    /// 降薪重新进入市场——本次候选按降薪后薪资生成，签约/续约按降薪结算。
    #[serde(default)]
    pub discounted: bool,
    /// 当前队 VRS 排名（None = 自由身）——auto 决策据此判断候选是否更强，
    /// 避免"只要有候选就跳槽最强队"的无脑转会（2026 试玩修复）。
    #[serde(default)]
    pub current_ranking: Option<i32>,
}

/// 降薪预期系数（合同状态机：全价无候选 → 薪资预期 ×0.6 即降薪 40% 重入市场）。
pub const SALARY_CUT_FACTOR: f64 = 0.6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let o = TransferOffer {
            point_id: "2026-06-08|transfer|ZywOo".into(),
            player_id: PlayerId(0),
            player_name: "ZywOo".into(),
            from_team_id: Some(TeamId(3)),
            from_team_signature: Some("Vitality|ZywOo,apEX".into()),
            candidates: vec![TransferTarget {
                team_id: TeamId(1),
                team_signature: "FaZe|broky,karrigan".into(),
                ranking: 2,
                power_delta: 3.5,
                salary: 150_000,
            }],
            discounted: false,
            current_ranking: Some(3),
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: TransferOffer = serde_json::from_str(&json).unwrap();
        assert_eq!(o, back);
    }
}
