//! 年度 Rating 累计器（Kotlin `YearlyRatingTracker.kt` 转写）。
//!
//! 跨赛事累计玩家逐图 Rating 与参赛图数，供年度 TOP20 结算。
//! 转写差异：Kotlin 闭包解析实体并就地写荣誉 → Rust 只累计纯值，
//! `award` 返回排名结果（ID + 场均），荣誉写入由结算层负责。
//!
//! **键控体系（B3 稳定 ID）**：全部累计以 [`PlayerId`] 为键，而非选手名——
//! 联赛数据含青训队共用 org 名的「同名不同队」合法场景，名字键控会导致
//! 两个同名选手的年度数据互相污染（TOP20 串数据）。名字只作展示字段。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::character::PlayerCharacter;
use csc_simulation::rating::RatingCalculator;
use csc_simulation::series::{MapScore, SeriesStage};
use csc_util::id::PlayerId;

/// HLTV TOP20 赛事层级权重（T1 及以上才计入；T2/预选忽略）。
///
/// Major 数据含金量最高，逐级递减——加权场均替代算术场均，避免 T1 好数据
/// 被 T2 预选赛稀释，也兑现「大比赛表现更值钱」的 HLTV 哲学。
pub fn tier_weight(tier: TourneyTier) -> f64 {
    match tier {
        TourneyTier::Major => 1.50,
        TourneyTier::SuperElite => 1.30,
        TourneyTier::Elite => 1.15,
        TourneyTier::T1 => 1.00,
        TourneyTier::T2 | TourneyTier::Qualify => 0.0,
    }
}

/// 年度统计（生涯档案素材；由 `stats` 采集）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct YearlyStat {
    /// 加权场均 Rating（T1+ 赛事按 tier 权重；TOP20 口径）
    pub rating: f64,
    /// 参赛图数（T1+ 才计；供顶部样本惩罚）
    pub maps: i32,
    /// **全赛事**（含 T2/预选）场均 Rating——生涯档案口径（2026 试玩修复：
    /// 此前低级别赛事选手的档案 rating 恒 0，结局复盘"巅峰 Rating 0.00"）。
    #[serde(default)]
    pub all_rating: f64,
    /// **全赛事**（含 T2/预选）参赛图数——生涯档案口径。
    ///
    /// 2026 修复「地图数与击杀数对不上」：`maps`（T1+）只供 TOP20 样本惩罚；
    /// 生涯档案的 `maps_played` 与 `kills`/`deaths`/`wins` 同为**全赛事口径**，
    /// 否则低级别赛事选手的击杀全量、图数被低估，二者对不上。
    #[serde(default)]
    pub maps_all: i32,
    pub kills: i32,
    pub deaths: i32,
    /// 全赛事累计助攻与总回合，用于全年累计 Rating 重算。
    #[serde(default)]
    pub assists: i32,
    #[serde(default)]
    pub rounds: i32,
    /// 系列赛胜场（按图；**全赛事口径**）
    pub wins: i32,
}

/// 全赛事年度统计（含 T2/预选——供「天才少年通配」判定：低阶赛事表现极优的
/// 年轻选手即使没打过 T1 赛事，也有机会凭全赛事数据入围 TOP10-20）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct YearlyStatAll {
    /// 全赛事算术场均 Rating（不分层级）
    pub rating: f64,
    /// 全赛事参赛图数
    pub maps: i32,
}

/// 年度 Rating 累计器（结算层状态）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct YearlyRatingTracker {
    /// 年度累计归属年份（首次记录时初始化；结算后复位）
    year: i32,
    /// 年度 Rating 加权累计（PlayerId → Σ(rating × W_tier)，逐图）
    rating: HashMap<PlayerId, f64>,
    /// 加权累计的分母（PlayerId → Σ W_tier，逐图）——加权场均 = rating / weight
    weight: HashMap<PlayerId, f64>,
    /// 淘汰赛 Rating 加权累计（PlayerId → Σ(rating × W_tier)，仅 Playoff 图）
    playoff_rating: HashMap<PlayerId, f64>,
    /// 淘汰赛加权分母（PlayerId → Σ W_tier，仅 Playoff 图）
    playoff_weight: HashMap<PlayerId, f64>,
    /// 年度参赛图数（T1+ 才计）
    maps: HashMap<PlayerId, i32>,
    /// 年度击杀/死亡
    kills: HashMap<PlayerId, i32>,
    deaths: HashMap<PlayerId, i32>,
    /// `serde(default)`：旧存档缺失时补空表，保证向后兼容。
    #[serde(default)]
    assists: HashMap<PlayerId, i32>,
    #[serde(default)]
    rounds: HashMap<PlayerId, i32>,
    /// 年度系列赛胜场（按图胜者队伍内玩家 +1）
    wins: HashMap<PlayerId, i32>,
    /// **全赛事**（含 T2/预选）Rating 累计（PlayerId → Σ rating，不分层级；天才少年通配用）
    rating_all: HashMap<PlayerId, f64>,
    /// **全赛事**图数（PlayerId → 图数）
    maps_all: HashMap<PlayerId, i32>,
    /// 年度 MVP 次数（**全员含 NPC**——2026 修订：MVP 评定放开 NPC 参与，
    /// TOP20 荣誉维度不再只对主角生效；`serde(default)` 旧存档兼容）
    #[serde(default)]
    mvp_count: HashMap<PlayerId, i32>,
    /// 年度 EVP 次数（全员含 NPC）
    #[serde(default)]
    evp_count: HashMap<PlayerId, i32>,
    /// 年度荣誉积分（MVP/EVP 按赛事 tier 矩阵折算；全员含 NPC）
    #[serde(default)]
    honor_points: HashMap<PlayerId, f64>,
}

impl Default for YearlyRatingTracker {
    fn default() -> Self {
        // year 哨兵 = -1（未锚定；derive Default 会给 0，导致首年判定失效）
        Self {
            year: -1,
            rating: HashMap::new(),
            weight: HashMap::new(),
            playoff_rating: HashMap::new(),
            playoff_weight: HashMap::new(),
            maps: HashMap::new(),
            kills: HashMap::new(),
            deaths: HashMap::new(),
            assists: HashMap::new(),
            rounds: HashMap::new(),
            wins: HashMap::new(),
            rating_all: HashMap::new(),
            maps_all: HashMap::new(),
            mvp_count: HashMap::new(),
            evp_count: HashMap::new(),
            honor_points: HashMap::new(),
        }
    }
}

impl YearlyRatingTracker {
    /// 记录一图数据：累计玩家（非 NPC）逐图加权 Rating / 图数 / 击杀 / 死亡 / 胜场。
    ///
    /// 首次记录时锚定归属年份；NPC 无生涯数据 → 跳过；
    /// T2/预选（tierWeight=0）不计入累计（HLTV 忽略 T1 以下）。
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        current_year: i32,
        map: &MapScore,
        tier: TourneyTier,
        stage: SeriesStage,
        team_a_sig: &str,
        team_a_roster: &[&PlayerCharacter],
        team_b_sig: &str,
        team_b_roster: &[&PlayerCharacter],
    ) {
        if self.year == -1 {
            self.year = current_year;
        }
        let w = tier_weight(tier);
        let rounds = map.team_a_score + map.team_b_score;
        for line in &map.lines {
            // 按签名定位所属队伍的 roster，再按昵称找选手实体（仅用于反解稳定 ID）
            let roster = if line.team_sig == team_a_sig {
                team_a_roster
            } else {
                team_b_roster
            };
            let Some(pc) = roster.iter().find(|p| p.name == line.player_name) else {
                continue;
            };
            // **2026 修订**：NPC 也进年度累计——真实职业选手（donk/ZywOo 等）与
            // 虚拟新秀同榜竞争，TOP20 不再是主角独角戏（试玩发现主角连续 12 年
            // 无对手霸榜——因为榜上只有他一个人）。荣誉仍只归玩家（NPC 无生涯）。
            let pid = pc.id;
            let r =
                RatingCalculator::rating_of_counts(line.kills, line.deaths, line.assists, rounds);
            // 全赛事累计（不分层级——天才少年通配判定用）
            *self.rating_all.entry(pid).or_insert(0.0) += r;
            *self.maps_all.entry(pid).or_insert(0) += 1;
            // 击杀/死亡/胜场：**全赛事口径**（2026 试玩修复——此前在 T1+ 门槛
            // 之后累计，低级别赛事选手的生涯档案 kills/wins 恒 0，结局复盘
            // "累计 0 击杀"）。TOP20 的 T1+ 加权仍走下方分支，互不影响。
            *self.kills.entry(pid).or_insert(0) += line.kills;
            *self.deaths.entry(pid).or_insert(0) += line.deaths;
            *self.assists.entry(pid).or_insert(0) += line.assists;
            *self.rounds.entry(pid).or_insert(0) += rounds;
            let won = if line.team_sig == team_a_sig {
                map.winner_sig == team_a_sig
            } else {
                map.winner_sig == team_b_sig
            };
            if won {
                *self.wins.entry(pid).or_insert(0) += 1;
            }
            if w <= 0.0 {
                continue; // T2/预选不计入 T1+ TOP20 加权累计（但已计入全赛事与档案口径）
            }
            *self.rating.entry(pid).or_insert(0.0) += r * w;
            *self.weight.entry(pid).or_insert(0.0) += w;
            if stage.is_playoff() {
                *self.playoff_rating.entry(pid).or_insert(0.0) += r * w;
                *self.playoff_weight.entry(pid).or_insert(0.0) += w;
            }
            *self.maps.entry(pid).or_insert(0) += 1;
        }
    }

    /// 某玩家的淘汰赛加权场均 Rating（无淘汰赛样本 → None）。
    pub fn playoff_rating_of(&self, pid: PlayerId) -> Option<f64> {
        let r = self.playoff_rating.get(&pid)?;
        let w = self.playoff_weight.get(&pid).copied().unwrap_or(1.0);
        Some(r / w)
    }

    /// 全赛事统计快照（含 T2/预选；PlayerId → 算术场均 + 图数）。
    /// 供「天才少年通配」判定（低阶赛事高 Rating 的年轻选手）。
    pub fn stats_all(&self) -> HashMap<PlayerId, YearlyStatAll> {
        self.rating_all
            .iter()
            .map(|(pid, r)| {
                (
                    *pid,
                    YearlyStatAll {
                        rating: r / self.maps_all.get(pid).copied().unwrap_or(1) as f64,
                        maps: self.maps_all.get(pid).copied().unwrap_or(0),
                    },
                )
            })
            .collect()
    }

    /// 年度统计快照（PlayerId → 加权场均 / 图数 / 全赛事场均 / 击杀 / 死亡 / 胜场）。
    /// 供生涯档案归档在 `award` 清空**之前**采集。
    pub fn stats(&self) -> HashMap<PlayerId, YearlyStat> {
        // 键集 = rating ∪ rating_all（低级别赛事选手只有全赛事数据，也进档案）
        let mut pids: Vec<PlayerId> = self
            .rating
            .keys()
            .chain(self.rating_all.keys())
            .copied()
            .collect();
        pids.sort_unstable_by_key(|p| p.0);
        pids.dedup_by_key(|p| p.0);
        pids.into_iter()
            .map(|pid| {
                (
                    pid,
                    YearlyStat {
                        rating: self
                            .rating
                            .get(&pid)
                            .map(|r| r / self.weight.get(&pid).copied().unwrap_or(1.0))
                            .unwrap_or(0.0),
                        maps: self.maps.get(&pid).copied().unwrap_or(0),
                        all_rating: self
                            .rating_all
                            .get(&pid)
                            .map(|r| r / self.maps_all.get(&pid).copied().unwrap_or(1) as f64)
                            .unwrap_or(0.0),
                        maps_all: self.maps_all.get(&pid).copied().unwrap_or(0),
                        kills: self.kills.get(&pid).copied().unwrap_or(0),
                        deaths: self.deaths.get(&pid).copied().unwrap_or(0),
                        assists: self.assists.get(&pid).copied().unwrap_or(0),
                        rounds: self.rounds.get(&pid).copied().unwrap_or(0),
                        wins: self.wins.get(&pid).copied().unwrap_or(0),
                    },
                )
            })
            .collect()
    }

    /// 年度结算：按本年度**加权场均 Rating** 排名取前 top 名，**并清空累计器**。
    ///
    /// 加权场均而非算术场均——Major 好数据不被 T1 稀释。荣誉写入由调用方执行。
    /// @return (PlayerId, 加权场均 rating) 按名次降序；与累计年度不符时返回空
    pub fn award(&mut self, year: i32, top: usize) -> Vec<(PlayerId, f64)> {
        if year != self.year || self.rating.is_empty() {
            return Vec::new(); // 防错年颁发
        }
        let mut ranked: Vec<(PlayerId, f64)> = self
            .rating
            .iter()
            .map(|(pid, r)| (*pid, r / self.weight.get(pid).copied().unwrap_or(1.0)))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(top);
        self.clear();
        ranked
    }

    /// 记录一次赛事个人荣誉（MVP 或 EVP；**全员含 NPC**）：次数 + 按 tier 折算积分。
    ///
    /// @param points 荣誉积分（按 `Top20Evaluator::mvp_score/evp_score` 的 tier 矩阵）
    pub fn record_honour(&mut self, pid: PlayerId, is_mvp: bool, points: f64) {
        if is_mvp {
            *self.mvp_count.entry(pid).or_insert(0) += 1;
        } else {
            *self.evp_count.entry(pid).or_insert(0) += 1;
        }
        *self.honor_points.entry(pid).or_insert(0.0) += points;
    }

    /// 荣誉积分的量纲换算（v5 迁移用）：旧存档 MVP=10/EVP=5 绝对分
    /// → 新 Rating 等值单位（×0.1）。榜单历史只读展示，不回写。
    pub fn rescale_honour_points(&mut self, factor: f64) {
        for v in self.honor_points.values_mut() {
            *v *= factor;
        }
    }

    /// 该选手本年度的荣誉统计（MVP 次数, EVP 次数, 积分）；无记录 → None
    /// （调用方可用 career 兜底——旧存档 tracker 无此数据）。
    pub fn honour_of(&self, pid: PlayerId) -> Option<(i32, i32, f64)> {
        if !self.mvp_count.contains_key(&pid) && !self.evp_count.contains_key(&pid) {
            return None;
        }
        let mvp = self.mvp_count.get(&pid).copied().unwrap_or(0);
        let evp = self.evp_count.get(&pid).copied().unwrap_or(0);
        let points = self.honor_points.get(&pid).copied().unwrap_or(0.0);
        Some((mvp, evp, points))
    }

    /// 清空全部年度累计（结算后复位；供 Top20Evaluator 路径单独调用）。
    pub fn clear(&mut self) {
        self.rating.clear();
        self.weight.clear();
        self.playoff_rating.clear();
        self.playoff_weight.clear();
        self.maps.clear();
        self.kills.clear();
        self.deaths.clear();
        self.assists.clear();
        self.rounds.clear();
        self.wins.clear();
        self.rating_all.clear();
        self.maps_all.clear();
        self.mvp_count.clear();
        self.evp_count.clear();
        self.honor_points.clear();
        self.year = -1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::attributes::{
        BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
    };
    use csc_entities::role::Role;
    use csc_simulation::series::PlayerLine;

    fn player(id: u32, name: &str) -> PlayerCharacter {
        PlayerCharacter {
            id: csc_util::id::PlayerId(id),
            name: name.into(),
            age: 24,
            role: Role::Rifler,
            base: BaseAttributes {
                reaction: 80,
                stability: 80,
                endurance: 80,
                stamina: 80,
                health: 80,
            },
            skill: SkillAttributes {
                aim: 80,
                leader: 50,
                communication: 50,
                clutch: 70,
            },
            pro: ProAttributes {
                mentality: 70,
                confidence: 70,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: WeaponAttributes {
                position: Role::Rifler,
                ak: 80,
                awp: 50,
                pistol: 70,
                smoke: 50,
                utility: 50,
            },
            potential: 90,
            fatigue: 0.0,
            injury: None,
            career: Some(csc_entities::career::CareerInfo::free_agent_default(2026)),
            team: None,
            retired: false,
        }
    }

    fn map(winner: &str) -> MapScore {
        MapScore {
            map_number: 1,
            team_a_score: 13,
            team_b_score: 9,
            winner_sig: winner.to_string(),
            lines: vec![
                PlayerLine {
                    player_name: "P1".into(),
                    team_sig: "A|p".into(),
                    kills: 20,
                    deaths: 10,
                    assists: 5,
                    adr: 80.0,
                    kast: 75.0,
                },
                PlayerLine {
                    player_name: "P2".into(),
                    team_sig: "B|q".into(),
                    kills: 12,
                    deaths: 18,
                    assists: 3,
                    adr: 60.0,
                    kast: 60.0,
                },
            ],
        }
    }

    #[test]
    fn records_and_stats() {
        let mut t = YearlyRatingTracker::default();
        let p1 = player(1, "P1");
        let p2 = player(2, "P2");
        let m = map("A|p");
        t.record(
            2026,
            &m,
            TourneyTier::T1,
            SeriesStage::Group,
            "A|p",
            &[&p1],
            "B|q",
            &[&p2],
        );
        let stats = t.stats();
        assert_eq!(stats[&PlayerId(1)].maps, 1);
        assert_eq!(stats[&PlayerId(1)].kills, 20);
        assert_eq!(stats[&PlayerId(1)].wins, 1, "胜方玩家 +1 胜场");
        assert_eq!(stats[&PlayerId(2)].wins, 0);
    }

    #[test]
    fn award_ranks_by_avg_and_clears() {
        let mut t = YearlyRatingTracker::default();
        let p1 = player(1, "P1");
        let p2 = player(2, "P2");
        let m1 = map("A|p");
        t.record(
            2026,
            &m1,
            TourneyTier::T1,
            SeriesStage::Group,
            "A|p",
            &[&p1],
            "B|q",
            &[&p2],
        );
        let m2 = map("B|q"); // P2 赢一图
        t.record(
            2026,
            &m2,
            TourneyTier::T1,
            SeriesStage::Playoff,
            "A|p",
            &[&p1],
            "B|q",
            &[&p2],
        );
        let winners = t.award(2026, 20);
        assert_eq!(winners.len(), 2);
        assert!(winners[0].1 > winners[1].1);
        // 清空后 stats 为空
        assert!(t.stats().is_empty());
        // 错年返回空
        assert!(t.award(2027, 20).is_empty());
    }

    #[test]
    fn npc_also_tracked_for_competition() {
        // 2026 修订：NPC 也进年度累计（真实选手同榜竞争）；荣誉仍只归玩家
        let mut t = YearlyRatingTracker::default();
        let p1 = player(1, "P1");
        let mut npc = player(2, "P2");
        npc.career = None; // NPC
        let m = map("A|p");
        t.record(
            2026,
            &m,
            TourneyTier::T1,
            SeriesStage::Group,
            "A|p",
            &[&p1],
            "B|q",
            &[&npc],
        );
        assert_eq!(t.stats().len(), 2, "玩家与 NPC 都累计");
        assert!(t.stats().contains_key(&PlayerId(2)));
    }

    #[test]
    fn t2_and_qualify_track_all_rating_but_no_t1_weight() {
        let mut t = YearlyRatingTracker::default();
        let p1 = player(1, "P1");
        let p2 = player(2, "P2");
        let m = map("A|p");
        // T2 权重 0 → 不累计 T1+ 加权 rating，但击杀/全赛事数据进档案口径（2026 试玩修复）
        t.record(
            2026,
            &m,
            TourneyTier::T2,
            SeriesStage::Group,
            "A|p",
            &[&p1],
            "B|q",
            &[&p2],
        );
        let s = t.stats();
        assert!(!s.is_empty(), "T2 也进生涯档案（全赛事击杀/胜场）");
        assert_eq!(s[&PlayerId(1)].rating, 0.0, "T2 不计入 TOP20 加权 rating");
        assert!(
            s[&PlayerId(1)].all_rating > 0.0,
            "全赛事场均 rating 计入档案"
        );
        assert!(s[&PlayerId(1)].kills > 0, "T2 击杀计入生涯档案");
        assert_eq!(
            s[&PlayerId(1)].maps,
            0,
            "T2 不计入 TOP20 的 T1+ 图数（样本惩罚口径）"
        );
        assert_eq!(
            s[&PlayerId(1)].maps_all,
            1,
            "T2 计入全赛事图数（生涯档案口径，与击杀对齐）"
        );
        // 但全赛事累计（天才少年通配判定）保留
        let all = t.stats_all();
        assert_eq!(all[&PlayerId(1)].maps, 1, "全赛事图数含 T2/预选");
        assert!(all[&PlayerId(1)].rating > 0.0);
    }

    #[test]
    fn tier_weight_table_matches_hlvt() {
        assert_eq!(tier_weight(TourneyTier::Major), 1.50);
        assert_eq!(tier_weight(TourneyTier::SuperElite), 1.30);
        assert_eq!(tier_weight(TourneyTier::Elite), 1.15);
        assert_eq!(tier_weight(TourneyTier::T1), 1.00);
        assert_eq!(tier_weight(TourneyTier::T2), 0.0);
        assert_eq!(tier_weight(TourneyTier::Qualify), 0.0);
    }
}
