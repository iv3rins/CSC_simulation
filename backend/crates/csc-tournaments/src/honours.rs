//! 赛事个人荣誉判定（MVP / EVP）——从 `award_event_honours` 抽出的**纯判定逻辑**。
//!
//! 目的（架构解耦）：`award_event_honours` 原本把「数据累计 → MVP 判定 → EVP 判定
//! → 写荣誉 → 记日志」揉在一个 100+ 行的函数里，判定规则散落、无法独立单测。
//! 本模块把**判定**抽成纯函数（输入已累计的逐选手数据，输出 MVP/EVP 名单），
//! 不碰 `World`/`Journal`——规则边界可独立、快速地单测，`award_event_honours`
//! 只做「累计 → 调判定 → 落地荣誉」的组合。
//!
//! HLTV 语义（2026 平衡性修订——试玩发现 MVP 泛滥 114 次/12 年）：
//! - **MVP**：只从**冠军或亚军**队伍中产生——冠军方数据最佳者；若亚军方数据
//!   最佳者**明显高出冠军一档**（≥ `RUNNER_UP_MARGIN`），则破例颁给亚军
//!   （真实案例：Marseille 2018 亚军 s1mple 力压冠军 Astralis 夺 MVP）。
//!   小组赛即被淘汰的选手不再能凭少样本高 rating 偷走 MVP。
//! - **EVP**：非 MVP 玩家，淘汰赛阶段场均 Rating ≥ `EVP_RATING_MIN`，
//!   且淘汰赛至少出战 `EVP_MIN_MAPS` 图——兑现「硬仗表现拔尖」。

use csc_util::id::{PlayerId, TeamId};

/// 逐选手累计数据：(player_id, 所属队伍, Σrating, 图数)。MVP 用全赛事，EVP 用淘汰赛。
pub type RatingAccum = Vec<(PlayerId, TeamId, f64, i32)>;

/// 一次个人荣誉判定结果（MVP + EVP 名单，均已排序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HonourDecision {
    /// 本赛事 MVP（None = 无玩家达标）
    pub mvp: Option<PlayerId>,
    /// 本赛事 EVP 名单（按淘汰赛场均降序，至多 `MAX_EVP_PER_EVENT` 名）
    pub evps: Vec<PlayerId>,
}

/// 个人荣誉判定器——纯函数（无状态、无随机）。
pub struct HonourEvaluator;

impl HonourEvaluator {
    /// MVP 至少需要出战的图数（防单图/一轮游爆发）。
    pub const MVP_MIN_MAPS: i32 = 2;
    /// 亚军破例门槛：亚军最佳场均高出冠军最佳多少才颁给亚军（「明显高出一档」）。
    pub const RUNNER_UP_MARGIN: f64 = 0.15;
    /// EVP 淘汰赛场均 Rating 门槛（HLTV「极具价值」≈ 大赛淘汰赛 ≥1.10）。
    pub const EVP_RATING_MIN: f64 = 1.10;
    /// EVP 至少需要出战的淘汰赛图数（防单图爆发）。
    pub const EVP_MIN_MAPS: i32 = 2;
    /// 每赛事最多颁发的 EVP 数量（避免泛滥稀释含金量）。
    pub const MAX_EVP_PER_EVENT: usize = 3;

    /// 判 MVP：冠军方数据最佳者；亚军方最佳者**明显高出冠军一档**
    /// （≥ [`Self::RUNNER_UP_MARGIN`]）时破例颁给亚军。
    ///
    /// @param overall  全赛事逐选手累计（含所属队伍）
    /// @param champion 冠军队伍 ID
    /// @param runner_up 亚军队伍 ID（None = 无亚军/决赛契约破坏，只评冠军方）
    pub fn pick_mvp(
        overall: &RatingAccum,
        champion: TeamId,
        runner_up: Option<TeamId>,
    ) -> Option<PlayerId> {
        let best = |team: TeamId| -> Option<(PlayerId, f64)> {
            overall
                .iter()
                .filter(|(_, tid, _, maps)| *tid == team && *maps >= Self::MVP_MIN_MAPS)
                .map(|(pid, _, r, maps)| (*pid, r / *maps as f64))
                .max_by(|a, b| a.1.total_cmp(&b.1))
        };
        let champ_best = best(champion);
        let runner_best = runner_up.and_then(best);
        match (champ_best, runner_best) {
            (None, None) => None,
            (Some((pid, _)), None) => Some(pid),
            (None, Some((pid, _))) => Some(pid),
            (Some((c_pid, c_avg)), Some((r_pid, r_avg))) => {
                if r_avg > c_avg + Self::RUNNER_UP_MARGIN {
                    Some(r_pid) // 亚军明显超档 → 破例
                } else {
                    Some(c_pid) // 冠军方最佳
                }
            }
        }
    }

    /// 判 EVP 名单：非 MVP、淘汰赛场均 ≥ `EVP_RATING_MIN`、淘汰赛图数 ≥ `EVP_MIN_MAPS`，
    /// 按场均降序取前 `MAX_EVP_PER_EVENT` 名。
    pub fn pick_evps(playoff: &RatingAccum, mvp: Option<PlayerId>) -> Vec<PlayerId> {
        let mut candidates: Vec<(PlayerId, f64)> = playoff
            .iter()
            .filter(|(pid, _, _, maps)| Some(*pid) != mvp && *maps >= Self::EVP_MIN_MAPS)
            .map(|(pid, _, r, maps)| (*pid, r / *maps as f64))
            .filter(|(_, a)| *a >= Self::EVP_RATING_MIN)
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        candidates.truncate(Self::MAX_EVP_PER_EVENT);
        candidates.into_iter().map(|(pid, _)| pid).collect()
    }

    /// 一站式判定：MVP + EVP。
    pub fn decide(
        overall: &RatingAccum,
        playoff: &RatingAccum,
        champion: TeamId,
        runner_up: Option<TeamId>,
    ) -> HonourDecision {
        let mvp = Self::pick_mvp(overall, champion, runner_up);
        let evps = Self::pick_evps(playoff, mvp);
        HonourDecision { mvp, evps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accum(items: &[(u32, u32, f64, i32)]) -> RatingAccum {
        items
            .iter()
            .map(|(id, team, r, m)| (PlayerId(*id), TeamId(*team), *r, *m))
            .collect()
    }

    #[test]
    fn mvp_prefers_champion_best_over_outside_player() {
        // P1（冠军队）场均 1.2、P2（八强出局队）场均 1.6——按新规则 P2 无资格，MVP=P1
        let overall = accum(&[(1, 10, 12.0, 10), (2, 99, 16.0, 10)]);
        assert_eq!(
            HonourEvaluator::pick_mvp(&overall, TeamId(10), Some(TeamId(20))),
            Some(PlayerId(1))
        );
    }

    #[test]
    fn mvp_runner_up_needs_clear_margin() {
        // 冠军最佳 1.20；亚军最佳 1.10（未超档）→ MVP 冠军
        let overall = accum(&[(1, 10, 12.0, 10), (2, 20, 11.0, 10)]);
        assert_eq!(
            HonourEvaluator::pick_mvp(&overall, TeamId(10), Some(TeamId(20))),
            Some(PlayerId(1))
        );
        // 亚军最佳 1.40（超 0.20 > 0.15）→ 破例颁给亚军
        let overall2 = accum(&[(1, 10, 12.0, 10), (2, 20, 14.0, 10)]);
        assert_eq!(
            HonourEvaluator::pick_mvp(&overall2, TeamId(10), Some(TeamId(20))),
            Some(PlayerId(2))
        );
    }

    #[test]
    fn mvp_min_maps_still_guards() {
        // 冠军队最佳只打 1 图（不达标）→ 亚军合规者补位
        let overall = accum(&[(1, 10, 1.9, 1), (2, 20, 12.0, 10)]);
        assert_eq!(
            HonourEvaluator::pick_mvp(&overall, TeamId(10), Some(TeamId(20))),
            Some(PlayerId(2))
        );
        // 双方都无合规者 → None
        let overall2 = accum(&[(1, 10, 1.9, 1)]);
        assert_eq!(
            HonourEvaluator::pick_mvp(&overall2, TeamId(10), Some(TeamId(20))),
            None
        );
    }

    #[test]
    fn evp_excludes_mvp_and_filters_by_rating_and_maps() {
        // 淘汰赛：P1 MVP（排除）、P2 场均 1.3（达标）、P3 场均 0.9（不达标）、P4 场均 1.5 但 1 图（图数不足）
        let playoff = accum(&[
            (1, 10, 13.0, 10),
            (2, 20, 13.0, 10),
            (3, 30, 9.0, 10),
            (4, 40, 1.5, 1),
        ]);
        assert_eq!(
            HonourEvaluator::pick_evps(&playoff, Some(PlayerId(1))),
            vec![PlayerId(2)]
        );
    }

    #[test]
    fn evp_capped_and_sorted_desc() {
        // 4 名达标者，只取前 3（MAX_EVP_PER_EVENT），按场均降序
        let playoff = accum(&[
            (1, 10, 14.0, 10),
            (2, 20, 13.0, 10),
            (3, 30, 12.5, 10),
            (4, 40, 12.0, 10),
        ]);
        let evps = HonourEvaluator::pick_evps(&playoff, None);
        assert_eq!(evps, vec![PlayerId(1), PlayerId(2), PlayerId(3)]);
        assert_eq!(evps.len(), HonourEvaluator::MAX_EVP_PER_EVENT);
    }

    #[test]
    fn decide_combines_mvp_and_evps() {
        let overall = accum(&[(1, 10, 14.0, 10), (2, 20, 13.0, 10)]);
        let playoff = accum(&[(1, 10, 14.0, 10), (2, 20, 13.0, 10)]);
        let d = HonourEvaluator::decide(&overall, &playoff, TeamId(10), Some(TeamId(20)));
        assert_eq!(d.mvp, Some(PlayerId(1)));
        assert_eq!(d.evps, vec![PlayerId(2)]);
    }
}
