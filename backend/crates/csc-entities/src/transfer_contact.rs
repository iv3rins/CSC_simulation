//! 转会市场「主动接触」值对象：队伍在转会窗之外对玩家表达招募兴趣。
//!
//! 与转会决策解耦：接触**不是**决策点，只写入 `CareerInfo.transfer_contacts`
//! 作为市场情报；真正签约仍在合同到期后的 `TransferWindow` 决策中完成。

use serde::{Deserialize, Serialize};

use csc_domain::team_tier::TeamTier;
use csc_util::id::TeamId;

/// 一条队伍主动接触记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamContact {
    /// 接触方队伍稳定 ID
    pub team_id: TeamId,
    /// 队伍名（展示）
    pub team_name: String,
    /// 接触方 VRS 排名（越小越强）
    pub ranking: i32,
    /// 预计年薪报价（市场价口径）
    pub salary_offer: i64,
    /// 接触理由（展示文案）
    pub reason: String,
    /// 接触日期（dateLabel）
    pub date: String,
    /// 队伍级别（青训/一线等，展示）
    pub tier: TeamTier,
}
