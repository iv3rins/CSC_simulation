//! 赛制 A 管线：瑞士轮小组赛 → 单败淘汰，决赛 BO5（Kotlin `format/SwissPlayoffBracket.kt` 转写）。

use super::bracket::{BracketResult, MatchRunner, TeamEntry};
use super::single_elim::SingleElimPlayoff;
use super::swiss::SwissStage;

/// 赛制 A 管线：瑞士轮 → 单败淘汰，决赛 BO5。
#[derive(Debug, Clone, Copy)]
pub struct SwissPlayoffBracket {
    /// 瑞士轮轮数（默认 3）
    pub swiss_rounds: i32,
    /// 瑞士轮晋级名额（默认 8）
    pub swiss_qualifiers: usize,
    /// 瑞士轮单场赛制（默认 BO1）
    pub swiss_best_of: i32,
    /// 淘汰赛常规轮赛制（默认 BO3）
    pub playoff_best_of: i32,
    /// 决赛赛制（默认 BO5）
    pub final_best_of: i32,
}

impl SwissPlayoffBracket {
    pub fn new(
        swiss_rounds: i32,
        swiss_qualifiers: usize,
        swiss_best_of: i32,
        playoff_best_of: i32,
        final_best_of: i32,
    ) -> Self {
        Self {
            swiss_rounds,
            swiss_qualifiers,
            swiss_best_of,
            playoff_best_of,
            final_best_of,
        }
    }

    /// 运行完整赛制（= Kotlin `run`）。
    pub fn run(&self, teams: &[TeamEntry], match_runner: &mut MatchRunner) -> BracketResult {
        assert!(
            teams.len() >= self.swiss_qualifiers,
            "瑞士轮需要至少 {} 支参赛队（实际 {}）",
            self.swiss_qualifiers,
            teams.len()
        );

        let swiss = SwissStage::new(self.swiss_rounds, self.swiss_qualifiers, self.swiss_best_of)
            .run(teams, match_runner);
        let playoff = SingleElimPlayoff::new(self.playoff_best_of, self.final_best_of)
            .run(&swiss.qualified, match_runner);

        let mut series = swiss.series;
        series.extend(playoff.series);
        BracketResult {
            champion: playoff.champion,
            series: series.clone(),
            matches: series.iter().map(|s| s.to_match_result()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tests_common::runner_a_wins;
    use csc_util::id::TeamId;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn swiss_playoff_8_teams() {
        let teams: Vec<TeamEntry> = (0..8)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let bracket = SwissPlayoffBracket::new(3, 8, 1, 3, 5);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = bracket.run(&teams, &mut r);
        // 瑞士轮 3 轮 × 4 场 = 12 场 + 淘汰赛 7 场 = 19 场
        assert_eq!(result.series.len(), 19);
        assert_eq!(result.champion.id, TeamId(0));
        assert_eq!(result.matches.len(), 19);
    }
}
