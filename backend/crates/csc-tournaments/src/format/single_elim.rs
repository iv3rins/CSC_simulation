//! 单败淘汰阶段（Kotlin `format/SingleElimPlayoff.kt` 转写）。

use super::bracket::{BracketSeeding, MatchRunner, TeamEntry};

/// 单败淘汰阶段：按种子对阵落位，常规轮 best_of、决赛轮 final_best_of；
/// 非 2 的幂队伍数补轮空。
#[derive(Debug, Clone, Copy)]
pub struct SingleElimPlayoff {
    /// 常规轮次赛制（默认 BO3）
    pub best_of: i32,
    /// 决赛赛制（默认 BO5）
    pub final_best_of: i32,
}

impl SingleElimPlayoff {
    pub fn new(best_of: i32, final_best_of: i32) -> Self {
        Self {
            best_of,
            final_best_of,
        }
    }

    /// 单败淘汰产出：冠军 + 全部系列赛。
    pub fn run(&self, teams: &[TeamEntry], match_runner: &mut MatchRunner) -> SingleElimResult {
        assert!(!teams.is_empty(), "淘汰赛需要至少一支队伍");

        let mut series_all: Vec<csc_simulation::series::SeriesResult> = Vec::new();
        let mut field = BracketSeeding::bracket_order(teams);

        // 轮次：队伍数每轮减半；决赛轮（field.size == 2）用 final_best_of。
        // 奇偶轮空：field.len() 为奇数时，最强种子（field[0]，种子 1）直接晋级
        // 不结算 series，其余两两配对——总场数恒 = N-1（淘汰赛守恒）。
        while field.len() > 1 {
            let mut next: Vec<TeamEntry> = Vec::new();
            let is_final = field.len() == 2;
            let stage = match field.len() {
                8 => csc_simulation::series::SeriesStage::Quarterfinal,
                4 => csc_simulation::series::SeriesStage::Semifinal,
                2 => csc_simulation::series::SeriesStage::Final,
                _ => csc_simulation::series::SeriesStage::Playoff,
            };
            let mut i = 0;
            // 奇数队伍数：最强种子（field[0]）轮空直接晋级，不结算 series。
            if field.len() % 2 == 1 {
                next.push(field[0].clone());
                i = 1;
            }
            while i < field.len() {
                let a = field[i].clone();
                let b = field[i + 1].clone();
                let winner = if a.is_bye() {
                    b // a 轮空：b 直接晋级（不结算）
                } else if b.is_bye() {
                    a
                } else {
                    let series = match_runner(
                        a.id,
                        b.id,
                        if is_final {
                            self.final_best_of
                        } else {
                            self.best_of
                        },
                        stage,
                    );
                    series_all.push(series.clone());
                    if series.winner_sig == a.signature {
                        a
                    } else {
                        b
                    }
                };
                next.push(winner);
                i += 2;
            }
            field = next;
        }
        SingleElimResult {
            champion: field[0].clone(),
            series: series_all,
        }
    }
}

/// 单败淘汰产出。
#[derive(Debug, Clone)]
pub struct SingleElimResult {
    pub champion: TeamEntry,
    pub series: Vec<csc_simulation::series::SeriesResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::bracket::TeamEntry;
    use crate::format::tests_common::runner_a_wins;
    use csc_util::id::TeamId;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn single_elim_4_teams() {
        let teams: Vec<TeamEntry> = (0..4)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let bracket = SingleElimPlayoff::new(3, 5);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = bracket.run(&teams, &mut r);
        assert_eq!(result.series.len(), 3, "4 队单败 3 场");
        assert_eq!(result.champion.id, TeamId(0), "种子 1 全胜夺冠");
    }

    #[test]
    fn bye_advances_without_match() {
        let teams: Vec<TeamEntry> = (0..5)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let bracket = SingleElimPlayoff::new(3, 5);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = bracket.run(&teams, &mut r);
        // 5 队 → 首轮 2 场 + 轮空；总 4 场（5→3→2→1）
        assert_eq!(result.series.len(), 4);
    }
}
