//! 赛制测试共享 runner：固定 A 胜的伪造 `MatchRunner`（**唯一事实源**）。
//!
//! 四个赛制模块（double_elim / swiss / swiss_playoff / single_elim）的测试都
//! 需要「调用方传入 A 队必赢」的确定性 MatchRunner，以隔离赛制推进逻辑与
//! 单场比赛模拟逻辑。历史上各自复制过一份几乎逐字符相同的 `runner()`，
//! 提取本模块后共享同一实现（risk_diagnose R3 Jaccard=1.00）。

use csc_domain::tourney_tier::TourneyTier;
use csc_simulation::series::{MapScore, SeriesResult};
use csc_util::id::TeamId;

/// 构造固定 A 胜的 `MatchRunner`（A = 参数中的第一个队伍）。
///
/// 行为：BO 任意值都返回 13-7 单图 A 胜，stage 透传。不消耗 RNG。
pub fn runner_a_wins()
-> impl FnMut(TeamId, TeamId, i32, csc_simulation::series::SeriesStage) -> SeriesResult {
    move |a: TeamId, b: TeamId, bo: i32, stage| SeriesResult {
        tier: TourneyTier::T1,
        team_a_id: a,
        team_b_id: b,
        team_a_sig: format!("A|{a:?}"),
        team_b_sig: format!("B|{b:?}"),
        best_of: bo,
        maps: vec![MapScore {
            map_number: 1,
            team_a_score: 13,
            team_b_score: 7,
            winner_sig: format!("A|{a:?}"),
            lines: Vec::new(),
        }],
        winner_sig: format!("A|{a:?}"),
        loser_sig: format!("B|{b:?}"),
        stage,
        replayable: false,
        live_feedback: Vec::new(),
        live_states: Vec::new(),
        outcome_analysis: None,
    }
}
