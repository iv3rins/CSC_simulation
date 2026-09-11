//! LIVE 关键节点的可解释状态、决策反馈与赛后分析。

use serde::{Deserialize, Serialize};

/// 玩家在一张地图上的可见比赛状态，隐藏底层胜率数学。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LiveMatchState {
    pub form: f64,
    pub fatigue: f64,
    pub confidence: f64,
    pub team_morale: f64,
    pub chemistry: f64,
    pub map_preparation: f64,
    pub rounds_lost: i32,
    #[serde(default)]
    pub momentum: f64,
    #[serde(default)]
    pub round_stability: f64,
    #[serde(default)]
    pub economy_pressure: f64,
}

/// LIVE 决策类型。选项 ID 仍由现有 DecisionPoint 协议承载。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LiveDecisionKind {
    Tactical,
    Aggressive,
    Conservative,
    Balanced,
    Timeout,
    Encourage,
    Criticize,
    Ignore,
}

/// 一次 LIVE 决策对比赛叙事的影响。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveDecisionFeedback {
    pub decision_id: String,
    #[serde(default)]
    pub decision: String,
    #[serde(default)]
    pub immediate_effect: String,
    #[serde(default)]
    pub affected_state: Vec<String>,
    #[serde(default)]
    pub narrative: String,
    #[serde(default)]
    pub next_state: String,
    #[serde(default)]
    pub headline: String,
    #[serde(default)]
    pub what_happened: String,
    #[serde(default)]
    pub affected_metrics: Vec<String>,
    #[serde(default)]
    pub explanation: String,
}

/// 比赛结束后的可解释因素，不暴露底层胜率公式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MatchOutcomeAnalysis {
    pub player_performance: f64,
    pub team_strength: f64,
    pub form: f64,
    pub chemistry: f64,
    pub map_preparation: f64,
    pub tactical_impact: f64,
    pub opponent_performance: f64,
    pub key_moments: Vec<String>,
    pub won: bool,
}

// —— LIVE 逐回合引擎（Phase 4a）值类型 ——
//
// 注意：本 crate 是**零依赖**（仅 serde）值对象层。需要 `PlayerId`/`TeamId` 的
// 类型（如 `LiveRoundDecision::Encourage(PlayerId)`、`LivePlayerState`、
// `BombState::Carried`）放在 `csc-simulation::live_match`，本文件只保留纯 serde
// 值类型，以维持 csc-domain 的零依赖架构契约。

/// 比赛生命周期阶段（数据状态，不由前端推断）。`Completed` 后才允许生成 Replay 入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LiveMatchPhase {
    Scheduled,
    Warmup,
    Live,
    RoundEnd,
    NextRound,
    Halftime,
    MapEnd,
    NextMap,
    SeriesEnd,
    Completed,
    Postponed,
    Cancelled,
}

/// 队伍所在方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    T,
    CT,
}

impl Side {
    /// 相反方（半场换边用）。
    pub fn opposite(self) -> Self {
        match self {
            Self::T => Self::CT,
            Self::CT => Self::T,
        }
    }
}

/// 进攻节奏（影响回合概率的叙事层档位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pace {
    Default,
    Slow,
    Normal,
    Fast,
}

/// 装备 / 武器大类（回合概率与叙事的粗略输入，非物理引擎）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponClass {
    Rifle,
    Sniper,
    Smg,
    Pistol,
    Heavy,
    Knife,
}

/// 团队经济档位（`LiveEconomy.equipment`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomyBuy {
    Eco,
    HalfBuy,
    ForceBuy,
    FullBuy,
}

/// 道具/战术计划档位（`LiveRoundDecision::ChangeUtilityPlan`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UtilityPlan {
    Default,
    Heavy,
    Eco,
    Full,
}

/// 炸弹安放点位（`BombSite`，简化 A/B 双点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BombSite {
    A,
    B,
}

/// 单队经济状态（每回合后更新，供下一回合概率/叙事）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LiveEconomy {
    pub money: i32,
    pub equipment: EconomyBuy,
    pub timeout_remaining: u8,
    /// 连败奖励加成（用于下回合经济档位推断）。
    pub loss_bonus: i32,
}

/// 选手的道具背包（仅数量，不模拟轨迹）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UtilityInventory {
    pub grenades: u8,
    pub flashbangs: u8,
    pub smokes: u8,
    pub molotovs: u8,
    pub he_grenades: u8,
}
