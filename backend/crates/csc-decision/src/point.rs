//! 决策点（Kotlin `DecisionPoint.kt` 转写）——事件驱动决策流的核心值对象。

use serde::{Deserialize, Serialize};

use csc_entities::injury::Injury;
use csc_entities::role::Role;
use csc_simulation::directives::{BlunderKind, MatchDirectives};
use csc_simulation::training::TrainingFocus;
use csc_util::id::{PlayerId, TeamId};

use crate::offer::TransferOffer;

/// 决策点——引擎推进到"需要外部输入"的时刻产出（纯值、可序列化）。
///
/// 两级决策流共享同一类型体系（id 确定性生成，保证重放一致）：
/// - **世界级批次**：月度收集（训练/转会窗/伤病/代言）；
/// - **比赛级现场**：图间/赛后现场（场内干预/队友失误反应）。
///
/// ID 全覆盖：每个决策点携带决策主体 `player_id`（稳定 ID，应用端按 ID 反解）；
/// `player_name` 降级为展示字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DecisionPoint {
    /// 转会窗：候选清单（决策选项 = 目标队签名 / "STAY" 不转会）
    TransferWindow {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        offers: Vec<TransferOffer>,
    },
    /// 场外养成：本月训练重点
    TrainingFocus {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        options: Vec<TrainingOption>,
    },
    /// 场内干预：图间暂停（仅玩家队伍在场时产出）
    MatchIntervention {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        event_name: String,
        map_number: i32,
        series_score: String,
        options: Vec<InterventionOption>,
    },
    /// 队友失误瞬间：玩家的反应塑造队内关系
    TeammateBlunder {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        event_name: String,
        map_number: i32,
        teammate_name: String,
        teammate_id: PlayerId,
        teammate_role: Role,
        blunder: BlunderKind,
        detail: String,
    },
    /// 伤病决策：带伤上阵 / 休养
    InjuryDecision {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        injury: Injury,
        options: Vec<InjuryOption>,
    },
    /// 代言邀约（声誉门槛触发）
    SponsorshipOffer {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        brand: String,
        annual_value: i64,
        years: i32,
        options: Vec<SponsorOption>,
    },
    /// 场外人生事件（2026 指挥中心个人决策）：
    /// 私下战队接触 / 假赛联系者 / 宫斗队友 / 战队正式接触 / 媒体采访 / 队友邀请
    LifeEvent {
        id: String,
        date: String,
        player_id: PlayerId,
        player_name: String,
        kind: LifeEventKind,
        actor: String,
        detail: String,
        options: Vec<LifeOption>,
        /// 接触方队伍稳定 ID（TeamContact/SecretTeamContact 用；P2-4 TeamId 化——
        /// `serde(default)` 前向兼容旧存档，旧档读入补 None）
        #[serde(default)]
        contact_team_id: Option<TeamId>,
    },
}

impl DecisionPoint {
    /// 决策点唯一 id。
    pub fn id(&self) -> &str {
        match self {
            Self::TransferWindow { id, .. }
            | Self::TrainingFocus { id, .. }
            | Self::MatchIntervention { id, .. }
            | Self::TeammateBlunder { id, .. }
            | Self::InjuryDecision { id, .. }
            | Self::SponsorshipOffer { id, .. }
            | Self::LifeEvent { id, .. } => id,
        }
    }

    /// 模拟日期（dateLabel）。
    pub fn date(&self) -> &str {
        match self {
            Self::TransferWindow { date, .. }
            | Self::TrainingFocus { date, .. }
            | Self::MatchIntervention { date, .. }
            | Self::TeammateBlunder { date, .. }
            | Self::InjuryDecision { date, .. }
            | Self::SponsorshipOffer { date, .. }
            | Self::LifeEvent { date, .. } => date,
        }
    }

    /// 决策主体（当前单主角 = 玩家昵称；展示用）。
    pub fn player_name(&self) -> &str {
        match self {
            Self::TransferWindow { player_name, .. }
            | Self::TrainingFocus { player_name, .. }
            | Self::MatchIntervention { player_name, .. }
            | Self::TeammateBlunder { player_name, .. }
            | Self::InjuryDecision { player_name, .. }
            | Self::SponsorshipOffer { player_name, .. }
            | Self::LifeEvent { player_name, .. } => player_name,
        }
    }

    /// 决策主体稳定 ID（应用端反解依据；名字降级为展示字段）。
    pub fn player_id(&self) -> PlayerId {
        match self {
            Self::TransferWindow { player_id, .. }
            | Self::TrainingFocus { player_id, .. }
            | Self::MatchIntervention { player_id, .. }
            | Self::TeammateBlunder { player_id, .. }
            | Self::InjuryDecision { player_id, .. }
            | Self::SponsorshipOffer { player_id, .. }
            | Self::LifeEvent { player_id, .. } => *player_id,
        }
    }
}

/// 一个决策（决策点 id → 选项 id），进决策日志（可序列化、可复现）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerDecision {
    pub point_id: String,
    pub option_id: String,
}

impl PlayerDecision {
    pub fn new(point_id: impl Into<String>, option_id: impl Into<String>) -> Self {
        Self {
            point_id: point_id.into(),
            option_id: option_id.into(),
        }
    }
}

/// 训练选项。
/// 场外人生事件类型（指挥中心个人决策）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LifeEventKind {
    /// 私下战队接触（提前挖人，非正式报价）
    SecretTeamContact,
    /// 假赛联系者试探
    MatchFixerContact,
    /// 队友宫斗（站队/调停/旁观）
    TeammatePowerStruggle,
    /// 战队正式接触（转会市场情报延伸）
    TeamContact,
    /// 媒体采访
    Interview,
    /// 队友邀请（双排/直播/团建）
    TeammateInvite,
}

impl LifeEventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::SecretTeamContact => "SECRET_TEAM_CONTACT",
            Self::MatchFixerContact => "MATCH_FIXER_CONTACT",
            Self::TeammatePowerStruggle => "TEAMMATE_POWER_STRUGGLE",
            Self::TeamContact => "TEAM_CONTACT",
            Self::Interview => "INTERVIEW",
            Self::TeammateInvite => "TEAMMATE_INVITE",
        }
    }
}

/// 场外人生事件的一个选项（选项语义由应用层按事件类型解释）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LifeOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingOption {
    pub focus: TrainingFocus,
    pub label: String,
    pub description: String,
}

/// 干预选项：携带 directive（应用给玩家队伍的当图指令）。
/// 选项顺序约定：**第 0 项 = 玩家当前打法风格的默认选项**（由生涯印记派生）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterventionOption {
    /// 决策选项 id（DEFAULT / AGGRESSIVE / CONSERVATIVE / BALANCED / TIMEOUT）
    pub id: String,
    pub directive: MatchDirectives,
    pub label: String,
    pub description: String,
}

/// 伤病选项（id 见 `csc-systems::injury`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InjuryOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// 代言选项（id = ACCEPT / DECLINE）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SponsorOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_accessors() {
        let p = DecisionPoint::TrainingFocus {
            id: "2026-06-08|train|ZywOo".into(),
            date: "2026-06-08".into(),
            player_id: PlayerId(3),
            player_name: "ZywOo".into(),
            options: Vec::new(),
        };
        assert_eq!(p.id(), "2026-06-08|train|ZywOo");
        assert_eq!(p.date(), "2026-06-08");
        assert_eq!(p.player_name(), "ZywOo");
        assert_eq!(p.player_id(), PlayerId(3));
    }

    #[test]
    fn serde_roundtrip() {
        let p = DecisionPoint::SponsorshipOffer {
            id: "id".into(),
            date: "2026-06-08".into(),
            player_id: PlayerId(3),
            player_name: "ZywOo".into(),
            brand: "Nike".into(),
            annual_value: 800_000,
            years: 2,
            options: vec![SponsorOption {
                id: "ACCEPT".into(),
                label: "接受".into(),
                description: "签约".into(),
            }],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.starts_with(r#"{"SPONSORSHIP_OFFER""#));
        let back: DecisionPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn decision_serde() {
        let d = PlayerDecision::new("p1", "AIM");
        let json = serde_json::to_string(&d).unwrap();
        let back: PlayerDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
