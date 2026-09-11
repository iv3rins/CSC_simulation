//! HLTV TOP20 综合评分（M8 核心）——把「数据基准 + 荣誉 + 淘汰赛表现 + 样本惩罚」
//! 组装为单一 `Top20_Score`，替代旧版纯场均排序。
//!
//! 公式（= 设计稿 `docs/TOP20-evaluator-design.md` §3）：
//! ```text
//! Top20_Score
//!   = ( Weighted_Rating × 0.60
//!     + Honor_Points   × 0.15
//!     + Playoff_Impact × 0.25 )
//!   × Sample_Penalty
//! ```
//!
//! 哲学（HLTV）：**「入围看个人数据与样本量，排名高低看大比赛淘汰赛表现
//! 与 MVP/EVP 数量。」**
//!
//! 纯函数、无随机源——同种子同决策序列 → 同 TOP20（可复现性不破）。

use std::collections::HashSet;

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::career::IndividualHonourType;
use csc_entities::world::World;
use csc_util::id::PlayerId;

use crate::yearly_rating::{YearlyRatingTracker, YearlyStat};

/// 一名选手的 TOP20 综合评分（含中间量，供排名与评语）。
#[derive(Debug, Clone, PartialEq)]
pub struct Top20Score {
    pub player_id: PlayerId,
    /// 综合得分（越高越靠前）
    pub score: f64,
    /// 加权场均 Rating（T1+ 按 tier 权重）
    pub weighted_rating: f64,
    /// MVP/EVP 荣誉积分（见 [`Top20Evaluator::mvp_score`] / [`Top20Evaluator::evp_score`]）
    pub honor_points: f64,
    /// 淘汰赛加权场均 Rating（无淘汰赛样本 = None）
    pub playoff_rating: Option<f64>,
    /// T1+ 参赛图数（样本量）
    pub maps: i32,
    /// 本年度 MVP 次数
    pub mvp_count: i32,
    /// 本年度 EVP 次数
    pub evp_count: i32,
    /// 是否「天才少年通配」入围（低阶赛事 Rating 极优的年轻选手，无 T1+ 样本）
    pub wildcard: bool,
}

/// 一届年度 TOP20 榜单的**持久化快照**（含每名入选者的入选依据）。
///
/// 存档进 `GameState`——解决「TOP20 历史不可复盘」缺口：年度累计器在颁奖后
/// 清空、NPC 无 `CareerInfo` 不能写荣誉，此前历届榜单（含 NPC 名次）在结算后
/// 丢失。快照保留 rank/评分组成/样本量/荣誉次数 → 「为什么 A 高于 B」可查。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Top20YearBoard {
    /// 归属年份
    pub year: i32,
    /// 该届全部入选者（按名次升序）
    pub entries: Vec<Top20BoardEntry>,
}

/// 榜单快照中的一名入选者（= 名次 + 评分组成的可解释依据）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Top20BoardEntry {
    /// 名次（1 起）
    pub rank: usize,
    pub player_id: PlayerId,
    /// 入选时昵称（选手可能退役/改名，快照保留历史名）
    pub player_name: String,
    /// 综合得分
    pub score: f64,
    /// 加权场均 Rating（0.60 权重项）
    pub weighted_rating: f64,
    /// MVP/EVP 荣誉积分（0.15 权重项）
    pub honor_points: f64,
    /// 淘汰赛加权场均 Rating（0.25 权重项；None = 无淘汰赛样本）
    pub playoff_rating: Option<f64>,
    /// T1+ 参赛图数（样本量）
    pub maps: i32,
    /// 本年度 MVP 次数
    pub mvp_count: i32,
    /// 本年度 EVP 次数
    pub evp_count: i32,
    /// 是否「天才少年通配」入围
    pub wildcard: bool,
}

/// HLTV TOP20 综合评分器——纯函数（无状态、无随机）。
pub struct Top20Evaluator;

impl Top20Evaluator {
    // —— 权重（HLTV 2024 methodology 校准，见 final-list 文章）——
    // HLTV 原文：「Statistics… not just final rating… biggest matches, best
    // opposition, biggest trophies；Awards… crucial role, but grades differ；
    // Sample size… brought down a notch」。据此：基础数据 60% 主导，
    // 大场面/淘汰赛语境 25%，MVP/EVP 荣誉 15%——荣誉是加分项，不能喧宾夺主。
    pub const WEIGHT_RATING: f64 = 0.60;
    pub const WEIGHT_HONOR: f64 = 0.15;
    pub const WEIGHT_PLAYOFF: f64 = 0.25;
    /// 样本惩罚的满额图数（低于此值按比例打折）。
    /// 2026 真实赛历（25 场 T1 桶/年）+ 16 队瑞士轮 BO3 的深度下，满勤 T1 主力
    /// 全年可积累 150–230 张 T1+ 图（实测 7 个月 138 图）——阈值 70 在模拟里
    /// 人人轻松超过、惩罚恒为 1.0。提到 150 后：半程主力（75 图）拿 50% 折扣、
    /// 20 图底线选手 13%，样本区分度恢复。
    pub const TARGET_MAPS: f64 = 150.0;
    /// 入围最低 T1+ 图数（低于此值直接不入围——HLTV 伪代码 `continue`）。
    pub const MIN_MAPS: i32 = 20;

    // —— 天才少年通配（2026 平衡修订：青训与黄金期逻辑）——
    /// 通配选手年龄上限（≤26 岁）。
    pub const WILDCARD_MAX_AGE: i32 = 26;
    /// 通配全赛事 Rating 门槛（低阶赛事表现极优）。
    pub const WILDCARD_RATING_MIN: f64 = 1.25;
    /// 通配全赛事最低图数（样本充足防单图爆发）。
    pub const WILDCARD_MIN_MAPS: i32 = 30;
    /// 通配的确定性"概率"（%）：由 (PlayerId, year) 哈希派生——同世界同结果，
    /// 不消费 RNG 序列、不破坏可复现性。
    pub const WILDCARD_CHANCE_PCT: u64 = 40;
    /// 每届榜单最多通配入围数（试玩校准：40% 命中率下 11/20 全是外卡太泛滥）。
    pub const MAX_WILDCARDS: usize = 5;

    // —— 荣誉分值矩阵（HLTV 2024 calibration）——
    // 2026 重构：旧版 MVP=10.0/EVP=5.0 是「绝对分」，与 1.2 上下的 Rating
    // 直接相加后荣誉项彻底压过基础数据（NiKo 1.16 + 6 EVP 的榜位失真）。
    // 新矩阵把奖项折算成**Rating 等值单位**（award units），再乘 15% 权重：
    // 6 个不同等级 EVP ≈ 1.6 units → +0.24 总分；1 个 Major MVP = 1.0 unit。
    // 这也对齐 HLTV「MVP/EVP grades differ by competition level」。
    pub fn mvp_score(tier: TourneyTier) -> f64 {
        match tier {
            TourneyTier::Major => 1.00,
            TourneyTier::SuperElite => 0.80,
            TourneyTier::Elite => 0.60,
            TourneyTier::T1 => 0.40,
            TourneyTier::T2 | TourneyTier::Qualify => 0.0,
        }
    }

    pub fn evp_score(tier: TourneyTier) -> f64 {
        match tier {
            TourneyTier::Major => 0.50,
            TourneyTier::SuperElite => 0.40,
            TourneyTier::Elite => 0.30,
            TourneyTier::T1 => 0.20,
            TourneyTier::T2 | TourneyTier::Qualify => 0.0,
        }
    }

    /// 纯评分函数：与 [`Self::evaluate`] 同一条公式（供单测/审计直接校准）。
    pub fn score_of(
        weighted_rating: f64,
        honor_points: f64,
        playoff_rating: Option<f64>,
        maps: i32,
    ) -> f64 {
        let playoff_term = playoff_rating.unwrap_or(weighted_rating);
        (weighted_rating * Self::WEIGHT_RATING
            + honor_points * Self::WEIGHT_HONOR
            + playoff_term * Self::WEIGHT_PLAYOFF)
            * Self::sample_penalty(maps)
    }

    /// 样本惩罚：图数达标 → 1.0；否则按比例打折。
    pub fn sample_penalty(maps: i32) -> f64 {
        (maps as f64 / Self::TARGET_MAPS).min(1.0)
    }

    /// 天才少年通配的确定性判定：`(PlayerId × 黄金比常数) ^ year` 混合哈希
    /// 取模 100 < `WILDCARD_CHANCE_PCT`。同输入恒同输出（纯函数）。
    pub fn wildcard_hit(player_id: PlayerId, year: i32) -> bool {
        let h = (player_id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (year as u64).wrapping_mul(0x517C_C1B7_2722_0A95);
        (h >> 32) as u32 % 100 < Self::WILDCARD_CHANCE_PCT as u32
    }

    /// 计算全年全部玩家的 TOP20 综合评分（未排序、未截断）。
    ///
    /// @param world   实体仓库（读 career.honours 的 MVP/EVP）
    /// @param tracker 年度累计器（读加权场均 + 淘汰赛场均 + 图数）
    /// @param year    归属年份（匹配荣誉）
    pub fn evaluate(world: &World, tracker: &YearlyRatingTracker, year: i32) -> Vec<Top20Score> {
        let stats = tracker.stats();
        let mut scores: Vec<Top20Score> = Vec::new();
        for (pid, stat) in &stats {
            let (mvp_count, evp_count, honor_points) = Self::honor_of(tracker, world, *pid, year);
            let playoff_rating = tracker.playoff_rating_of(*pid);
            let weighted_rating = stat.rating;
            // Playoff_Impact：有淘汰赛样本用淘汰赛场均；无样本中性回落到加权场均
            // （既不虚高奖励，也不硬扣——对应设计稿「无淘汰赛样本按中性值处理」）
            let score = Self::score_of(weighted_rating, honor_points, playoff_rating, stat.maps);
            scores.push(Top20Score {
                player_id: *pid,
                score,
                weighted_rating,
                honor_points,
                playoff_rating,
                maps: stat.maps,
                mvp_count,
                evp_count,
                wildcard: false,
            });
        }
        // 按综合分降序；同分按 PlayerId 升序（确定性 tiebreak——防 HashMap 迭代序影响同分排名）
        scores.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        scores
    }

    /// 全年 TOP20 榜单（含**天才少年通配**）：正常入围 + 低阶赛事极优的年轻
    /// 选手按确定性概率补进 TOP10-20 区段。
    ///
    /// 规则（2026 平衡修订「青训与黄金期逻辑」）：
    /// 年龄 ≤26、全赛事 Rating ≥1.25、全赛事 ≥30 图、T1+ 图数 < `MIN_MAPS` 的
    /// 玩家，以 `WILDCARD_CHANCE_PCT` 的确定性概率获得一张"未来之星"外卡。
    /// 外卡得分封顶在第 9 名正常选手之下（保证只落在 10-20 区段），不挤占前十。
    /// 兼容旧 API 的年度 TOP20 入口。
    ///
    /// 旧版本只按表现/荣誉/淘汰赛 Rating/样本量排名，`wildcard_hit` 是死代码。
    /// 现在真正把低阶赛事表现极优的年轻选手按确定性哈希补进 10-20 区段。
    pub fn evaluate_with_wildcards(
        world: &World,
        tracker: &YearlyRatingTracker,
        year: i32,
    ) -> Vec<Top20Score> {
        let stats = tracker.stats();
        let all = Self::evaluate(world, tracker, year);

        // 正常入围：已通过 MIN_MAPS（T1+ 口径）的选手，按 score 降序。
        let normal: Vec<Top20Score> = all
            .iter()
            .filter(|s| s.maps >= Self::MIN_MAPS)
            .cloned()
            .collect();
        // 正常入围前 20（外卡候选不得挤占这些正常名次）。
        let normal_top20: HashSet<PlayerId> = normal.iter().take(20).map(|s| s.player_id).collect();

        // 外卡候选：低阶赛事极优、未入围正常 TOP20 的年轻选手，确定性哈希命中。
        let mut wildcards: Vec<Top20Score> = Vec::new();
        for (pid, stat) in &stats {
            if normal_top20.contains(pid) {
                continue;
            }
            let Some(pc) = world.player(*pid) else {
                continue;
            };
            // 年龄 ≤ 上限、全赛事 Rating 达标、全赛事图数充足，但 T1+ 图数不足。
            if pc.age > Self::WILDCARD_MAX_AGE
                || stat.all_rating < Self::WILDCARD_RATING_MIN
                || stat.maps_all < Self::WILDCARD_MIN_MAPS
                || stat.maps >= Self::MIN_MAPS
                || !Self::wildcard_hit(*pid, year)
            {
                continue;
            }

            let (_, _, honor_points) = Self::honor_of(tracker, world, *pid, year);
            let playoff_rating = tracker.playoff_rating_of(*pid);
            let raw_score =
                Self::score_of(stat.all_rating, honor_points, playoff_rating, stat.maps_all);
            wildcards.push(Top20Score {
                player_id: *pid,
                score: raw_score,
                weighted_rating: stat.all_rating,
                honor_points,
                playoff_rating,
                maps: stat.maps_all,
                mvp_count: 0,
                evp_count: 0,
                wildcard: true,
            });
        }

        // 外卡排序后封顶，保证确定性 + 上限 + 不挤占前十：
        //  - 同分按 PlayerId 升序（确定性 tiebreak）；
        //  - 得分封顶在第 9 名正常选手之下，正常不足 9 名时封顶在末位正常选手之下
        //    （否则外卡会挤进前 9，违背『只落 10-20』）；
        //  - 每届最多 MAX_WILDCARDS 张外卡。
        wildcards.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        let cap = normal
            .get(8)
            .or_else(|| normal.last())
            .map(|s| s.score - 1e-9);
        if let Some(cap) = cap {
            for w in &mut wildcards {
                w.score = w.score.min(cap);
            }
        }
        wildcards.truncate(Self::MAX_WILDCARDS);

        // 外卡替换掉其在 evaluate() 里的原始低分条目（避免同一选手出现两次），
        // 合并后按 (score 降序, PlayerId 升序) 重排；调用方 .take(20) 消费。
        let mut ranked: Vec<Top20Score> = normal; // 正常入围（maps>=MIN_MAPS）
        ranked.extend(wildcards);
        ranked.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        ranked
    }

    /// 年度 MVP/EVP 荣誉统计与折算积分。
    ///
    /// **2026 修订（NPC 参与）**：主源 = 年度累计器（`YearlyRatingTracker`，赛事
    /// 结算时全员含 NPC 写入）；旧存档（tracker 无此数据）回退读玩家 career
    /// 荣誉记录（与修订前行为一致）。
    fn honor_of(
        tracker: &YearlyRatingTracker,
        world: &World,
        pid: PlayerId,
        year: i32,
    ) -> (i32, i32, f64) {
        if let Some((mvp, evp, points)) = tracker.honour_of(pid) {
            return (mvp, evp, points);
        }
        let Some(pc) = world.player(pid) else {
            return (0, 0, 0.0);
        };
        let Some(career) = pc.career.as_ref() else {
            return (0, 0, 0.0);
        };
        let mut mvp_count = 0;
        let mut evp_count = 0;
        let mut points = 0.0;
        for h in &career.honours.individual {
            if h.year != year {
                continue;
            }
            match h.r#type {
                IndividualHonourType::Mvp => {
                    mvp_count += 1;
                    points += h.tier.map(Self::mvp_score).unwrap_or(0.0);
                }
                IndividualHonourType::Evp => {
                    evp_count += 1;
                    points += h.tier.map(Self::evp_score).unwrap_or(0.0);
                }
                _ => {}
            }
        }
        (mvp_count, evp_count, points)
    }
}

/// 供测试与展示导出的「某玩家年度统计 + 评分」便捷视图。
pub type YearlyScored = (PlayerId, YearlyStat);

/// HLTV 风格评语（玩家端代入感；纯函数，基于 `Top20Score` 中间量条件触发）。
///
/// 规则（= 设计稿 §5）：
/// 1. 年度最佳——榜首 + Award 统治级；
/// 2. 高 Rating 低排名——rating 高但缺硬仗/奖牌；
/// 3. 大赛型选手——淘汰赛 Rating 高 + EVP；
/// 4. 样本不足——出场图数低于满额，排名被打折。
pub struct Top20Commentary;

impl Top20Commentary {
    /// 淘汰赛 Rating 高水位阈值（>1.15 视为"统治级硬仗"）。
    pub const PLAYOFF_ELITE: f64 = 1.15;

    /// 为榜单上某一位（0 基 index）生成评语（文案来自 `text` 包 `top20.*` 键）。
    ///
    /// @param ranked 已按 score 降序排序的榜单
    /// @param index  目标玩家在榜单中的位置（0 基）
    pub fn comment(ranked: &[Top20Score], index: usize, text: &csc_text::TextBundle) -> String {
        let Some(s) = ranked.get(index) else {
            return String::new();
        };
        let rank = index + 1;

        // 0. 天才少年通配：低阶赛事杀出的未来之星
        if s.wildcard {
            return text.format("top20.wildcard", &[&format!("{:.2}", s.weighted_rating)]);
        }

        // 1. 年度最佳：榜首 + 多 MVP（或淘汰赛统治）
        if rank == 1
            && (s.mvp_count >= 2
                || s.playoff_rating
                    .map(|p| p >= Self::PLAYOFF_ELITE)
                    .unwrap_or(false))
        {
            return text.format("top20.best", &[&s.mvp_count.to_string()]);
        }

        // 4. 样本不足（优先提示——解释了排名被压低的原因）
        if s.maps < Top20Evaluator::MIN_MAPS * 2 {
            return text.format("top20.low_sample", &[&s.maps.to_string()]);
        }

        // 3. 大赛型选手：淘汰赛高 + EVP
        if s.playoff_rating
            .is_some_and(|p| p >= Self::PLAYOFF_ELITE && s.evp_count > 0)
        {
            let p = s.playoff_rating.unwrap();
            return text.format(
                "top20.playoff",
                &[&format!("{p:.2}"), &s.evp_count.to_string()],
            );
        }

        // 2. 高 Rating 低排名：rating 高但 rank 明显靠后（第 4 名之后）
        if s.weighted_rating >= 1.15 && rank > 4 && s.mvp_count == 0 && s.evp_count == 0 {
            return text.format("top20.high_rating", &[&format!("{:.2}", s.weighted_rating)]);
        }

        // 默认：中性概述
        text.format(
            "top20.default",
            &[
                &format!("{:.2}", s.weighted_rating),
                &s.mvp_count.to_string(),
                &s.evp_count.to_string(),
                &rank.to_string(),
            ],
        )
    }

    /// 为整个榜单生成 (排名, 玩家 ID, 评语) 列表。
    pub fn all(
        ranked: &[Top20Score],
        text: &csc_text::TextBundle,
    ) -> Vec<(usize, PlayerId, String)> {
        ranked
            .iter()
            .enumerate()
            .map(|(i, s)| (i + 1, s.player_id, Self::comment(ranked, i, text)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honor_score_matrix_matches_hlvt() {
        assert_eq!(Top20Evaluator::mvp_score(TourneyTier::Major), 1.0);
        assert_eq!(Top20Evaluator::mvp_score(TourneyTier::SuperElite), 0.8);
        assert_eq!(Top20Evaluator::mvp_score(TourneyTier::Elite), 0.6);
        assert_eq!(Top20Evaluator::mvp_score(TourneyTier::T1), 0.4);
        assert_eq!(Top20Evaluator::mvp_score(TourneyTier::T2), 0.0);
        assert_eq!(Top20Evaluator::evp_score(TourneyTier::Major), 0.5);
        assert_eq!(Top20Evaluator::evp_score(TourneyTier::Elite), 0.3);
        assert_eq!(Top20Evaluator::evp_score(TourneyTier::T1), 0.2);
    }

    #[test]
    fn hlvt_2024_style_calibration() {
        // 以真实 2024 TOP5 形态校准（rating/荣誉形态参考 HLTV final-list 各选手页）：
        // donk（rating 最高 + 多个 Major/Super-Elite MVP） > m0NESY（1.27 + 多 MVP）
        // > ZywOo（1.32 但奖项少） > NiKo（1.16 + 6 EVP 无 MVP） > jL（1.13 + Major MVP）。
        let full = |rating: f64, honor: f64, playoff: f64| {
            Top20Evaluator::score_of(rating, honor, Some(playoff), 70)
        };
        let donk = full(1.36, 2.2, 1.38);
        let monesy = full(1.27, 2.0, 1.30);
        let zywoo = full(1.32, 1.4, 1.26);
        let niko = full(1.16, 1.6, 1.19);
        let jl = full(1.13, 1.6, 1.15);
        assert!(
            donk > monesy && monesy > zywoo && zywoo > niko && niko > jl,
            "{donk:.3} {monesy:.3} {zywoo:.3} {niko:.3} {jl:.3}"
        );
        // 基础数据权重：0.5 Rating 差 × 0.6 不应被 1.0 unit 荣誉完全吞掉
        let high_rating = full(1.35, 0.0, 1.30);
        let low_rating_one_mvp = full(1.00, 1.0, 1.00);
        assert!(
            high_rating > low_rating_one_mvp,
            "基础数据应主导榜位：{high_rating:.3} vs {low_rating_one_mvp:.3}"
        );
        // 荣誉仍关键：同 rating 时 6 个 T1 EVP（1.2 units）足以压过无荣誉
        let six_evp = full(1.15, 1.2, 1.15);
        let no_honor = full(1.15, 0.0, 1.15);
        assert!(
            six_evp > no_honor,
            "EVP 群应形成有效加成：{six_evp:.3} vs {no_honor:.3}"
        );
    }

    #[test]
    fn sample_penalty_scales_by_maps() {
        // 2026 复审校准：满额图数 70 → 150（对齐真实赛历下 T1 主力全年 150+ 图）
        assert_eq!(Top20Evaluator::sample_penalty(150), 1.0);
        assert_eq!(Top20Evaluator::sample_penalty(200), 1.0, "超出封顶 1.0");
        assert!((Top20Evaluator::sample_penalty(75) - 0.5).abs() < 1e-9);
        assert!((Top20Evaluator::sample_penalty(0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn min_maps_threshold() {
        assert_eq!(Top20Evaluator::MIN_MAPS, 20);
        assert!(Top20Evaluator::sample_penalty(20) < Top20Evaluator::sample_penalty(70));
    }

    #[test]
    fn commentary_triggers_by_profile() {
        use csc_util::id::PlayerId;
        let s = |pid: u32, score: f64, wr: f64, po: Option<f64>, maps: i32, mvp: i32, evp: i32| {
            Top20Score {
                player_id: PlayerId(pid),
                score,
                weighted_rating: wr,
                honor_points: 0.0,
                playoff_rating: po,
                maps,
                mvp_count: mvp,
                evp_count: evp,
                wildcard: false,
            }
        };
        // 年度最佳：榜首 + 多 MVP
        let best = s(1, 2.0, 1.25, Some(1.30), 60, 3, 0);
        assert!(
            Top20Commentary::comment(
                std::slice::from_ref(&best),
                0,
                &csc_text::TextBundle::default()
            )
            .contains("年度最佳")
        );
        // 高 Rating 低排名：rating 高、无奖牌、第 5 名
        let no_honor = s(2, 1.0, 1.20, None, 80, 0, 0);
        let list = vec![
            best,
            s(3, 0.9, 1.0, None, 50, 0, 0),
            s(4, 0.8, 0.9, None, 50, 0, 0),
            s(5, 0.7, 0.85, None, 50, 0, 0),
            no_honor,
        ];
        assert!(
            Top20Commentary::comment(&list, 4, &csc_text::TextBundle::default())
                .contains("恐怖 Rating")
        );
        assert_eq!(
            Top20Commentary::all(&list, &csc_text::TextBundle::default()).len(),
            5
        );
        // 通配评语
        let wc = s(9, 0.6, 1.35, None, 40, 0, 0);
        let wc = Top20Score {
            wildcard: true,
            ..wc
        };
        assert!(
            Top20Commentary::comment(
                std::slice::from_ref(&wc),
                0,
                &csc_text::TextBundle::default()
            )
            .contains("未来之星")
        );
    }

    #[test]
    fn wildcard_hit_is_deterministic() {
        // 同 (pid, year) 恒同判定；不同 pid 有差异
        let a = Top20Evaluator::wildcard_hit(PlayerId(7), 2026);
        let b = Top20Evaluator::wildcard_hit(PlayerId(7), 2026);
        assert_eq!(a, b, "确定性哈希：同输入同输出");
        // 40% 概率在统计上应产生两个阵营（大样本）
        let hits: usize = (0..1000)
            .filter(|pid| Top20Evaluator::wildcard_hit(PlayerId(*pid as u32), 2026))
            .count();
        assert!(hits > 250 && hits < 550, "命中率应接近 40%: {hits}/1000");
    }

    #[test]
    fn wildcard_injects_into_10_20_band() {
        use csc_domain::tier::Tier;
        use csc_entities::character::PlayerCharacter;
        use csc_entities::role::Role;
        use csc_simulation::series::{MapScore, PlayerLine, SeriesStage};
        use csc_util::rng::Xoshiro256StarStar;

        let mut rng = Xoshiro256StarStar::seed(2026);
        let mut world = World::new();
        // 先推入青训新秀 → id 0（2026 年 wildcard_hit(0) 为真）。
        let prospect_id = world.create_player(
            Tier::Tier2,
            Some("Prospect"),
            &mut rng,
            Some(19),
            Some(Role::Rifler),
        );
        assert_eq!(prospect_id, PlayerId(0));
        assert!(
            Top20Evaluator::wildcard_hit(prospect_id, 2026),
            "测试前置：新秀须确定性命中外卡"
        );

        // 15 个满图高分正常选手（id 1..=15）。
        let normal_ids: Vec<PlayerId> = (1..=15)
            .map(|i| {
                world.create_player(
                    Tier::Tier2,
                    Some(&format!("Normal{i}")),
                    &mut rng,
                    Some(24),
                    Some(Role::Rifler),
                )
            })
            .collect();
        assert_eq!(normal_ids.len(), 15);

        let mut tracker = YearlyRatingTracker::default();
        // 正常选手 T1 图：每图 15 人同场，记录 25 场 → 每人 maps=25>=20。
        let normal_rosters: Vec<&PlayerCharacter> = normal_ids
            .iter()
            .map(|id| world.player(*id).unwrap())
            .collect();
        let normal_names: Vec<String> = normal_ids
            .iter()
            .map(|id| world.player(*id).unwrap().name.clone())
            .collect();
        let normal_lines: Vec<PlayerLine> = normal_names
            .iter()
            .map(|name| PlayerLine {
                player_name: name.clone(),
                team_sig: "A|p".to_string(),
                kills: 30,
                deaths: 8,
                assists: 6,
                adr: 90.0,
                kast: 80.0,
            })
            .collect();
        let normal_map = MapScore {
            map_number: 1,
            team_a_score: 13,
            team_b_score: 9,
            winner_sig: "A|p".to_string(),
            lines: normal_lines,
        };
        for _ in 0..25 {
            tracker.record(
                2026,
                &normal_map,
                TourneyTier::T1,
                SeriesStage::Group,
                "A|p",
                &normal_rosters,
                "B|q",
                &[],
            );
        }

        // 新秀 T2 图：记录 35 场 → maps_all=35、maps(T1+)=0（T2 权重 0 不计 T1+）。
        let prospect_name = world.player(prospect_id).unwrap().name.clone();
        let prospect_map = MapScore {
            map_number: 1,
            team_a_score: 13,
            team_b_score: 9,
            winner_sig: "A|p".to_string(),
            lines: vec![PlayerLine {
                player_name: prospect_name.clone(),
                team_sig: "A|p".to_string(),
                kills: 30,
                deaths: 8,
                assists: 6,
                adr: 90.0,
                kast: 80.0,
            }],
        };
        for _ in 0..35 {
            tracker.record(
                2026,
                &prospect_map,
                TourneyTier::T2,
                SeriesStage::Group,
                "A|p",
                &[world.player(prospect_id).unwrap()],
                "B|q",
                &[],
            );
        }

        // 前置自检：新秀满足外卡条件，正常选手满图。
        let stats = tracker.stats();
        let ps = &stats[&prospect_id];
        assert!(
            ps.all_rating >= Top20Evaluator::WILDCARD_RATING_MIN,
            "all_rating={}",
            ps.all_rating
        );
        assert!(
            ps.maps_all >= Top20Evaluator::WILDCARD_MIN_MAPS,
            "maps_all={}",
            ps.maps_all
        );
        assert!(ps.maps < Top20Evaluator::MIN_MAPS, "T1+ maps={}", ps.maps);
        for id in &normal_ids {
            assert!(
                stats[id].maps >= Top20Evaluator::MIN_MAPS,
                "正常选手 maps={}",
                stats[id].maps
            );
        }

        // 生成含外卡的完整榜单（.take(20) 由调用方执行，这里直接看全量）。
        let board = Top20Evaluator::evaluate_with_wildcards(&world, &tracker, 2026);
        let rank_of = |pid: PlayerId| board.iter().position(|s| s.player_id == pid).map(|i| i + 1);

        let prospect_rank = rank_of(prospect_id).expect("新秀应进入榜单");
        assert!(
            (10..=20).contains(&prospect_rank),
            "新秀应落在 10-20 区段，实际第 {prospect_rank} 名"
        );
        let ps_board = &board[prospect_rank - 1];
        assert!(ps_board.wildcard, "新秀条目应为通配外卡");
        assert_eq!(ps_board.maps, ps.maps_all, "外卡 maps 用全赛事口径");

        // 榜内无重复 PlayerId；正常选手不被标记为外卡。
        let mut seen: HashSet<PlayerId> = HashSet::new();
        for s in &board {
            assert!(seen.insert(s.player_id), "榜单不得出现重复选手");
        }
        for id in &normal_ids {
            assert!(
                !board.iter().any(|s| s.player_id == *id && s.wildcard),
                "正常选手不应是外卡"
            );
        }
    }

    #[test]
    fn wildcard_limited_to_max_wildcards() {
        // 回归测试：修复前 MAX_WILDCARDS 未生效，命中候选可能超过 5 人全部注入。
        // 2026 年 wildcard_hit 命中 pid：0,1,7,9,25,30,37,39（8 人）。
        use csc_domain::tier::Tier;
        use csc_entities::role::Role;
        use csc_simulation::series::{MapScore, PlayerLine, SeriesStage};
        use csc_util::rng::Xoshiro256StarStar;

        let hit_pids: Vec<u32> = (0..40u32)
            .filter(|p| Top20Evaluator::wildcard_hit(PlayerId(*p), 2026))
            .collect();
        assert!(hit_pids.len() > Top20Evaluator::MAX_WILDCARDS);

        let mut rng = Xoshiro256StarStar::seed(2026);
        let mut world = World::new();
        // 先建 6 个命中候选（全赛事满足外卡条件：年轻、低 T1+ 图、高全赛事 rating）。
        let cand_ids: Vec<PlayerId> = hit_pids[..6]
            .iter()
            .map(|p| {
                world.create_player(
                    Tier::Tier2,
                    Some(&format!("Prospect{p}")),
                    &mut rng,
                    Some(18),
                    Some(Role::Rifler),
                )
            })
            .collect();
        // 再加 15 个正常满图高分选手（保证榜单有 9+ 名正常选手、cap 有效）。
        let _normals: Vec<PlayerId> = (0..15)
            .map(|i| {
                world.create_player(
                    Tier::Tier2,
                    Some(&format!("Normal{i}")),
                    &mut rng,
                    Some(24),
                    Some(Role::Rifler),
                )
            })
            .collect();

        let mut tracker = YearlyRatingTracker::default();
        // 6 名候选全部记录高表现 T2 赛事 35 场（T1+ 图 = 0 < MIN_MAPS）。
        for id in &cand_ids {
            let name = world.player(*id).unwrap().name.clone();
            let map = MapScore {
                map_number: 1,
                team_a_score: 13,
                team_b_score: 9,
                winner_sig: "A|p".to_string(),
                lines: vec![PlayerLine {
                    player_name: name.clone(),
                    team_sig: "A|p".to_string(),
                    kills: 30,
                    deaths: 8,
                    assists: 6,
                    adr: 90.0,
                    kast: 80.0,
                }],
            };
            for _ in 0..35 {
                tracker.record(
                    2026,
                    &map,
                    TourneyTier::T2,
                    SeriesStage::Group,
                    "A|p",
                    &[world.player(*id).unwrap()],
                    "B|q",
                    &[],
                );
            }
        }
        // 15 名正常选手 T1 图 25 场 → maps=25>=20。
        let normal_rosters: Vec<&csc_entities::character::PlayerCharacter> = _normals
            .iter()
            .map(|id| world.player(*id).unwrap())
            .collect();
        let normal_names: Vec<String> = _normals
            .iter()
            .map(|id| world.player(*id).unwrap().name.clone())
            .collect();
        let normal_lines: Vec<PlayerLine> = normal_names
            .iter()
            .map(|name| PlayerLine {
                player_name: name.clone(),
                team_sig: "A|p".to_string(),
                kills: 30,
                deaths: 8,
                assists: 6,
                adr: 90.0,
                kast: 80.0,
            })
            .collect();
        let normal_map = MapScore {
            map_number: 1,
            team_a_score: 13,
            team_b_score: 9,
            winner_sig: "A|p".to_string(),
            lines: normal_lines,
        };
        for _ in 0..25 {
            tracker.record(
                2026,
                &normal_map,
                TourneyTier::T1,
                SeriesStage::Group,
                "A|p",
                &normal_rosters,
                "B|q",
                &[],
            );
        }

        let board = Top20Evaluator::evaluate_with_wildcards(&world, &tracker, 2026);
        let wildcard_count = board.iter().filter(|s| s.wildcard).count();
        assert!(
            wildcard_count <= Top20Evaluator::MAX_WILDCARDS,
            "外卡数不得超过 MAX_WILDCARDS={}，实际 {wildcard_count}",
            Top20Evaluator::MAX_WILDCARDS
        );
        assert!(wildcard_count >= 1, "应至少注入 1 张外卡（候选齐全）");
        // 外卡必须落在 10-20 区段、不挤占前十。
        for (idx, s) in board.iter().enumerate() {
            if s.wildcard {
                assert!(
                    (10..=20).contains(&(idx + 1)),
                    "外卡 {}({} 名) 应落在 10-20 区段，实际第 {} 名",
                    s.player_id.0,
                    s.weighted_rating,
                    idx + 1
                );
            }
        }
    }
}
