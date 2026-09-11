//! Journal 的展示投影。canonical `WorldEvent` 与投影元数据分离，保证存档和 replay 不变。

use serde::{Deserialize, Serialize};

use crate::event::{WorldEvent, WorldEventKind};
use csc_util::id::PlayerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum JournalTier {
    S,
    A,
    B,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalChannel {
    CareerFeed,
    WorldWire,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionContext {
    pub protagonist_id: Option<PlayerId>,
    /// 主角当前队伍。比赛和冠军事件只能通过稳定 TeamId 判断是否属于主角故事。
    pub protagonist_team_id: Option<csc_util::id::TeamId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JournalProjection {
    pub tier: JournalTier,
    pub channel: JournalChannel,
    /// 展示层降噪开关；canonical journal 不丢弃任何事件。
    pub visible: bool,
}

impl JournalProjection {
    pub fn visible_in(self, channel: JournalChannel) -> bool {
        self.visible && self.channel == channel
    }
}

pub fn project(event: &WorldEvent, context: ProjectionContext) -> JournalProjection {
    let player_related = context
        .protagonist_id
        .is_some_and(|id| event.player_id() == Some(id));
    let team_related = context
        .protagonist_team_id
        .is_some_and(|team_id| match event {
            WorldEvent::MatchPlayed {
                winner_id,
                loser_id,
                ..
            } => *winner_id == team_id || *loser_id == team_id,
            WorldEvent::Championship { champion_id, .. } => *champion_id == team_id,
            _ => false,
        });
    let direct = player_related || team_related;
    let elite = matches!(event, WorldEvent::MatchPlayed { tier, .. } | WorldEvent::Championship { tier, .. } if tier.is_elite());

    // Career Feed stays deliberately conservative: only the protagonist's own story
    // enters it. Unrelated high-profile results remain discoverable on World Wire.
    let channel = if direct || matches!(event.kind(), WorldEventKind::LiveUpdate) {
        JournalChannel::CareerFeed
    } else {
        JournalChannel::WorldWire
    };
    let tier = if channel == JournalChannel::CareerFeed {
        JournalTier::S
    } else if elite {
        JournalTier::A
    } else if matches!(
        event.kind(),
        WorldEventKind::TournamentStage
            | WorldEventKind::Championship
            | WorldEventKind::HonourAwarded
            | WorldEventKind::TransferDone
    ) {
        JournalTier::B
    } else {
        JournalTier::C
    };
    // Background results are retained in the canonical journal but do not become a
    // broadcast. World Wire admits top-tier tournament conclusions and player-market
    // stories; anything ambiguous defaults to suppression.
    let visible = channel == JournalChannel::CareerFeed
        || match event {
            WorldEvent::Championship { tier, .. } => tier.is_elite(),
            WorldEvent::TransferDone { .. }
            | WorldEvent::Retirement { .. }
            | WorldEvent::HonourAwarded { .. } => true,
            _ => false,
        };
    JournalProjection {
        tier,
        channel,
        visible,
    }
}

pub fn projected_json(event: &WorldEvent, context: ProjectionContext) -> Option<serde_json::Value> {
    let mut value = serde_json::to_value(event).ok()?;
    let object = value.as_object_mut()?;
    let projection = project(event, context);
    object.insert("projection".into(), serde_json::to_value(projection).ok()?);
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tourney_tier::TourneyTier;
    use csc_util::id::TeamId;

    fn match_event(tier: TourneyTier) -> WorldEvent {
        WorldEvent::MatchPlayed {
            date: "d".into(),
            seq: 1,
            event_name: "x".into(),
            tier,
            winner: "A".into(),
            winner_id: TeamId(1),
            loser: "B".into(),
            loser_id: TeamId(2),
            score: "13:9".into(),
            maps: 1,
        }
    }

    #[test]
    fn direct_event_is_s_career_feed() {
        let event = WorldEvent::DecisionMade {
            date: "d".into(),
            seq: 1,
            player_name: "p".into(),
            player_id: PlayerId(7),
            point_id: "point".into(),
            option_id: "option".into(),
        };
        let p = project(
            &event,
            ProjectionContext {
                protagonist_id: Some(PlayerId(7)),
                ..ProjectionContext::default()
            },
        );
        assert_eq!(
            p,
            JournalProjection {
                tier: JournalTier::S,
                channel: JournalChannel::CareerFeed,
                visible: true
            }
        );
    }

    #[test]
    fn unrelated_elite_event_is_a_world_wire() {
        let p = project(&match_event(TourneyTier::T1), ProjectionContext::default());
        assert_eq!(
            p,
            JournalProjection {
                tier: JournalTier::A,
                channel: JournalChannel::WorldWire,
                visible: false
            }
        );
    }

    #[test]
    fn protagonist_team_match_is_career_feed() {
        let p = project(
            &match_event(TourneyTier::T1),
            ProjectionContext {
                protagonist_team_id: Some(TeamId(1)),
                ..ProjectionContext::default()
            },
        );
        assert_eq!(p.channel, JournalChannel::CareerFeed);
        assert_eq!(p.tier, JournalTier::S);
    }

    #[test]
    fn unrelated_championship_is_b_world_wire() {
        let event = WorldEvent::Championship {
            date: "d".into(),
            seq: 1,
            event_name: "x".into(),
            tier: TourneyTier::T2,
            champion: "A".into(),
            champion_id: TeamId(1),
            mvp: None,
            mvp_id: None,
        };
        let p = project(&event, ProjectionContext::default());
        assert_eq!(
            p,
            JournalProjection {
                tier: JournalTier::B,
                channel: JournalChannel::WorldWire,
                visible: false
            }
        );
    }

    #[test]
    fn unrelated_low_tier_event_is_c_world_wire() {
        let p = project(&match_event(TourneyTier::T2), ProjectionContext::default());
        assert_eq!(
            p,
            JournalProjection {
                tier: JournalTier::C,
                channel: JournalChannel::WorldWire,
                visible: false
            }
        );
    }
}
