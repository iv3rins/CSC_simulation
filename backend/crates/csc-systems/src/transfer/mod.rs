//! 转会子系统（Kotlin `TransferEngine.kt` 转写）：玩家转会窗（职业生涯系统的一部分）。
//!
//! 因果方向（与 TransferRules 一致）：**选手变强才加入更高级别的队伍**。
//!
//! 与决策点管线对接（事件驱动决策流）：
//! - [`Self::generate_offers`]：生成候选（决策批次输入，offer 自带确定性 pointId）；
//! - [`Self::execute_choices`]：消费决策（STAY / 目标签名）执行转会——生成与执行解耦，
//!   中间由 `DecisionSource` 传递；
//! - [`Self::process_transfer_window`]：便捷门面（生成 → 决策 → 执行）。
//!
//! 转会动作 = **玩家与目标队最弱选手互换**（两方 roster 保持 5 人），并同步
//! 队伍实体与 VRS 状态（签名迁移 + 出走 ≥ 3 人积分清零）。
//!
//! **经济闭环**（培养补偿模型）：到期/自由转会时，买方支付
//! [`FinanceModel::transfer_fee`] 转会费（原队培养补偿，自由身免收）——转会市场
//! 有了摩擦端，队伍预算约束 = 薪资 + 转会费（[`FinanceModel::can_afford`]）。
//!
//! 结构拆分（2026）：本模块已按职责拆分——`market`（情报/报价生成）、
//! `execution`（决策消费/续约/离队/买断/转会执行）；本文件只保留类型、
//! 常量与共享辅助函数。

use csc_domain::team_tier::TeamTier;
use csc_entities::world::World;
use csc_util::id::{PlayerId, TeamId};
use serde::{Deserialize, Serialize};

mod execution;
mod market;

#[cfg(test)]
mod tests;

/// 转会市场洞察（结果解释层）：为什么现在有/没有报价，以及当前约束。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferInsight {
    pub player_name: String,
    pub contract_years: i32,
    pub salary: i64,
    pub reputation: i32,
    pub current_team: Option<String>,
    pub current_team_rank: Option<i32>,
    pub months_unsigned: i32,
    pub eligible_teams_full_price: usize,
    pub eligible_teams_discounted: usize,
    pub explanation: String,
}

/// 一次转会事件（信息用途，供展示/测试断言；ID 供事件日志/档案引用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferEvent {
    pub player_name: String,
    /// 转会选手稳定 ID
    pub player_id: PlayerId,
    /// null = 自由身首签
    pub from_team: Option<String>,
    /// 原队稳定 ID（None = 自由身）
    pub from_team_id: Option<TeamId>,
    pub to_team: String,
    /// 新队稳定 ID
    pub to_team_id: TeamId,
    /// 新合同年限（按能力值决定）
    pub contract_years: i32,
    /// 新年薪
    pub salary: i64,
    /// 转会费（培养补偿；自由身 = 0）
    pub fee: i64,
    /// 转会日期（模拟时钟）
    pub date: String,
}

/// 转会子系统：无跨调用状态（全部依赖参数化）。
pub struct TransferEngine;

impl TransferEngine {
    /// 决策选项 id 常量。
    pub const STAY: &'static str = "STAY";
    /// 离队赋闲选项（合同到期主动离开，进入自由市场）。
    pub const LEAVE: &'static str = "LEAVE";
}

/// 层级严格更强：`a` 的层级比 `b` 高（T1 最高、T4 最低）。
pub(crate) fn tier_strictly_higher(a: TeamTier, b: TeamTier) -> bool {
    (a as u8) < (b as u8)
}
/// 排序后的 roster 名字（签名组件；= Kotlin `Team.signature` 内部逻辑）。
pub(crate) fn sorted_names_of(world: &World, team_id: TeamId) -> Vec<String> {
    let mut names: Vec<String> = world
        .roster(team_id)
        .iter()
        .map(|p| p.name.clone())
        .collect();
    names.sort();
    names
}
