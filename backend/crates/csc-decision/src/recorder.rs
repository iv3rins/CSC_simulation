//! 决策记录工具（Kotlin `DecisionRecorder.kt` 转写）——决策日志 + 事件镜像的**单点实现**。

use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;

use crate::log::DecisionLog;
use crate::point::{DecisionPoint, PlayerDecision};

/// 决策记录工具——世界级批次与比赛级现场共用同一套"记录决策"语义：
/// 决策入 [`DecisionLog`]（可复现性）+ 镜像为 [`WorldEvent::DecisionMade`]（叙事/复盘）。
pub struct DecisionRecorder;

impl DecisionRecorder {
    /// 记录一批决策：入决策日志 + journal 事件镜像（playerName/playerId 从决策点反查）。
    pub fn record(
        decision_log: &mut DecisionLog,
        journal: &mut WorldJournal,
        date: &str,
        points: &[DecisionPoint],
        decisions: &[PlayerDecision],
    ) {
        decision_log.record_all(decisions.to_vec());
        for d in decisions {
            let point = points.iter().find(|p| p.id() == d.point_id);
            let (player_name, player_id) = match point {
                Some(p) => (p.player_name().to_string(), p.player_id()),
                None => ("?".to_string(), csc_util::id::PlayerId::NONE),
            };
            journal.record(WorldEvent::DecisionMade {
                date: date.to_string(),
                seq: -1,
                player_name,
                player_id,
                point_id: d.point_id.clone(),
                option_id: d.option_id.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::TrainingOption;
    use crate::source::{AutoDecisionSource, DecisionSource};
    use csc_simulation::training::TrainingFocus;

    #[test]
    fn records_log_and_journal_mirror() {
        let mut log = DecisionLog::default();
        let mut journal = WorldJournal::default();
        let points = vec![DecisionPoint::TrainingFocus {
            id: "t1".into(),
            date: "2026-06-08".into(),
            player_id: csc_util::id::PlayerId(1),
            player_name: "ZywOo".into(),
            options: vec![TrainingOption {
                focus: TrainingFocus::Aim,
                label: "瞄准特训".into(),
                description: "x".into(),
            }],
        }];
        let decisions = AutoDecisionSource.decide(&points);
        DecisionRecorder::record(&mut log, &mut journal, "2026-06-08", &points, &decisions);
        assert_eq!(log.count(), 1);
        assert_eq!(journal.len(), 1);
        let mirror = journal.all()[0].clone();
        assert_eq!(mirror.player_name(), Some("ZywOo"));
        assert_eq!(mirror.player_id(), Some(csc_util::id::PlayerId(1)));
        assert_eq!(mirror.seq(), 0);
    }
}
