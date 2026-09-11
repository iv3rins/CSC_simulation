//! 事件日志存储（Kotlin `WorldJournal` 的转写）——追加式、可查询的只读历史流。

use serde::{Deserialize, Serialize};

use crate::event::WorldEvent;

/// 世界事件日志——追加式、可查询的只读历史流。
///
/// 解决三类问题：表现层查询（增量拉取）/ 生涯档案素材 / 可复现性（随存档重放）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldJournal {
    events: Vec<WorldEvent>,
    next_seq: i32,
}

impl WorldJournal {
    /// 记录一条事件（seq 自动盖章；= Kotlin `record`）。
    ///
    /// **性能修订（结构拆分）**：不再返回已盖章事件——此前 `push(stamped.clone())`
    /// 后再返回 `stamped` 造成每次记录 2 次深拷贝（事件流可累积数万条）。
    /// 现在 `with_seq` 只拷贝一次入队；调用方需要已盖章值时用 [`Self::all`] 尾部。
    pub fn record(&mut self, event: WorldEvent) {
        let stamped = event.with_seq(self.next_seq);
        self.next_seq += 1;
        self.events.push(stamped);
    }

    /// 全部事件（按 seq 升序）。
    pub fn all(&self) -> Vec<WorldEvent> {
        self.events.clone()
    }

    /// 增量拉取：seq 之后的全部事件（UI/服务端同步游标）。
    pub fn since(&self, seq: i32) -> Vec<WorldEvent> {
        self.events
            .iter()
            .filter(|e| e.seq() > seq)
            .cloned()
            .collect()
    }

    /// 某玩家的全部相关事件（按名字，展示用；重名时不可靠）。
    pub fn of(&self, player_name: &str) -> Vec<WorldEvent> {
        self.events
            .iter()
            .filter(|e| e.player_name() == Some(player_name))
            .cloned()
            .collect()
    }

    /// 某玩家的全部相关事件（按稳定 ID，谱系可靠）。
    pub fn of_player(&self, player_id: csc_util::id::PlayerId) -> Vec<WorldEvent> {
        self.events
            .iter()
            .filter(|e| e.player_id() == Some(player_id))
            .cloned()
            .collect()
    }

    /// 当前游标（最大 seq）。
    pub fn cursor(&self) -> i32 {
        self.next_seq - 1
    }

    /// 事件总数。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 恢复（读档用；= Kotlin `restore`：seq 从最大 +1 续）。
    pub fn restore(&mut self, events: Vec<WorldEvent>) {
        self.next_seq = events.iter().map(|e| e.seq()).max().map_or(0, |s| s + 1);
        self.events = events;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::WorldEvent;
    use csc_domain::tourney_tier::TourneyTier;

    fn match_played() -> WorldEvent {
        WorldEvent::MatchPlayed {
            date: "2026-06-08".into(),
            seq: -1,
            event_name: "T1 #1".into(),
            tier: TourneyTier::T1,
            winner: "A|p1".into(),
            winner_id: csc_util::id::TeamId(1),
            loser: "B|p2".into(),
            loser_id: csc_util::id::TeamId(2),
            score: "13:9".into(),
            maps: 1,
        }
    }

    #[test]
    fn seq_is_auto_stamped_incrementally() {
        let mut j = WorldJournal::default();
        j.record(match_played());
        j.record(match_played());
        let all = j.all();
        assert_eq!(all[0].seq(), 0);
        assert_eq!(all[1].seq(), 1);
        assert_eq!(j.cursor(), 1);
        assert_eq!(j.len(), 2);
    }

    #[test]
    fn since_is_incremental_cursor() {
        let mut j = WorldJournal::default();
        for _ in 0..5 {
            j.record(match_played());
        }
        let after = j.since(2);
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].seq(), 3);
    }

    #[test]
    fn of_filters_by_player() {
        let mut j = WorldJournal::default();
        j.record(WorldEvent::Retirement {
            date: "d".into(),
            seq: -1,
            player_name: "old1".into(),
            player_id: csc_util::id::PlayerId(7),
            age: 35,
            from_team: Some("T".into()),
            from_team_id: Some(csc_util::id::TeamId(1)),
        });
        j.record(match_played());
        assert_eq!(j.of("old1").len(), 1);
        assert!(j.of("nobody").is_empty());
        assert_eq!(
            j.of_player(csc_util::id::PlayerId(7)).len(),
            1,
            "按稳定 ID 过滤"
        );
        assert!(j.of_player(csc_util::id::PlayerId(99)).is_empty());
    }

    #[test]
    fn restore_continues_seq() {
        let mut j = WorldJournal::default();
        j.record(match_played());
        assert_eq!(j.all()[0].seq(), 0);
        // 存档 → 恢复 → 续写
        let snapshot = j.all();
        let mut j2 = WorldJournal::default();
        j2.restore(snapshot);
        assert_eq!(j2.cursor(), 0);
        j2.record(match_played());
        assert_eq!(j2.all()[1].seq(), 1, "恢复后 seq 从最大+1 续");
    }

    #[test]
    fn serde_roundtrip() {
        let mut j = WorldJournal::default();
        j.record(match_played());
        let json = serde_json::to_string(&j).unwrap();
        let back: WorldJournal = serde_json::from_str(&json).unwrap();
        assert_eq!(j, back);
    }
}
