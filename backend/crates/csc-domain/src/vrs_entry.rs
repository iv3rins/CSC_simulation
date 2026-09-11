//! 一条 VRS 全球排名记录（Kotlin `VrsEntry.kt` 转写；对应 standings 表的一行）。
//!
//! 架构归属说明：`VrsEntry` 是**纯值对象**（仅 serde 可序列化的数据，零逻辑），
//! 被 `csc-entities`（VrsMapper 映射）、`csc-simulation`（TransferRules 转会门槛）、
//! `csc-vrs`（排名引擎）三方消费。按「值对象下沉、引擎上浮」的分层原则，
//! 它归属 `csc-domain`（跨系统共享值对象，零依赖），避免底层 crate 反向依赖排名引擎。

use serde::{Deserialize, Serialize};

/// 一条 VRS 全球排名记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VrsEntry {
    /// 世界排名（1 起，越小越强）
    pub ranking: i32,
    /// VRS 积分（决定排名的分数）
    pub points: i32,
    /// 队伍名称（如 "Vitality"；JSON 字段名对齐数据源 teamName）
    #[serde(rename = "teamName")]
    pub team_name: String,
    /// 阵容名单（选手 ID 列表，通常 5 人）
    pub roster: Vec<String>,
}

impl VrsEntry {
    pub fn new(
        ranking: i32,
        points: i32,
        team_name: impl Into<String>,
        roster: Vec<String>,
    ) -> Self {
        Self {
            ranking,
            points,
            team_name: team_name.into(),
            roster,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let e = VrsEntry::new(1, 2081, "Vitality", vec!["apEX".into(), "ZywOo".into()]);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""teamName""#));
        let back: VrsEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
