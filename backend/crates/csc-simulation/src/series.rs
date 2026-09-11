//! 比赛协议类型：PlayerLine / MapScore / SeriesResult（Kotlin `SeriesResult.kt` 转写）。
//!
//! 转写差异：Kotlin `SeriesResult.teamA/teamB` 持 **Team 实体引用**（对象图）；
//! Rust 为**纯值**——`team_a_id/team_b_id`（ID 引用）+ 双方签名快照（供 VRS 结算/
//! 展示，签名与 JSON 数据匹配用）。跨引擎传递零别名、可序列化。

use serde::{Deserialize, Serialize};

use csc_domain::live::{LiveDecisionFeedback, LiveMatchState, MatchOutcomeAnalysis};
use csc_domain::match_result::MatchResult;
use csc_domain::tier_profile::tier_venue;
use csc_domain::tourney_tier::TourneyTier;
use csc_util::id::TeamId;

/// 系列赛所处的赛事阶段——HLTV TOP20「淘汰赛表现」的数据地基。
///
/// 淘汰赛硬仗 Rating 与小组赛刷分 Rating 分量不同：只对 `Playoff` 阶段的图
/// 单独累计，才能在年度 TOP20 结算里兑现「大赛型选手」的权重。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SeriesStage {
    /// 小组赛 / 瑞士轮（含双败小组段）
    Group,
    /// 淘汰赛阶段但轮次未知（旧存档兼容）。
    Playoff,
    /// 八强。
    Quarterfinal,
    /// 半决赛。
    Semifinal,
    /// 决赛。
    Final,
    /// 旧存档 / 单败淘汰整场无法细分（迁移期兜底）
    #[default]
    Unknown,
}

impl SeriesStage {
    pub const fn is_playoff(self) -> bool {
        matches!(
            self,
            Self::Playoff | Self::Quarterfinal | Self::Semifinal | Self::Final
        )
    }
}

/// 单个选手在一张地图的战绩（比赛日志粒度）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerLine {
    /// 选手昵称（玩家与 NPC 统一）
    pub player_name: String,
    /// 所属队伍签名（`Team.signature`）
    pub team_sig: String,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    /// 每回合平均伤害估值（模拟无真实伤害源）
    pub adr: f64,
    /// 有贡献回合百分比估值 0~100
    pub kast: f64,
}

/// 一场（一张地图）的比分与全员战绩。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapScore {
    /// 地图序号（从 1 开始）
    pub map_number: i32,
    /// 队伍 A 的比分（MR12 十三胜制；12-12 平后 MR4 加时，加时比分形如 16-14）
    pub team_a_score: i32,
    /// 队伍 B 的比分
    pub team_b_score: i32,
    /// 该图胜方签名
    pub winner_sig: String,
    /// 双方全部选手战绩（先 A 队 5 人，后 B 队 5 人；quick 路径为空）
    pub lines: Vec<PlayerLine>,
}

/// 一场系列赛（bo1 / bo3 / bo5）的完整结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesResult {
    /// 赛事等级（决定胜率放大系数与赛制）
    pub tier: TourneyTier,
    /// 队伍 A ID
    pub team_a_id: TeamId,
    /// 队伍 B ID
    pub team_b_id: TeamId,
    /// 队伍 A 签名快照（构造时从 World 获取）
    pub team_a_sig: String,
    /// 队伍 B 签名快照
    pub team_b_sig: String,
    /// 赛制（1 / 3 / 5）
    pub best_of: i32,
    /// 逐图比分与战绩（先到 target 图者胜）
    pub maps: Vec<MapScore>,
    /// 系列赛胜者签名
    pub winner_sig: String,
    /// 系列赛败者签名（构造时计算，与 winner_sig 互补）
    pub loser_sig: String,
    /// 赛事阶段（小组赛 / 淘汰赛）；旧存档默认 Unknown
    #[serde(default)]
    pub stage: SeriesStage,
    /// 是否可回放（**玩家参与过的对局** = true；NPC 后台粗 tick = false）。
    ///
    /// 2026 架构约定「仅回放玩家参与过的比赛，其它对局仅作后台粗 tick 快速模拟
    /// （供年度 TOP20 评选）」：`conductor::run_lod_series` 按 `has_player` 分流——
    /// 玩家队精确路径置 true，纯 NPC quick 路径置 false。前端据此只在玩家对局
    /// 显示「直播回放」，避免误回放无逐图决策数据的后台对局。
    #[serde(default)]
    pub replayable: bool,
    /// 玩家 LIVE 地图决策反馈；后台 SIM 为空。
    #[serde(default)]
    pub live_feedback: Vec<LiveDecisionFeedback>,
    /// 每个关键节点后的简化可见状态。
    #[serde(default)]
    pub live_states: Vec<LiveMatchState>,
    /// 玩家视角的 Why Won / Why Lost；旧存档为空。
    #[serde(default)]
    pub outcome_analysis: Option<MatchOutcomeAnalysis>,
}

impl SeriesResult {
    /// 系列赛折算为一场 [`MatchResult`]（供 VRS 结算；venue 按 tier 推导）。
    pub fn to_match_result(&self) -> MatchResult {
        MatchResult {
            winner_sig: self.winner_sig.clone(),
            loser_sig: self.loser_sig.clone(),
            tier: self.tier,
            venue: tier_venue(self.tier),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SeriesResult {
        SeriesResult {
            tier: TourneyTier::T1,
            team_a_id: TeamId(0),
            team_b_id: TeamId(1),
            team_a_sig: "A | p1,p2".into(),
            team_b_sig: "B | p3,p4".into(),
            best_of: 3,
            maps: vec![MapScore {
                map_number: 1,
                team_a_score: 13,
                team_b_score: 9,
                winner_sig: "A | p1,p2".into(),
                lines: Vec::new(),
            }],
            winner_sig: "A | p1,p2".into(),
            loser_sig: "B | p3,p4".into(),
            stage: SeriesStage::Unknown,
            replayable: false,
            live_feedback: Vec::new(),
            live_states: Vec::new(),
            outcome_analysis: None,
        }
    }

    #[test]
    fn loser_sig_is_complement() {
        let s = sample();
        assert_ne!(s.winner_sig, s.loser_sig);
    }

    #[test]
    fn to_match_result_derives_venue() {
        let s = sample();
        let m = s.to_match_result();
        assert_eq!(m.winner_sig, s.winner_sig);
        assert_eq!(m.loser_sig, s.loser_sig);
        assert_eq!(m.tier, TourneyTier::T1);
        assert_eq!(
            m.venue,
            csc_domain::match_result::MatchVenue::Lan,
            "T1 默认线下"
        );
    }

    #[test]
    fn serde_roundtrip() {
        let s = sample();
        let json = serde_json::to_string(&s).unwrap();
        let back: SeriesResult = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
