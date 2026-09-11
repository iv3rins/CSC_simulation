//! 赛制引擎公共类型：TeamEntry / MatchRunner / BracketResult / 种子工具
//! （Kotlin `format/Bracket.kt` 转写）。

use csc_domain::match_result::MatchResult;
use csc_simulation::series::{SeriesResult, SeriesStage};
use csc_util::id::TeamId;

/// 参赛队伍标识（= Kotlin `Team` 实体的标识侧：id + 签名）。
///
/// 转写差异：Kotlin 赛制引擎持有 `Team` 实体（标识+数据一体）；
/// Rust 分离为标识（本类型）与数据（World 中的完整 roster）——
/// 赛制引擎只做对阵推进，结算经 [`MatchRunner`] 回调由引擎闭包完成。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TeamEntry {
    pub id: TeamId,
    /// 队伍签名（胜者判定用：`series.winner_sig == entry.signature`）
    pub signature: String,
}

impl TeamEntry {
    /// 轮空占位（= Kotlin `BracketSeeding.BYE`）。
    pub fn bye() -> Self {
        Self {
            id: TeamId::NONE,
            signature: "BYE".to_string(),
        }
    }

    /// 是否为轮空占位（Kotlin 引用判定 → Rust id 哨兵判定）。
    pub fn is_bye(&self) -> bool {
        self.id == TeamId::NONE
    }
}

/// 一场比赛的结算回调——赛制引擎与结算层之间的**唯一耦合点**。
///
/// 赛制引擎只负责「对阵推进」，不碰 VRS / 生涯 / 实体仓库；
/// 结算（series 模拟 + 生涯回写 + VRS 积分）由调用方（赛事引擎）注入闭包。
/// 第 4 个参数 `stage` 由**赛制引擎**按当前阶段传入（小组赛 / 淘汰赛），
/// 供 HLTV TOP20 的「淘汰赛硬仗 Rating」单独累计。
pub type MatchRunner<'a> = dyn FnMut(TeamId, TeamId, i32, SeriesStage) -> SeriesResult + 'a;

/// 赛制引擎的统一产出：冠军 + 全部系列赛（按进行顺序）+ 折算比分。
#[derive(Debug, Clone)]
pub struct BracketResult {
    pub champion: TeamEntry,
    pub series: Vec<SeriesResult>,
    pub matches: Vec<MatchResult>,
}

/// 分组 / 对阵种子工具（赛制引擎共用；= Kotlin `BracketSeeding`）。
pub struct BracketSeeding;

impl BracketSeeding {
    /// 蛇形分组（snake seeding）：把按种子排序的队伍分进 group_count 个实力均衡的小组。
    pub fn snake_seed(teams: &[TeamEntry], group_count: usize) -> Vec<Vec<TeamEntry>> {
        assert!(group_count > 0, "分组数必须为正");
        if teams.is_empty() {
            return vec![Vec::new(); group_count];
        }
        let mut groups: Vec<Vec<TeamEntry>> = vec![Vec::new(); group_count];
        let mut forward = true;
        for (index, team) in teams.iter().enumerate() {
            let group_index = if forward {
                index % group_count
            } else {
                group_count - 1 - index % group_count
            };
            groups[group_index].push(team.clone());
            if (index + 1) % group_count == 0 {
                forward = !forward;
            }
        }
        groups
    }

    /// 单败淘汰的**标准种子对阵顺序**（= Kotlin `bracketOrder`）：
    /// 8 队 → 1,8,4,5,2,7,3,6。
    ///
    /// 非 2 的幂（5/6/7/9 队等）**不再补 BYE**：直接按标准种子顺序输出
    /// 全部 N 队（递归镜像序列截断到 N）。轮空由赛制引擎逐轮处理
    /// （奇偶轮空：每轮队伍数奇数时最强种子直接晋级），避免
    /// 「16 槽 7 轮空 / 9 队仅 1 场真赛」的轮空过载。
    ///
    /// 2 的幂队伍数行为与旧版完全一致（无 BYE 可补，输出顺序不变）。
    pub fn bracket_order(seeded: &[TeamEntry]) -> Vec<TeamEntry> {
        assert!(!seeded.is_empty(), "单败淘汰需要至少一支队伍");
        // 标准种子 bracket 下标序列：1, n, 2, n-1, 4, n-3, 3, n-2, …（n = 2^k 槽数）
        let mut size = 1;
        while size < seeded.len() {
            size *= 2;
        }
        let mut order = vec![1usize];
        while order.len() < size {
            let new_size = order.len() * 2;
            order = order
                .iter()
                .flat_map(|i| vec![*i, new_size + 1 - *i])
                .collect();
        }
        // 截断到 N 队：只取 1..=N 的下标（非 2 幂时剩余下标 > N 自然丢弃）。
        order
            .into_iter()
            .filter(|&i| i <= seeded.len())
            .map(|i| seeded[i - 1].clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<TeamEntry> {
        (0..n)
            .map(|i| TeamEntry {
                id: TeamId(i as u32),
                signature: format!("T{i}"),
            })
            .collect()
    }

    #[test]
    fn snake_seed_balances_strength() {
        let teams = entries(8);
        let groups = BracketSeeding::snake_seed(&teams, 2);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 4);
        // 第 1 组收 1,4,5,8；第 2 组收 2,3,6,7（蛇形）
        assert_eq!(groups[0][0].signature, "T0");
        assert_eq!(groups[0][1].signature, "T3");
        assert_eq!(groups[1][0].signature, "T1");
    }

    #[test]
    fn bracket_order_standard_8() {
        let teams = entries(8);
        let order = BracketSeeding::bracket_order(&teams);
        let sigs: Vec<&str> = order.iter().map(|t| t.signature.as_str()).collect();
        assert_eq!(sigs, vec!["T0", "T7", "T3", "T4", "T1", "T6", "T2", "T5"]);
    }

    #[test]
    fn bracket_order_truncates_to_teams_no_bye() {
        // 5 队非 2 的幂：**不再补 BYE**——标准种子序列（1,8,4,5,2,7,3,6）
        // 截断到 ≤5 → [1,4,5,2,3]，输出恰好 5 队。轮空由赛制引擎逐轮
        // 奇偶处理（最强种子直接晋级），避免「8 槽 3 轮空」的轮空过载。
        let teams = entries(5);
        let order = BracketSeeding::bracket_order(&teams);
        assert_eq!(order.len(), 5, "非 2 的幂不再补 BYE，输出 N 队");
        assert_eq!(
            order.iter().filter(|t| t.is_bye()).count(),
            0,
            "非 2 的幂不再补 BYE"
        );
        let sigs: Vec<&str> = order.iter().map(|t| t.signature.as_str()).collect();
        assert_eq!(sigs, vec!["T0", "T3", "T4", "T1", "T2"], "标准种子截断到 5");
    }
}
