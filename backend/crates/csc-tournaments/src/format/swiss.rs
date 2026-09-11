//! 瑞士轮阶段（Kotlin `format/SwissStage.kt` 转写）——小组赛赛制 A。

use std::collections::{HashMap, HashSet};

use super::bracket::{MatchRunner, TeamEntry};

/// 瑞士轮阶段：固定轮数，按战绩分组配对、避免重复对阵、轮空计一胜，
/// 最终按 (胜场↓, 净胜图↓, 种子↑) 取前 qualifiers 名晋级。
#[derive(Debug, Clone, Copy)]
pub struct SwissStage {
    /// 轮数（默认 3）
    pub rounds: i32,
    /// 晋级名额（默认 8）
    pub qualifiers: usize,
    /// 每场比赛的赛制（默认 BO1）
    pub best_of: i32,
}

impl SwissStage {
    pub fn new(rounds: i32, qualifiers: usize, best_of: i32) -> Self {
        Self {
            rounds,
            qualifiers,
            best_of,
        }
    }

    /// 瑞士轮产出：晋级队伍（按名次升序）+ 全部系列赛。
    pub fn run(&self, teams: &[TeamEntry], match_runner: &mut MatchRunner) -> SwissResult {
        assert!(!teams.is_empty(), "瑞士轮需要至少一支队伍");
        assert!(
            self.rounds > 0 && self.qualifiers > 0,
            "轮数与晋级数必须为正"
        );

        let mut records: HashMap<String, (i32, i32)> = HashMap::new(); // signature → (wins, mapDiff)
        let mut played: HashMap<String, HashSet<String>> = HashMap::new(); // signature → 已对阵签名集
        let mut series_all = Vec::new();

        // 瑞士轮不淘汰：全体队伍每轮都参赛（按战绩分组配对）
        for _ in 0..self.rounds {
            // 每轮：按 (胜场↓, 种子↑) 排序后从剩余池逐一配对
            let mut remaining: Vec<TeamEntry> = {
                let mut v = teams.to_vec();
                v.sort_by(|x, y| {
                    let wx = wins(&records, x);
                    let wy = wins(&records, y);
                    wy.cmp(&wx).then_with(|| {
                        teams
                            .iter()
                            .position(|t| t.id == x.id)
                            .cmp(&teams.iter().position(|t| t.id == y.id))
                    })
                });
                v
            };
            while !remaining.is_empty() {
                let a = remaining.remove(0);
                if remaining.is_empty() {
                    // 奇数轮空：计一胜（无比赛、无 mapDiff）；下轮照常参赛
                    let w = wins(&records, &a) + 1;
                    records.insert(a.signature.clone(), (w, diff_of(&records, &a)));
                    continue;
                }
                let played_a = played.entry(a.signature.clone()).or_default();
                let b_idx = remaining
                    .iter()
                    .position(|t| !played_a.contains(&t.signature));
                let b = match b_idx {
                    Some(idx) => remaining.remove(idx),
                    None => remaining.remove(0), // 全对阵过 → 允许重复兜底
                };
                played
                    .entry(a.signature.clone())
                    .or_default()
                    .insert(b.signature.clone());
                played
                    .entry(b.signature.clone())
                    .or_default()
                    .insert(a.signature.clone());

                let series = match_runner(
                    a.id,
                    b.id,
                    self.best_of,
                    csc_simulation::series::SeriesStage::Group,
                );
                series_all.push(series.clone());
                let a_won = series.winner_sig == a.signature;
                let (winner, loser) = if a_won { (&a, &b) } else { (&b, &a) };
                // 净胜图（A 视角图分差；胜者取正、败者取负）
                let diff: i32 = series
                    .maps
                    .iter()
                    .map(|m| m.team_a_score - m.team_b_score)
                    .sum();
                let winner_diff = if a_won { diff } else { -diff };
                let (ww, wd) = records.get(&winner.signature).copied().unwrap_or((0, 0));
                records.insert(winner.signature.clone(), (ww + 1, wd + winner_diff));
                let (lw, ld) = records.get(&loser.signature).copied().unwrap_or((0, 0));
                records.insert(loser.signature.clone(), (lw, ld - winner_diff));
            }
        }

        // 排名：(胜场↓, 净胜图↓, 种子↑)，取前 qualifiers
        let mut ranked = teams.to_vec();
        ranked.sort_by(|x, y| {
            let wx = wins(&records, x);
            let wy = wins(&records, y);
            wy.cmp(&wx)
                .then_with(|| {
                    let dx = diff_of(&records, x);
                    let dy = diff_of(&records, y);
                    dy.cmp(&dx)
                })
                .then_with(|| {
                    teams
                        .iter()
                        .position(|t| t.id == x.id)
                        .cmp(&teams.iter().position(|t| t.id == y.id))
                })
        });
        SwissResult {
            qualified: ranked.into_iter().take(self.qualifiers).collect(),
            series: series_all,
        }
    }
}

/// 瑞士轮产出。
#[derive(Debug, Clone)]
pub struct SwissResult {
    pub qualified: Vec<TeamEntry>,
    pub series: Vec<csc_simulation::series::SeriesResult>,
}

/// 已获胜场数（0 起）。
fn wins(records: &HashMap<String, (i32, i32)>, team: &TeamEntry) -> i32 {
    records.get(&team.signature).map(|r| r.0).unwrap_or(0)
}

/// 已累计净胜图（0 起）。
fn diff_of(records: &HashMap<String, (i32, i32)>, team: &TeamEntry) -> i32 {
    records.get(&team.signature).map(|r| r.1).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tests_common::runner_a_wins;
    use csc_util::id::TeamId;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn swiss_3_rounds_4_teams() {
        let teams: Vec<TeamEntry> = (0..4)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let stage = SwissStage::new(3, 2, 1);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = stage.run(&teams, &mut r);
        // 每轮 2 场 × 3 轮 = 6 场
        assert_eq!(result.series.len(), 6);
        // 种子 1 全胜 → 晋级且第 1
        assert_eq!(result.qualified.len(), 2);
        assert_eq!(result.qualified[0].id, TeamId(0));
    }

    #[test]
    fn odd_team_gets_bye_win() {
        let teams: Vec<TeamEntry> = (0..3)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let stage = SwissStage::new(1, 3, 1);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = stage.run(&teams, &mut r);
        // 1 轮：1 场 + 1 轮空
        assert_eq!(result.series.len(), 1);
        assert_eq!(result.qualified.len(), 3);
    }
}
