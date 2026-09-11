//! 世界事件（Kotlin `WorldEvent` sealed interface 的转写）。
//!
//! 转写差异（ID 全覆盖，docs/09 B3 / ARCHITECTURE-MAPPING M2 收尾）：
//! 每个事件除展示字段（名字/签名）外，一律携带稳定 ID（`PlayerId`/`TeamId`）——
//! 名字降级为展示字段，跨存档引用、谱系追踪、重名消歧都以 ID 为准。

use serde::{Deserialize, Serialize};

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::injury::{InjuryKind, InjurySeverity};
use csc_util::id::{PlayerId, TeamId};

/// 世界事件类型（= Kotlin `WorldEvent` sealed interface；全部为纯值）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorldEvent {
    /// 一场系列赛（胜者/败者签名 + 稳定 ID + 比分 + 图数）
    MatchPlayed {
        /// 事件日期（dateLabel）
        date: String,
        /// 全局递增序号（增量拉取游标；-1 占位由 journal 盖章）
        seq: i32,
        event_name: String,
        tier: TourneyTier,
        /// 胜者队名（展示）
        winner: String,
        /// 胜者稳定 ID
        winner_id: TeamId,
        /// 败者队名（展示）
        loser: String,
        /// 败者稳定 ID
        loser_id: TeamId,
        score: String,
        maps: i32,
    },
    /// 赛事冠军（含 MVP）
    Championship {
        date: String,
        seq: i32,
        event_name: String,
        tier: TourneyTier,
        /// 冠军队名（展示）
        champion: String,
        /// 冠军稳定 ID
        champion_id: TeamId,
        /// MVP 选手名（展示；None = 未评）
        mvp: Option<String>,
        /// MVP 稳定 ID
        mvp_id: Option<PlayerId>,
    },
    /// 赛事阶段推进（Major 的开幕/小组赛/淘汰赛等里程碑）
    TournamentStage {
        date: String,
        seq: i32,
        event_name: String,
        /// 阶段标签（如「小组赛」「淘汰赛」）
        stage: String,
        /// 阶段说明（如「16 队瑞士轮开赛」「8 队晋级淘汰赛」）
        detail: String,
    },
    /// 赛事取消（参赛不足等，P0-D 可观察契约：玩家可见取消而非静默消失）
    TournamentCancelled {
        date: String,
        seq: i32,
        event_name: String,
        tier: TourneyTier,
        /// 取消原因（如「参赛不足 / 未凑满 8 队」）
        reason: String,
    },
    /// 转会完成
    TransferDone {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        from_team: Option<String>,
        from_team_id: Option<TeamId>,
        to_team: String,
        to_team_id: TeamId,
        fee: i64,
    },
    /// 退役
    Retirement {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        age: i32,
        from_team: Option<String>,
        from_team_id: Option<TeamId>,
    },
    /// 新秀入世
    RookieIntake {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        age: i32,
        into_team: String,
        into_team_id: TeamId,
    },
    /// 受伤
    InjuryOccurred {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        kind: InjuryKind,
        severity: InjurySeverity,
    },
    /// 伤病痊愈
    InjuryRecovered {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        kind: InjuryKind,
    },
    /// 队内冲突（关系跌破阈值，凝聚力受损）
    Conflict {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        teammate_name: String,
        teammate_id: PlayerId,
        severity: f64,
    },
    /// 一次玩家决策（决策日志的对外镜像）
    DecisionMade {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        point_id: String,
        option_id: String,
    },
    /// 玩家主动训练计划执行完成（训练已从月度强制决策改为主动触发）
    TrainingDone {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        /// 训练重点（TrainingFocus.name()，如 "AIM" / "REST"）
        focus: String,
    },
    /// 荣誉颁发（年度 TOP20 / MVP 等）
    HonourAwarded {
        date: String,
        seq: i32,
        player_name: String,
        player_id: PlayerId,
        honour: String,
    },
    /// 按天推进的文字直播消息（2026 日级自动推进）
    LiveUpdate {
        date: String,
        seq: i32,
        headline: String,
        detail: String,
    },
}

/// 事件公共访问（= Kotlin sealed interface 的公共 val）。
impl WorldEvent {
    /// 事件日期（dateLabel）。
    pub fn date(&self) -> &str {
        match self {
            Self::MatchPlayed { date, .. }
            | Self::Championship { date, .. }
            | Self::TournamentStage { date, .. }
            | Self::TournamentCancelled { date, .. }
            | Self::TransferDone { date, .. }
            | Self::Retirement { date, .. }
            | Self::RookieIntake { date, .. }
            | Self::InjuryOccurred { date, .. }
            | Self::InjuryRecovered { date, .. }
            | Self::Conflict { date, .. }
            | Self::DecisionMade { date, .. }
            | Self::TrainingDone { date, .. }
            | Self::HonourAwarded { date, .. }
            | Self::LiveUpdate { date, .. } => date,
        }
    }

    /// 全局递增序号。
    pub fn seq(&self) -> i32 {
        match self {
            Self::MatchPlayed { seq, .. }
            | Self::Championship { seq, .. }
            | Self::TournamentStage { seq, .. }
            | Self::TournamentCancelled { seq, .. }
            | Self::TransferDone { seq, .. }
            | Self::Retirement { seq, .. }
            | Self::RookieIntake { seq, .. }
            | Self::InjuryOccurred { seq, .. }
            | Self::InjuryRecovered { seq, .. }
            | Self::Conflict { seq, .. }
            | Self::DecisionMade { seq, .. }
            | Self::TrainingDone { seq, .. }
            | Self::HonourAwarded { seq, .. }
            | Self::LiveUpdate { seq, .. } => *seq,
        }
    }

    /// 关联玩家名（None = 队伍级事件；展示用）。
    pub fn player_name(&self) -> Option<&str> {
        match self {
            Self::MatchPlayed { .. }
            | Self::Championship { .. }
            | Self::TournamentStage { .. }
            | Self::TournamentCancelled { .. }
            | Self::LiveUpdate { .. } => None,
            Self::TransferDone { player_name, .. }
            | Self::Retirement { player_name, .. }
            | Self::RookieIntake { player_name, .. }
            | Self::InjuryOccurred { player_name, .. }
            | Self::InjuryRecovered { player_name, .. }
            | Self::Conflict { player_name, .. }
            | Self::DecisionMade { player_name, .. }
            | Self::TrainingDone { player_name, .. }
            | Self::HonourAwarded { player_name, .. } => Some(player_name),
        }
    }

    /// 关联玩家稳定 ID（None = 队伍级事件）。
    pub fn player_id(&self) -> Option<PlayerId> {
        match self {
            Self::MatchPlayed { .. }
            | Self::Championship { .. }
            | Self::TournamentStage { .. }
            | Self::TournamentCancelled { .. }
            | Self::LiveUpdate { .. } => None,
            Self::TransferDone { player_id, .. }
            | Self::Retirement { player_id, .. }
            | Self::RookieIntake { player_id, .. }
            | Self::InjuryOccurred { player_id, .. }
            | Self::InjuryRecovered { player_id, .. }
            | Self::Conflict { player_id, .. }
            | Self::DecisionMade { player_id, .. }
            | Self::TrainingDone { player_id, .. }
            | Self::HonourAwarded { player_id, .. } => Some(*player_id),
        }
    }

    /// 用新序号重建（= Kotlin `copy(seq = ...)` stamp）。
    pub fn with_seq(&self, seq: i32) -> Self {
        match self {
            Self::MatchPlayed {
                date,
                seq: _,
                event_name,
                tier,
                winner,
                winner_id,
                loser,
                loser_id,
                score,
                maps,
            } => Self::MatchPlayed {
                date: date.clone(),
                seq,
                event_name: event_name.clone(),
                tier: *tier,
                winner: winner.clone(),
                winner_id: *winner_id,
                loser: loser.clone(),
                loser_id: *loser_id,
                score: score.clone(),
                maps: *maps,
            },
            Self::Championship {
                date,
                seq: _,
                event_name,
                tier,
                champion,
                champion_id,
                mvp,
                mvp_id,
            } => Self::Championship {
                date: date.clone(),
                seq,
                event_name: event_name.clone(),
                tier: *tier,
                champion: champion.clone(),
                champion_id: *champion_id,
                mvp: mvp.clone(),
                mvp_id: *mvp_id,
            },
            Self::TournamentStage {
                date,
                seq: _,
                event_name,
                stage,
                detail,
            } => Self::TournamentStage {
                date: date.clone(),
                seq,
                event_name: event_name.clone(),
                stage: stage.clone(),
                detail: detail.clone(),
            },
            Self::TournamentCancelled {
                date,
                seq: _,
                event_name,
                tier,
                reason,
            } => Self::TournamentCancelled {
                date: date.clone(),
                seq,
                event_name: event_name.clone(),
                tier: *tier,
                reason: reason.clone(),
            },
            Self::TransferDone {
                date,
                seq: _,
                player_name,
                player_id,
                from_team,
                from_team_id,
                to_team,
                to_team_id,
                fee,
            } => Self::TransferDone {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                from_team: from_team.clone(),
                from_team_id: *from_team_id,
                to_team: to_team.clone(),
                to_team_id: *to_team_id,
                fee: *fee,
            },
            Self::Retirement {
                date,
                seq: _,
                player_name,
                player_id,
                age,
                from_team,
                from_team_id,
            } => Self::Retirement {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                age: *age,
                from_team: from_team.clone(),
                from_team_id: *from_team_id,
            },
            Self::RookieIntake {
                date,
                seq: _,
                player_name,
                player_id,
                age,
                into_team,
                into_team_id,
            } => Self::RookieIntake {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                age: *age,
                into_team: into_team.clone(),
                into_team_id: *into_team_id,
            },
            Self::InjuryOccurred {
                date,
                seq: _,
                player_name,
                player_id,
                kind,
                severity,
            } => Self::InjuryOccurred {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                kind: *kind,
                severity: *severity,
            },
            Self::InjuryRecovered {
                date,
                seq: _,
                player_name,
                player_id,
                kind,
            } => Self::InjuryRecovered {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                kind: *kind,
            },
            Self::Conflict {
                date,
                seq: _,
                player_name,
                player_id,
                teammate_name,
                teammate_id,
                severity,
            } => Self::Conflict {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                teammate_name: teammate_name.clone(),
                teammate_id: *teammate_id,
                severity: *severity,
            },
            Self::DecisionMade {
                date,
                seq: _,
                player_name,
                player_id,
                point_id,
                option_id,
            } => Self::DecisionMade {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                point_id: point_id.clone(),
                option_id: option_id.clone(),
            },
            Self::TrainingDone {
                date,
                seq: _,
                player_name,
                player_id,
                focus,
            } => Self::TrainingDone {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                focus: focus.clone(),
            },
            Self::HonourAwarded {
                date,
                seq: _,
                player_name,
                player_id,
                honour,
            } => Self::HonourAwarded {
                date: date.clone(),
                seq,
                player_name: player_name.clone(),
                player_id: *player_id,
                honour: honour.clone(),
            },
            Self::LiveUpdate {
                date,
                seq: _,
                headline,
                detail,
            } => Self::LiveUpdate {
                date: date.clone(),
                seq,
                headline: headline.clone(),
                detail: detail.clone(),
            },
        }
    }
}

/// 事件类型标签（= Kotlin 变体名，供匹配/调试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldEventKind {
    MatchPlayed,
    Championship,
    TournamentStage,
    TournamentCancelled,
    TransferDone,
    Retirement,
    RookieIntake,
    InjuryOccurred,
    InjuryRecovered,
    Conflict,
    DecisionMade,
    TrainingDone,
    HonourAwarded,
    LiveUpdate,
}

impl WorldEvent {
    /// 事件类型标签。
    pub fn kind(&self) -> WorldEventKind {
        match self {
            Self::MatchPlayed { .. } => WorldEventKind::MatchPlayed,
            Self::Championship { .. } => WorldEventKind::Championship,
            Self::TournamentStage { .. } => WorldEventKind::TournamentStage,
            Self::TournamentCancelled { .. } => WorldEventKind::TournamentCancelled,
            Self::TransferDone { .. } => WorldEventKind::TransferDone,
            Self::Retirement { .. } => WorldEventKind::Retirement,
            Self::RookieIntake { .. } => WorldEventKind::RookieIntake,
            Self::InjuryOccurred { .. } => WorldEventKind::InjuryOccurred,
            Self::InjuryRecovered { .. } => WorldEventKind::InjuryRecovered,
            Self::Conflict { .. } => WorldEventKind::Conflict,
            Self::DecisionMade { .. } => WorldEventKind::DecisionMade,
            Self::TrainingDone { .. } => WorldEventKind::TrainingDone,
            Self::HonourAwarded { .. } => WorldEventKind::HonourAwarded,
            Self::LiveUpdate { .. } => WorldEventKind::LiveUpdate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_seq_stamps_all_variants() {
        let e = WorldEvent::MatchPlayed {
            date: "2026-06-08".into(),
            seq: -1,
            event_name: "T1".into(),
            tier: TourneyTier::T1,
            winner: "A|p1".into(),
            winner_id: TeamId(1),
            loser: "B|p2".into(),
            loser_id: TeamId(2),
            score: "13:9".into(),
            maps: 1,
        };
        let stamped = e.with_seq(7);
        assert_eq!(stamped.seq(), 7);
        assert_eq!(stamped.date(), "2026-06-08");
        assert_eq!(stamped.player_name(), None);
        assert_eq!(stamped.player_id(), None);
        assert_eq!(stamped.kind(), WorldEventKind::MatchPlayed);
    }

    #[test]
    fn tournament_cancelled_exposes_fields_and_kind() {
        let e = WorldEvent::TournamentCancelled {
            date: "2026-06-20".into(),
            seq: -1,
            event_name: "T3 Fallback Cup".into(),
            tier: TourneyTier::Qualify,
            reason: "参赛不足 / 未凑满 8 队".into(),
        };
        let stamped = e.with_seq(9);
        assert_eq!(stamped.seq(), 9);
        assert_eq!(stamped.date(), "2026-06-20");
        assert_eq!(stamped.player_name(), None);
        assert_eq!(stamped.player_id(), None);
        assert_eq!(stamped.kind(), WorldEventKind::TournamentCancelled);
        // serde 变体名对齐 Kotlin。
        let json = serde_json::to_string(&stamped).unwrap();
        assert!(
            json.starts_with(r#"{"TOURNAMENT_CANCELLED""#),
            "变体名应为 TOURNAMENT_CANCELLED: {json}"
        );
        let back: WorldEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(stamped, back);
    }

    #[test]
    fn player_scoped_events_expose_name_and_id() {
        let e = WorldEvent::InjuryOccurred {
            date: "2026-06-08".into(),
            seq: 0,
            player_name: "ZywOo".into(),
            player_id: PlayerId(3),
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Minor,
        };
        assert_eq!(e.player_name(), Some("ZywOo"));
        assert_eq!(e.player_id(), Some(PlayerId(3)));
    }

    #[test]
    fn serde_names_match_kotlin() {
        let e = WorldEvent::DecisionMade {
            date: "2026-06-08".into(),
            seq: 1,
            player_name: "ZywOo".into(),
            player_id: PlayerId(3),
            point_id: "p".into(),
            option_id: "AIM".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(
            json.starts_with(r#"{"DECISION_MADE""#),
            "变体名对齐 Kotlin: {json}"
        );
        let back: WorldEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
