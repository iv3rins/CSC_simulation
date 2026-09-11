//! 官方 VRS 静态因子（Seed）计算函数链（Kotlin `VrsModifiers.kt` 转写）。
//!
//! 官方两层模型：①静态 Seed（本文件）②动态 ELO（`scoring`）。
//! Phase 1 只用本队数据统计 → Phase 2 与全联盟第 5 高比较归一化 → Phase 3 对手质量加权。

use std::collections::HashMap;

use csc_time::window::TimeWindow;

/// 一场已结算的比赛记录（模拟推进时由赛事系统写入，供 Seed 重算）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MatchRecord {
    /// 胜者队伍签名
    pub winner_sig: String,
    /// 败者队伍签名
    pub loser_sig: String,
    /// 比赛时间（Unix 秒）→ 决定时间衰减
    pub timestamp: i64,
    /// 赛事总奖池（美元）→ 决定奖池曲线权重
    pub prize_pool: i32,
    /// 是否线下赛 → 计入 LAN 因子
    pub lan: bool,
}

/// 官方 VRS 静态因子计算（全部为纯函数）。
pub struct VrsModifiers;

impl VrsModifiers {
    /// 每个因子只取「最近 / 最高」的前 N 项（官方 team.js `bucketSize = 10`）
    pub const BUCKET_SIZE: usize = 10;
    /// 离群数：与全联盟第 5 高的值比较（官方 ranking.js `setOutlierCount(5)`）
    pub const OUTLIER_COUNT: usize = 5;
    /// Seed 分 remap 下限（官方 ranking.js `MIN_SEEDED_RANK`）
    pub const MIN_SEEDED_RANK: f64 = 400.0;
    /// Seed 分 remap 上限（官方 ranking.js `MAX_SEEDED_RANK`）
    pub const MAX_SEEDED_RANK: f64 = 2000.0;
    /// 奖池封顶 $1,000,000（官方 data_loader.js `parsePrizePool` cap）
    pub const MAX_PRIZE_POOL: i32 = 1_000_000;

    /// Seed 因子权重（官方 ranking.js `SEED_MODIFIER_FACTORS`，ownNetwork 权重为 0）。
    pub const SEED_FACTOR_WEIGHTS: [(&'static str, f64); 5] = [
        ("bountyCollected", 1.0), // 击败的对手强不强（对手的赏金池）
        ("bountyOffered", 1.0),   // 自己赢得过多少奖金
        ("opponentNetwork", 1.0), // 击败过多少强队（对手网络）
        ("ownNetwork", 0.0),      // 官方权重为 0，仅作结构占位
        ("lanFactor", 1.0),       // 线下赛战绩
    ];

    /// 官方 curveFunction：`1 / (1 + |log10 x|)`，把「奖金 / 100万」压成 0~1 权重。
    pub fn curve_function(x: f64) -> f64 {
        1.0 / (1.0 + x.log10().abs())
    }

    /// 官方 powerFunction：`Math.pow(x, 1)` 即恒等，保留只为对应官方结构。
    #[inline]
    pub fn power_function(x: f64) -> f64 {
        x
    }

    /// 官方 remapValueClamped：线性重映射并 clamp；输入区间退化（a==b）时取中点。
    ///
    /// ⚠️ 与 `csc_time::window::remap_value_clamped` 代数等价但**运算顺序不同**
    /// （本实现 = `clamped*out_end + (1-clamped)*out_start`，csc-time =
    /// `out_start + (out_end-out_start)*clamped`），浮点舍入可在最后 1 ulp 分叉。
    /// `seed_to_elo` 的输出进入 VRS 排名与邀请链，而 VRS 是 Kotlin 官方
    /// ranking.js 的位级转写——**运算顺序属于转写语义的一部分**，在拿到
    /// 跨语言位级 golden 证明前不得合并（结构拆分评审已确认此约束）。
    pub fn remap_value_clamped(
        value: f64,
        in_start: f64,
        in_end: f64,
        out_start: f64,
        out_end: f64,
    ) -> f64 {
        let interp = if in_start == in_end {
            0.5
        } else {
            (value - in_start) / (in_end - in_start)
        };
        let clamped = interp.clamp(0.0, 1.0);
        clamped * out_end + (1.0 - clamped) * out_start
    }

    /// 官方 nthHighest：降序第 n 大（1-based）；n 超长取最小；空表兜底 0.0。
    pub fn nth_highest(values: &[f64], n: usize) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let n = n.max(1); // n=0 时下溢 usize 会 panic（release 亦然），clamp 到 1
        sorted[n.min(values.len()) - 1]
    }

    /// Phase 2 参考值：全联盟第 OUTLIER_COUNT 高；队伍数不足时退化为最大值
    /// （Kotlin 注释：模拟早期队伍少，以最强队伍为基准）。
    fn reference(values: &[f64]) -> f64 {
        if values.len() < Self::OUTLIER_COUNT {
            values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                .max(0.0)
        } else {
            Self::nth_highest(values, Self::OUTLIER_COUNT)
        }
    }
}

/// 单队累计统计（Phase 1 → 2 → 3 的中间产物）。
#[derive(Default)]
struct TeamStats {
    sig: String,
    matches_played: i32,
    last_played: i64,
    // Phase 1 原始统计
    distinct_teams_defeated: f64,
    scaled_lan_wins: f64,
    scaled_winnings: f64,
    // Phase 2 归一化
    bounty_offered: f64,
    own_network: f64,
    lan_participation: f64,
    // Phase 3 对手加权
    opponent_bounties: f64,
    opponent_network: f64,
    // 最终 modifiers
    bounty_collected: f64,
    lan_factor: f64,
}

impl TeamStats {
    /// 5 因子加权平均（官方 calculateSeedModifierValue；sumCoeff = 1+1+1+0+1 = 4）。
    fn seed_value(&self) -> f64 {
        let mods = [
            self.bounty_collected,
            self.bounty_offered,
            self.opponent_network,
            self.own_network,
            self.lan_factor,
        ];
        let mut sum_coeff = 0.0;
        let mut scaled = 0.0;
        for (i, (_, weight)) in VrsModifiers::SEED_FACTOR_WEIGHTS.iter().enumerate() {
            sum_coeff += weight;
            scaled += weight * mods[i];
        }
        if sum_coeff == 0.0 {
            0.0
        } else {
            scaled / sum_coeff
        }
    }
}

impl VrsModifiers {
    /// 主入口：由历史比赛记录计算每队的 raw seed 值（未 remap；= Kotlin `computeSeedValues`）。
    pub fn compute_seed_values(
        matches: &[MatchRecord],
        window: &TimeWindow,
    ) -> HashMap<String, f64> {
        if matches.is_empty() {
            return HashMap::new();
        }

        // ========== Phase 1：只用本队数据统计 ==========
        let mut stats: HashMap<String, TeamStats> = HashMap::new();
        let mut last_win_by_opp: HashMap<String, HashMap<String, i64>> = HashMap::new();
        let mut lan_buckets: HashMap<String, Vec<f64>> = HashMap::new();
        let mut win_buckets: HashMap<String, Vec<f64>> = HashMap::new();

        for m in matches {
            let s = stats
                .entry(m.winner_sig.clone())
                .or_insert_with(|| TeamStats {
                    sig: m.winner_sig.clone(),
                    ..Default::default()
                });
            s.matches_played += 1;
            s.last_played = s.last_played.max(m.timestamp);

            let time_mod = window.window_mod(m.timestamp); // 官方 getTimestampModifier
            lan_buckets
                .entry(m.winner_sig.clone())
                .or_default()
                .push(if m.lan { time_mod } else { 0.0 });
            win_buckets
                .entry(m.winner_sig.clone())
                .or_default()
                .push(m.prize_pool as f64 * time_mod);

            // 网络：同一对手只保留最近一次击败（官方 opponentMap 取 max）
            let opp_map = last_win_by_opp.entry(m.winner_sig.clone()).or_default();
            let prev = opp_map.get(&m.loser_sig).copied();
            if prev.is_none() || prev.unwrap() < m.timestamp {
                opp_map.insert(m.loser_sig.clone(), m.timestamp);
            }
        }
        // 败者也有参赛记录（官方 initTeams 两队都 accumulateMatch）
        for m in matches {
            let s = stats
                .entry(m.loser_sig.clone())
                .or_insert_with(|| TeamStats {
                    sig: m.loser_sig.clone(),
                    ..Default::default()
                });
            s.matches_played += 1;
            s.last_played = s.last_played.max(m.timestamp);
        }

        for s in stats.values_mut() {
            // 确定性护栏（2026 结构拆分修复）：`last_win_by_opp` 内层是 HashMap，
            // `.values()` 迭代序跨进程随机（RandomState）；浮点加法不可结合，
            // 直接 `.sum()` 会在最后 1 ulp 分叉。先收集排序（total_cmp 全序）
            // 再左折叠——求和顺序与哈希桶序解耦，跨进程位级一致。
            s.distinct_teams_defeated = last_win_by_opp
                .get(&s.sig)
                .map(|m| {
                    let mut mods: Vec<f64> = m.values().map(|ts| window.window_mod(*ts)).collect();
                    mods.sort_by(f64::total_cmp);
                    mods.iter().sum()
                })
                .unwrap_or(0.0);
            s.scaled_lan_wins = lan_buckets
                .get(&s.sig)
                .map(|b| top_n_sum(b, Self::BUCKET_SIZE) / Self::BUCKET_SIZE as f64)
                .unwrap_or(0.0);
            s.scaled_winnings = win_buckets
                .get(&s.sig)
                .map(|b| top_n_sum(b, Self::BUCKET_SIZE))
                .unwrap_or(0.0);
        }

        // ========== Phase 2：与全联盟第 5 高的值比较，归一化到 [0,1] ==========
        let all_winnings: Vec<f64> = stats.values().map(|s| s.scaled_winnings).collect();
        let all_opponents: Vec<f64> = stats.values().map(|s| s.distinct_teams_defeated).collect();
        let all_lan: Vec<f64> = stats.values().map(|s| s.scaled_lan_wins).collect();
        let ref_winnings = Self::reference(&all_winnings);
        let ref_opponents = Self::reference(&all_opponents);
        let ref_lan = Self::reference(&all_lan);
        for s in stats.values_mut() {
            // 参考值 ≤ 0（窗口外/无任何胜局）时因子取 0，避免 0/0 → NaN 传播
            s.bounty_offered = if ref_winnings <= 0.0 {
                0.0
            } else {
                (s.scaled_winnings / ref_winnings).min(1.0)
            };
            s.own_network = if ref_opponents <= 0.0 {
                0.0
            } else {
                (s.distinct_teams_defeated / ref_opponents).min(1.0)
            };
            s.lan_participation = if ref_lan <= 0.0 {
                0.0
            } else {
                (s.scaled_lan_wins / ref_lan).min(1.0)
            };
        }

        // ========== Phase 3：按对手质量加权 ==========
        let mut bounty_buckets: HashMap<String, Vec<f64>> = HashMap::new();
        let mut network_buckets: HashMap<String, Vec<f64>> = HashMap::new();
        for m in matches {
            let Some(_) = stats.get(&m.winner_sig) else {
                continue;
            };
            let Some(loser) = stats.get(&m.loser_sig) else {
                continue;
            };
            let time_mod = window.window_mod(m.timestamp);
            let capped_prize = m.prize_pool.clamp(1, Self::MAX_PRIZE_POOL); // 官方 getCappedPrizePool
            let stakes = Self::curve_function(capped_prize as f64 / Self::MAX_PRIZE_POOL as f64); // 官方 stakesModifier
            let match_context = time_mod * stakes; // 官方 matchContext
            bounty_buckets
                .entry(m.winner_sig.clone())
                .or_default()
                .push(loser.bounty_offered * match_context);
            network_buckets
                .entry(m.winner_sig.clone())
                .or_default()
                .push(loser.own_network * match_context);
        }
        for s in stats.values_mut() {
            s.opponent_bounties = bounty_buckets
                .get(&s.sig)
                .map(|b| top_n_sum(b, Self::BUCKET_SIZE) / Self::BUCKET_SIZE as f64)
                .unwrap_or(0.0);
            s.opponent_network = network_buckets
                .get(&s.sig)
                .map(|b| top_n_sum(b, Self::BUCKET_SIZE) / Self::BUCKET_SIZE as f64)
                .unwrap_or(0.0);
        }

        // ========== 组装最终 modifiers + seed ==========
        for s in stats.values_mut() {
            s.bounty_collected = Self::curve_function(s.opponent_bounties);
            s.bounty_offered = Self::curve_function(s.bounty_offered);
            s.lan_factor = Self::power_function(s.lan_participation);
        }
        stats
            .into_iter()
            .map(|(sig, s)| (sig, s.seed_value()))
            .collect()
    }

    /// 把 raw seed 线性重映射到 [400, 2000] 作为 ELO 初始分（官方 ranking.js `seedTeams`）。
    pub fn seed_to_elo(raw_seeds: &HashMap<String, f64>) -> HashMap<String, f64> {
        if raw_seeds.is_empty() {
            return HashMap::new();
        }
        let min_seed = raw_seeds.values().copied().fold(f64::INFINITY, f64::min);
        let max_seed = raw_seeds
            .values()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        raw_seeds
            .iter()
            .map(|(sig, v)| {
                (
                    sig.clone(),
                    Self::remap_value_clamped(
                        *v,
                        min_seed,
                        max_seed,
                        Self::MIN_SEEDED_RANK,
                        Self::MAX_SEEDED_RANK,
                    ),
                )
            })
            .collect()
    }
}

/// 降序前 n 项之和（= Kotlin `sortedDescending().take(BUCKET_SIZE).sum()`）。
fn top_n_sum(values: &[f64], n: usize) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    sorted.iter().take(n).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_function_shape() {
        // x=1 → 1/(1+0) = 1；x=0.01 → 1/(1+2) = 1/3
        assert_eq!(VrsModifiers::curve_function(1.0), 1.0);
        assert!((VrsModifiers::curve_function(0.01) - 1.0 / 3.0).abs() < 1e-12);
        // Kotlin 语义：log10(0) = -Inf → 1/(1+Inf) = 0（无 NaN）
        assert_eq!(VrsModifiers::curve_function(0.0), 0.0);
    }

    #[test]
    fn nth_highest_semantics() {
        assert_eq!(VrsModifiers::nth_highest(&[3.0, 1.0, 2.0], 2), 2.0);
        assert_eq!(
            VrsModifiers::nth_highest(&[3.0, 1.0, 2.0], 5),
            1.0,
            "n 超长取最小"
        );
        assert_eq!(VrsModifiers::nth_highest(&[], 1), 0.0, "空表兜底");
    }

    #[test]
    fn remap_degenerate_takes_midpoint() {
        assert_eq!(
            VrsModifiers::remap_value_clamped(5.0, 5.0, 5.0, 400.0, 2000.0),
            1200.0
        );
        assert_eq!(
            VrsModifiers::remap_value_clamped(0.0, 0.0, 10.0, 400.0, 2000.0),
            400.0
        );
        assert_eq!(
            VrsModifiers::remap_value_clamped(10.0, 0.0, 10.0, 400.0, 2000.0),
            2000.0
        );
    }

    #[test]
    fn seed_value_weights() {
        // 单因子（bountyCollected=1，其余 0）：加权平均 = 1/4（sumCoeff=4）
        let mut s = TeamStats {
            sig: "A".into(),
            ..Default::default()
        };
        s.bounty_collected = 1.0;
        assert!((s.seed_value() - 0.25).abs() < 1e-12);
        s.bounty_offered = 1.0;
        assert!((s.seed_value() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn compute_seed_values_basic() {
        let w = TimeWindow::new(0, 1_000_000);
        let matches = vec![
            MatchRecord {
                winner_sig: "A|p1".into(),
                loser_sig: "B|p2".into(),
                timestamp: 900_000,
                prize_pool: 1_000_000,
                lan: true,
            },
            MatchRecord {
                winner_sig: "A|p1".into(),
                loser_sig: "C|p3".into(),
                timestamp: 950_000,
                prize_pool: 500_000,
                lan: false,
            },
            MatchRecord {
                winner_sig: "B|p2".into(),
                loser_sig: "D|p4".into(),
                timestamp: 980_000,
                prize_pool: 100_000,
                lan: false,
            },
        ];
        let seeds = VrsModifiers::compute_seed_values(&matches, &w);
        // A 胜 2 场（含 1 LAN 大池）、B 胜 1 场 → A seed 应高于 B
        let a = seeds["A|p1"];
        let b = seeds["B|p2"];
        assert!(a > b, "A 的 seed 应更高: {a} vs {b}");
        // 全部在 [0,1] 内（0 = 无任何胜局的队伍，Kotlin 同值）
        for v in seeds.values() {
            assert!(*v >= 0.0 && *v <= 1.0, "seed 应在 [0,1]: {v}");
        }
    }

    #[test]
    fn seed_to_elo_remap_range() {
        let mut raw = HashMap::new();
        raw.insert("A".to_string(), 0.1);
        raw.insert("B".to_string(), 0.9);
        let elo = VrsModifiers::seed_to_elo(&raw);
        assert_eq!(elo["A"], 400.0);
        assert_eq!(elo["B"], 2000.0);
        // 退化：全相同 → 中点
        let mut raw2 = HashMap::new();
        raw2.insert("A".to_string(), 0.5);
        raw2.insert("B".to_string(), 0.5);
        let elo2 = VrsModifiers::seed_to_elo(&raw2);
        assert_eq!(elo2["A"], 1200.0);
    }
}
