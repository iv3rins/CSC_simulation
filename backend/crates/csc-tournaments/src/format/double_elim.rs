//! 双败赛制（Kotlin `format/DoubleElimGroup.kt` + `DoubleElimGroupsBracket.kt` 转写）。

use super::bracket::{BracketResult, BracketSeeding, MatchRunner, TeamEntry};
use super::single_elim::SingleElimPlayoff;

/// 组内双败阶段——标准 4 队双败 bracket，组内前 2 出线。
#[derive(Debug, Clone, Copy)]
pub struct DoubleElimGroup {
    /// 每场比赛赛制（默认 BO3）
    pub best_of: i32,
}

impl DoubleElimGroup {
    /// 每组队伍数（固定 4，双败 bracket 的标准规模）
    pub const GROUP_SIZE: usize = 4;

    pub fn new(best_of: i32) -> Self {
        Self { best_of }
    }

    /// 双败小组产出：组内第 1（胜者组冠军）、组内第 2（败者组冠军）+ 全部系列赛。
    pub fn run(&self, teams: &[TeamEntry], match_runner: &mut MatchRunner) -> DoubleElimResult {
        assert_eq!(
            teams.len(),
            Self::GROUP_SIZE,
            "双败小组需要恰好 {} 支队伍",
            Self::GROUP_SIZE
        );
        assert!(
            self.best_of == 1 || self.best_of == 3 || self.best_of == 5,
            "双败小组赛制仅支持 bo1/bo3/bo5"
        );

        let mut series_all = Vec::new();
        let play = |a: &TeamEntry,
                    b: &TeamEntry,
                    match_runner: &mut MatchRunner,
                    series_all: &mut Vec<_>|
         -> TeamEntry {
            let s = match_runner(
                a.id,
                b.id,
                self.best_of,
                csc_simulation::series::SeriesStage::Group,
            );
            series_all.push(s.clone());
            if s.winner_sig == a.signature {
                a.clone()
            } else {
                b.clone()
            }
        };

        // 种子 1 vs 种子 4、种子 2 vs 种子 3（组内按种子相邻配对）
        let t1 = teams[0].clone();
        let t2 = teams[1].clone();
        let t3 = teams[2].clone();
        let t4 = teams[3].clone();
        let wb_a = play(&t1, &t4, match_runner, &mut series_all); // 胜者组半决 1
        let wb_b = play(&t2, &t3, match_runner, &mut series_all); // 胜者组半决 2
        let wb_winner = play(&wb_a, &wb_b, match_runner, &mut series_all); // 胜者组决赛 → 组内第 1
        let lb_a = play(
            &loser_of(&wb_a, &t1, &t4),
            &loser_of(&wb_b, &t2, &t3),
            match_runner,
            &mut series_all,
        ); // 败者组第 1 轮
        let lb_winner = play(
            &loser_side(&wb_winner, &wb_a, &wb_b),
            &lb_a,
            match_runner,
            &mut series_all,
        ); // 败者组决赛 → 组内第 2

        DoubleElimResult {
            winner: wb_winner,
            runner_up: lb_winner,
            series: series_all,
        }
    }
}

/// 双败小组产出。
#[derive(Debug, Clone)]
pub struct DoubleElimResult {
    /// 胜者组冠军（组内第 1）
    pub winner: TeamEntry,
    /// 败者组冠军（组内第 2）
    pub runner_up: TeamEntry,
    pub series: Vec<csc_simulation::series::SeriesResult>,
}

/// 从 (胜者, 种子A, 种子B) 中取败者（= Kotlin `loserOf`，id 判定）。
fn loser_of(winner: &TeamEntry, seed_a: &TeamEntry, seed_b: &TeamEntry) -> TeamEntry {
    if winner.id == seed_a.id {
        seed_b.clone()
    } else {
        seed_a.clone()
    }
}

/// 胜者组决赛败者（wbWinner 之外的另一个胜者组半决胜者；= Kotlin `loserSide`）。
fn loser_side(wb_winner: &TeamEntry, wb_a: &TeamEntry, wb_b: &TeamEntry) -> TeamEntry {
    if wb_winner.id == wb_a.id {
        wb_b.clone()
    } else {
        wb_a.clone()
    }
}

/// 赛制 B 管线：双败小组赛 → 单败淘汰，决赛 BO5（**弹性赛制**：非 4 的倍数降级单败）。
#[derive(Debug, Clone, Copy)]
pub struct DoubleElimGroupsBracket {
    /// 小组数（默认 4）
    pub groups: usize,
    /// 每组赛制（默认 BO3）
    pub group_best_of: i32,
    /// 淘汰赛常规轮赛制（默认 BO3）
    pub playoff_best_of: i32,
    /// 决赛赛制（默认 BO5）
    pub final_best_of: i32,
}

impl DoubleElimGroupsBracket {
    pub fn new(
        groups: usize,
        group_best_of: i32,
        playoff_best_of: i32,
        final_best_of: i32,
    ) -> Self {
        Self {
            groups,
            group_best_of,
            playoff_best_of,
            final_best_of,
        }
    }

    pub fn run(&self, teams: &[TeamEntry], match_runner: &mut MatchRunner) -> BracketResult {
        assert!(!teams.is_empty(), "双败小组赛需要至少一支参赛队");
        let group_size = DoubleElimGroup::GROUP_SIZE;

        // 弹性赛制：非 4 的倍数（池不足 / 数据不全）→ 降级单败淘汰
        if !teams.len().is_multiple_of(group_size) {
            eprintln!(
                "[DoubleElimGroupsBracket] 参赛队数 {} 不是 {} 的倍数（{} 组 × {} 队满编配置）——降级单败淘汰（弹性赛制）",
                teams.len(),
                group_size,
                self.groups,
                group_size
            );
            let single = SingleElimPlayoff::new(self.playoff_best_of, self.final_best_of)
                .run(teams, match_runner);
            return BracketResult {
                champion: single.champion,
                series: single.series.clone(),
                matches: single.series.iter().map(|s| s.to_match_result()).collect(),
            };
        }
        let group_count = teams.len() / group_size; // 动态分组：满编按配置、超编/缩编按实际队数

        let mut series_all: Vec<csc_simulation::series::SeriesResult> = Vec::new();

        // 1. 蛇形分组 → 每组双败，收集各组第 1 / 第 2
        let group_results: Vec<DoubleElimResult> = BracketSeeding::snake_seed(teams, group_count)
            .iter()
            .map(|group_teams| {
                let r = DoubleElimGroup::new(self.group_best_of).run(group_teams, match_runner);
                series_all.extend(r.series.clone());
                r
            })
            .collect();
        let group_winners: Vec<TeamEntry> =
            group_results.iter().map(|r| r.winner.clone()).collect();
        let group_runners_up: Vec<TeamEntry> =
            group_results.iter().map(|r| r.runner_up.clone()).collect();

        // 2. 淘汰赛：四分之一 = 胜者组冠军 vs 败者组冠军，错位交叉配对
        //    （W1 vs R2、W2 vs R1...），避免同组冠亚军首轮重逢
        let play = |a: &TeamEntry,
                    b: &TeamEntry,
                    bo: i32,
                    match_runner: &mut MatchRunner,
                    series_all: &mut Vec<_>|
         -> TeamEntry {
            let s = match_runner(a.id, b.id, bo, csc_simulation::series::SeriesStage::Playoff);
            series_all.push(s.clone());
            if s.winner_sig == a.signature {
                a.clone()
            } else {
                b.clone()
            }
        };

        // 单组（group_count == 1）时 w1 vs r1 即总决赛（finalBestOf）
        let mut field: Vec<TeamEntry> = group_winners
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let opp = &group_runners_up[(i + 1) % group_count];
                play(
                    w,
                    opp,
                    if group_count == 1 {
                        self.final_best_of
                    } else {
                        self.playoff_best_of
                    },
                    match_runner,
                    &mut series_all,
                )
            })
            .collect();
        while field.len() > 1 {
            let is_final = field.len() == 2;
            let mut next: Vec<TeamEntry> = Vec::new();
            let mut i = 0;
            if field.len() % 2 == 1 {
                // 奇数队：最高种子（field 首位）轮空晋级（bracket 惯例）
                next.push(field[0].clone());
                i = 1;
            }
            while i < field.len() {
                let a = field[i].clone();
                let b = field[i + 1].clone();
                next.push(play(
                    &a,
                    &b,
                    if is_final {
                        self.final_best_of
                    } else {
                        self.playoff_best_of
                    },
                    match_runner,
                    &mut series_all,
                ));
                i += 2;
            }
            field = next;
        }

        BracketResult {
            champion: field[0].clone(),
            series: series_all.clone(),
            matches: series_all.iter().map(|s| s.to_match_result()).collect(),
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
    fn double_elim_group_5_matches() {
        let teams: Vec<TeamEntry> = (0..4)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let group = DoubleElimGroup::new(3);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = group.run(&teams, &mut r);
        assert_eq!(result.series.len(), 5, "4 队双败共 5 场");
        assert_eq!(result.winner.id, TeamId(0), "胜者组冠军 = 种子 1");
        assert_eq!(result.runner_up.id, TeamId(1), "败者组冠军 = 种子 2");
    }

    #[test]
    fn groups_bracket_16_teams() {
        let teams: Vec<TeamEntry> = (0..16)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let bracket = DoubleElimGroupsBracket::new(4, 3, 3, 5);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = bracket.run(&teams, &mut r);
        // 4 组 × 5 场 + 四强(4) + 半决(2) + 决赛(1) = 27 场
        assert_eq!(result.series.len(), 27);
        assert_eq!(result.champion.id, TeamId(0));
        assert_eq!(result.matches.len(), 27);
    }

    #[test]
    fn elastic_downgrade_on_non_multiple() {
        let teams: Vec<TeamEntry> = (0..6)
            .map(|i| TeamEntry {
                id: TeamId(i),
                signature: format!("A|TeamId({i})"),
            })
            .collect();
        let bracket = DoubleElimGroupsBracket::new(4, 3, 3, 5);
        let _rng = Xoshiro256StarStar::seed(42);
        let mut r = runner_a_wins();
        let result = bracket.run(&teams, &mut r);
        // 降级单败：6 队 → 5 场
        assert_eq!(result.series.len(), 5, "6 队非 4 倍数 → 降级单败");
        assert_eq!(result.champion.id, TeamId(0));
    }
}
