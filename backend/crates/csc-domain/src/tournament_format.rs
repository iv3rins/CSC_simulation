//! 赛制（Tournament Format）—— 赛事对抗结构的高层配置（Kotlin `TournamentFormat.kt`）。

use serde::{Deserialize, Serialize};

/// 赛制 —— 赛事对抗结构的高层配置。
///
/// 每种赛制 = 一个或多个**阶段引擎**的组合管线：
/// - [`TournamentFormat::SingleElim`]：随机配对单败淘汰（历史默认，兼容旧行为）；
/// - [`TournamentFormat::SwissPlayoff`]：小组赛瑞士轮 → 单败淘汰，决赛 BO5（赛制 A）；
/// - [`TournamentFormat::DoubleElimGroups`]：小组赛 BO3 双败 → 单败淘汰，决赛 BO5（赛制 B）。
///
/// 设计差异：Kotlin 为 `sealed interface`（`data object` + `data class`）；
/// Rust 用 **tagged enum**——单元变体对应 `SINGLE_ELIM`，带 payload 变体对应
/// `SWISS_PLAYOFF`/`DOUBLE_ELIM_GROUPS`（参数随实例携带，语义完全一致）。
///
/// 赛制与「赛事等级」（[`crate::tourney_tier::TourneyTier`]）正交：
/// 同一等级赛事可配置不同赛制；同一赛制可用于不同等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TournamentFormat {
    /// 随机单败淘汰（旧默认；无小组赛，直接单败决冠军）
    SingleElim,

    /// 赛制 A：小组赛瑞士轮（BO1，按战绩配对、避免重复对阵）
    /// → 前 `qualifiers` 名晋级单败淘汰（`playoff_best_of`）→ 决赛 `final_best_of`。
    SwissPlayoff {
        /// 瑞士轮轮数（默认 3）
        rounds: i32,
        /// 瑞士轮晋级名额（默认 8，进淘汰赛）
        qualifiers: i32,
        /// 瑞士轮单场赛制（默认 BO1）
        swiss_best_of: i32,
        /// 淘汰赛常规轮赛制（默认 BO3）
        playoff_best_of: i32,
        /// 决赛赛制（默认 BO5）
        final_best_of: i32,
    },

    /// 赛制 B：双败小组赛（组内 `group_best_of`）→ 单败淘汰，决赛 `final_best_of`。
    /// 每组胜者组冠军（第 1）与败者组冠军（第 2）出线；淘汰赛为
    /// 胜者组冠军 vs 败者组冠军错位交叉（避免同组首轮重逢）。
    DoubleElimGroups {
        /// 小组数（默认 4 组 × 4 队 = 16 队）
        groups: i32,
        /// 小组赛赛制（默认 BO3）
        group_best_of: i32,
        /// 淘汰赛常规轮赛制（默认 BO3）
        playoff_best_of: i32,
        /// 决赛赛制（默认 BO5）
        final_best_of: i32,
    },
}

impl TournamentFormat {
    /// 赛制 A 默认参数（Kotlin `SWISS_PLAYOFF()` 默认值：rounds=3、qualifiers=8、
    /// swiss=BO1、playoff=BO3、final=BO5）。
    pub fn swiss_playoff() -> Self {
        Self::SwissPlayoff {
            rounds: 3,
            qualifiers: 8,
            swiss_best_of: 1,
            playoff_best_of: 3,
            final_best_of: 5,
        }
    }

    /// 赛制 B 默认参数（Kotlin `DOUBLE_ELIM_GROUPS()` 默认值：groups=4、group=BO3、
    /// playoff=BO3、final=BO5）。
    pub fn double_elim_groups() -> Self {
        Self::DoubleElimGroups {
            groups: 4,
            group_best_of: 3,
            playoff_best_of: 3,
            final_best_of: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swiss_playoff_defaults_match_kotlin() {
        let f = TournamentFormat::swiss_playoff();
        match f {
            TournamentFormat::SwissPlayoff {
                rounds,
                qualifiers,
                swiss_best_of,
                playoff_best_of,
                final_best_of,
            } => {
                assert_eq!(rounds, 3);
                assert_eq!(qualifiers, 8);
                assert_eq!(swiss_best_of, 1);
                assert_eq!(playoff_best_of, 3);
                assert_eq!(final_best_of, 5);
            }
            _ => panic!("应为 SwissPlayoff"),
        }
    }

    #[test]
    fn double_elim_defaults_match_kotlin() {
        let f = TournamentFormat::double_elim_groups();
        match f {
            TournamentFormat::DoubleElimGroups {
                groups,
                group_best_of,
                playoff_best_of,
                final_best_of,
            } => {
                assert_eq!(groups, 4);
                assert_eq!(group_best_of, 3);
                assert_eq!(playoff_best_of, 3);
                assert_eq!(final_best_of, 5);
            }
            _ => panic!("应为 DoubleElimGroups"),
        }
    }

    #[test]
    fn serde_names_match_kotlin() {
        let json = serde_json::to_string(&TournamentFormat::swiss_playoff()).unwrap();
        // externally-tagged：变体名与 Kotlin 常量一致
        assert!(json.starts_with(r#"{"SWISS_PLAYOFF""#), "got: {json}");
        let back: TournamentFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TournamentFormat::swiss_playoff());
    }

    #[test]
    fn parameterized_usage() {
        // 同一赛制因规模不同而参数化（如 T3 双败 8 队 = groups=2、BO1）
        let f = TournamentFormat::DoubleElimGroups {
            groups: 2,
            group_best_of: 1,
            playoff_best_of: 1,
            final_best_of: 3,
        };
        match f {
            TournamentFormat::DoubleElimGroups { final_best_of, .. } => {
                assert_eq!(final_best_of, 3)
            }
            _ => panic!("应为 DoubleElimGroups"),
        }
    }
}
