//! 生涯档案（Kotlin `CareerArchive.kt` 转写）——单个赛季的完整记录 + 档案册。
//!
//! 转写差异（ID 全覆盖，docs/09 B3）：档案册按**稳定 ID**（`PlayerId`）键控——
//! 名字降级为展示字段（每条 `SeasonRecord` 自含 `player_name`，实体移除后档案仍可读）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use csc_domain::season_goal::SeasonGoal;
use csc_entities::mark::CareerMarkType;
use csc_util::id::PlayerId;

/// 单个赛季的完整记录（玩家成长叙事的原始素材）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonRecord {
    /// 玩家稳定 ID（档案归属）
    pub player_id: PlayerId,
    /// 玩家昵称（展示；自含于记录，实体移除后仍可读）
    pub player_name: String,
    /// 赛季年份
    pub year: i32,
    /// 赛季末所在队伍名（None = 自由身）
    pub team_name: Option<String>,
    /// 参赛赛事数
    pub events_played: i32,
    /// 参赛图数
    pub maps_played: i32,
    pub kills: i32,
    pub deaths: i32,
    /// 赛季助攻（K-D-A 展示；2027 补齐口径）
    #[serde(default)]
    pub assists: i32,
    /// 场均 HLTV Rating（年度累计器提供）
    pub rating: f64,
    /// 系列赛胜场
    pub wins: i32,
    /// 本季荣誉（年度 TOP20 / MVP / 冠军，字符串摘要）
    pub honours: Vec<String>,
    /// 本季收入（奖金分成 + 代言）
    pub earnings: i64,
    /// 本季获得的生涯印记类型（快照）
    pub marks: Vec<CareerMarkType>,
    /// 本季伤病缺勤天数
    pub injury_days: i32,
    /// 本赛季开始时设定的主目标（v6 赛季闭环）
    #[serde(default)]
    pub season_goal: Option<SeasonGoal>,
    /// 目标是否达成（未设置目标 = None）
    #[serde(default)]
    pub goal_met: Option<bool>,
    /// 目标结算说明（可解释赛季总结）
    #[serde(default)]
    pub goal_outcome: String,
}

/// 生涯汇总（表现层"生涯总览"页）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeasonTotals {
    pub seasons: i32,
    pub total_kills: i64,
    pub total_wins: i32,
    pub total_earnings: i64,
    pub peak_rating: f64,
    pub honours: Vec<String>,
    pub marks: Vec<CareerMarkType>,
}

/// 生涯档案册：PlayerId → 逐赛季记录（纯值存储，随 GameState 存档）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CareerArchive {
    by_player: HashMap<PlayerId, Vec<SeasonRecord>>,
}

impl CareerArchive {
    /// 追加一个赛季记录（按年份升序保持——调用方按年追加）。
    pub fn record_season(&mut self, player_id: PlayerId, record: SeasonRecord) {
        self.by_player.entry(player_id).or_default().push(record);
    }

    /// 某玩家的全部赛季记录（年份升序）。
    pub fn seasons_of(&self, player_id: PlayerId) -> Vec<SeasonRecord> {
        self.by_player.get(&player_id).cloned().unwrap_or_default()
    }

    /// 某玩家的生涯汇总（无记录返回 None；= Kotlin `totalsOf`）。
    pub fn totals_of(&self, player_id: PlayerId) -> Option<SeasonTotals> {
        let seasons = self.seasons_of(player_id);
        if seasons.is_empty() {
            return None;
        }
        let mut marks: Vec<CareerMarkType> = Vec::new();
        let mut honours: Vec<String> = Vec::new();
        let mut total_kills = 0i64;
        let mut total_wins = 0i32;
        let mut total_earnings = 0i64;
        let mut peak_rating = f64::NEG_INFINITY;
        for s in &seasons {
            total_kills += s.kills as i64;
            total_wins += s.wins;
            total_earnings += s.earnings;
            peak_rating = peak_rating.max(s.rating);
            honours.extend(s.honours.iter().cloned());
            for m in &s.marks {
                if !marks.contains(m) {
                    marks.push(*m);
                }
            }
        }
        Some(SeasonTotals {
            seasons: seasons.len() as i32,
            total_kills,
            total_wins,
            total_earnings,
            peak_rating,
            honours,
            marks,
        })
    }

    /// 有档案的全部玩家 ID（排序）。
    pub fn all_players(&self) -> Vec<PlayerId> {
        let mut v: Vec<PlayerId> = self.by_player.keys().cloned().collect();
        v.sort();
        v
    }

    /// 一次性克隆整个档案映射（存档快照用；= `seasons_of` 逐选手克隆 + 重组
    /// 的等价物——单次 HashMap clone 路径更短，结构拆分收敛）。
    pub fn snapshot_map(&self) -> HashMap<PlayerId, Vec<SeasonRecord>> {
        self.by_player.clone()
    }

    /// 恢复（读档用）。
    pub fn restore(&mut self, records: HashMap<PlayerId, Vec<SeasonRecord>>) {
        self.by_player = records;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(year: i32, rating: f64, kills: i32, earnings: i64) -> SeasonRecord {
        SeasonRecord {
            player_id: PlayerId(1),
            player_name: "ZywOo".into(),
            year,
            team_name: Some("Vitality".into()),
            events_played: 6,
            maps_played: 30,
            kills,
            deaths: 20,
            assists: 15,
            rating,
            wins: 8,
            honours: Vec::new(),
            earnings,
            marks: Vec::new(),
            injury_days: 0,
            season_goal: None,
            goal_met: None,
            goal_outcome: String::new(),
        }
    }

    #[test]
    fn record_and_query() {
        let mut a = CareerArchive::default();
        a.record_season(PlayerId(1), record(2026, 1.25, 100, 100_000));
        a.record_season(PlayerId(1), record(2027, 1.30, 120, 150_000));
        assert_eq!(a.seasons_of(PlayerId(1)).len(), 2);
        assert_eq!(a.all_players(), vec![PlayerId(1)]);
        assert!(a.seasons_of(PlayerId(2)).is_empty());
        // 记录自含名字（实体移除后档案仍可读）
        assert_eq!(a.seasons_of(PlayerId(1))[0].player_name, "ZywOo");
    }

    #[test]
    fn totals_aggregate() {
        let mut a = CareerArchive::default();
        a.record_season(PlayerId(1), record(2026, 1.25, 100, 100_000));
        a.record_season(PlayerId(1), record(2027, 1.30, 120, 150_000));
        let t = a.totals_of(PlayerId(1)).unwrap();
        assert_eq!(t.seasons, 2);
        assert_eq!(t.total_kills, 220);
        assert_eq!(t.total_earnings, 250_000);
        assert!((t.peak_rating - 1.30).abs() < 1e-9);
        assert!(a.totals_of(PlayerId(2)).is_none());
    }

    #[test]
    fn serde_roundtrip() {
        let mut a = CareerArchive::default();
        a.record_season(PlayerId(1), record(2026, 1.25, 100, 100_000));
        let json = serde_json::to_string(&a).unwrap();
        let back: CareerArchive = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
