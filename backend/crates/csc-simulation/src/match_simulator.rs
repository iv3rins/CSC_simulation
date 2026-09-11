//! 战斗模拟器（Kotlin `MatchSimulator.kt` 转写）——把「一场比赛」从二元胜负
//! 下钻为完整系列赛（bo1/bo3/bo5）。
//!
//! 随机消耗顺序（跨语言可复现，与 Kotlin 逐位一致）：
//! 每图：`winRate`（self 每选手 1 gaussian → opp 每选手 1 gaussian）
//! → `scorePair`（1 nextDouble 加时判定；不进加时 1 nextDouble 定胜负；
//!   进加时每回合 1 nextDouble）→ `roster_lines`（A 每选手 1 gaussian →
//!   B 每选手 1 gaussian → 每击杀 2 nextDouble（杀手+受害者）→ 每助攻 1 nextDouble）。

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::character::PlayerCharacter;
use csc_util::id::TeamId;
use csc_util::rng::Xoshiro256StarStar;
use csc_util::{gaussian, pick_index, round_to_int};

use crate::kills_alloc::allocate_kills;

use crate::directives::MatchDirectives;
use crate::rating::RatingCalculator;
use crate::series::{MapScore, PlayerLine, SeriesResult, SeriesStage};
use crate::win_rate::WinRateCalculator;

/// 参赛队伍引用（= Kotlin `Team` 的 arena 形态：ID + 签名快照 + roster 引用）。
pub struct SeriesTeam<'a> {
    /// 队伍 ID
    pub id: TeamId,
    /// 队伍签名快照（供胜负判定/结算）
    pub signature: String,
    /// 阵容选手引用（由 World 组装）
    pub roster: Vec<&'a PlayerCharacter>,
    /// 关系网络凝聚力乘数（[`csc_systems::chemistry::ChemistryEngine::cohesion_factor_of`]
    /// 的组装时快照；1.0 = 中性——队友互动演化的团队氛围进入胜率）。
    pub cohesion: f64,
}

/// 队伍结构因子：角色多样性互补 × 属性磨合（团队精神/沟通/士气）× 关系网络凝聚力。
///
/// **2026 修订（比赛结果可信）**：此前胜率 = 5 名个体实力求和——「队伍 = 五个人
/// 相加」，实体层的 `PowerCalculator::team_power`（角色多样性/磨合因子版）无任何
/// 生产消费方，已在结构拆分时删除，本函数是队伍结构因子的**唯一实现**。
/// 现结构因子经**差异通道**进入胜率：位置重叠（角色重复）、沟通/团队精神差、
/// 队内关系恶劣 → 因子 <1 拖累；角色互补、磨合优秀 → >1 加成。
/// 1.0 = 中性（全同角色、磨合属性 150、关系中性）——双方同构时因子差为 0，
/// 与旧行为完全一致（跨语言 golden 不受影响）。
fn team_structure(team: &SeriesTeam) -> f64 {
    if team.roster.is_empty() {
        return 1.0; // 空队
    }
    // 角色多样性互补：种类越多，1 + 0.02·(k-1)
    let mut kinds = [false; 6];
    for pc in &team.roster {
        kinds[pc.role as usize] = true;
    }
    let role_kinds = kinds.iter().filter(|b| **b).count();
    let diversity = 1.0 + 0.02 * (role_kinds as f64 - 1.0);
    // 属性磨合：团队精神/沟通/士气的均值（150 中性）
    let cohesion_attr = team
        .roster
        .iter()
        .map(|pc| (pc.pro.team_spirit + pc.skill.communication + pc.pro.morale) as f64)
        .sum::<f64>()
        / team.roster.len() as f64;
    let attr_factor = 1.0 + (cohesion_attr - 150.0) / 1000.0;
    diversity * attr_factor * team.cohesion
}

/// 战斗模拟器——无副作用纯模拟器：个体击杀/死亡回写生涯属于结算层职责
/// （`csc-tournaments`），本模拟器只产出 [`SeriesResult`]。
pub struct MatchSimulator;

impl MatchSimulator {
    // —— 比分 / 击杀模型常量 ——

    /// MR12 制：先到 13 分获胜
    pub const MR12_WIN_SCORE: i32 = 13;
    /// MR4 加时：12-12 平后进入加时；每轮双方各打 8 回合，先到 4 回合者赢得该轮（即整图）
    pub const OVERTIME_TARGET: i32 = 4;
    pub const OVERTIME_ROUNDS: i32 = 8;
    /// 加时概率的二项式系数 C(24,12)
    pub const OVERTIME_COMB24_12: f64 = 2_704_156.0;
    /// 每回合双方总击杀基准（5v5，回合内击杀守恒）。
    ///
    /// **2026 IGL/Rating 生态校准**：此前 10.0 使全员平均 KPR≈1.0，把整条 Rating
    /// 尺度顶到 1.3+（真实 HLTV 均 Rating ≈ 1.0，TOP20 门槛 p5 ≈ 1.10）——IGL 即使
    /// 击杀减半仍能靠虚高的绝对 Rating 挤进 TOP20（10.25% vs 真实 ~2%）。真实 CS2
    /// 单回合总击杀 ≈ 6.5-7.0（全员平均 KPR ≈ 0.68-0.72），据此下调到 6.8，
    /// 让 Rating 整体回归 1.05-1.35 真实区间。
    pub const KILLS_PER_ROUND: f64 = 6.8;
    /// 赢队每回合平均击杀（输队 = KILLS_PER_ROUND − 该值 ≈ 3.3）
    pub const WIN_KILL_RATE: f64 = 3.5;
    /// 助攻 / 击杀比
    pub const ASSIST_RATE: f64 = 0.25;
    /// 败方得分下限 / 上限（13-x 中 x 的范围；x=12 触发加时，故上限 11）
    pub const LOSER_SCORE_MIN: i32 = 4;
    pub const LOSER_SCORE_MAX: i32 = 11;
    /// 分配权重下限（防实力极低选手权重为 0 拿不到击杀）
    pub const MIN_WEIGHT: f64 = 1.0;

    /// 12-12 平（进入加时）的概率：`C(24,12)·(p·(1-p))^12`。
    /// 势均力敌（p→0.5）概率最高（≈16%）；实力悬殊按指数衰减趋近 0。
    pub fn overtime_chance(win_prob_a: f64) -> f64 {
        Self::OVERTIME_COMB24_12 * (win_prob_a * (1.0 - win_prob_a)).powi(12)
    }

    /// 模拟一场系列赛（bo1 / bo3 / bo5），逐图产出比分与全员战绩。
    pub fn simulate_series(
        tier: TourneyTier,
        team_a: SeriesTeam,
        team_b: SeriesTeam,
        best_of: i32,
        rng: &mut Xoshiro256StarStar,
        stage: SeriesStage,
    ) -> SeriesResult {
        assert!(
            best_of == 1 || best_of == 3 || best_of == 5,
            "bestOf 仅支持 bo1/bo3/bo5（当前 {best_of}）"
        );

        let mut maps = Vec::new();
        let (mut a_wins, mut b_wins) = (0, 0);
        let target = best_of / 2 + 1; // 先赢 target 图者胜

        while a_wins < target && b_wins < target {
            let map = Self::simulate_map(
                maps.len() as i32 + 1,
                tier,
                &team_a,
                &team_b,
                rng,
                None,
                None,
            );
            if map.winner_sig == team_a.signature {
                a_wins += 1;
            } else {
                b_wins += 1;
            }
            maps.push(map);
        }

        let winner_sig = if a_wins > b_wins {
            team_a.signature.clone()
        } else {
            team_b.signature.clone()
        };
        let loser_sig = if a_wins > b_wins {
            team_b.signature.clone()
        } else {
            team_a.signature.clone()
        };
        SeriesResult {
            tier,
            team_a_id: team_a.id,
            team_b_id: team_b.id,
            team_a_sig: team_a.signature,
            team_b_sig: team_b.signature,
            best_of,
            maps,
            winner_sig,
            loser_sig,
            stage,
            replayable: true,
            live_feedback: Vec::new(),
            live_states: Vec::new(),
            outcome_analysis: None,
        }
    }

    /// 粗略模拟一场系列赛（LOD 粗略路径）：出胜负 + 逐图比分 + **10 人战绩**。
    /// 胜负判定与 `simulate_series` 同源（score_pair），VRS 结算只依赖胜负 → 积分不受影响。
    /// 2026 修订：也生成逐人战绩——TOP20 年度累计需要 NPC 的逐图 Rating
    /// （此前 quick 路径无 line 数据，TOP20 榜上只有主角 → 竞争真空）。
    pub fn simulate_quick_series(
        tier: TourneyTier,
        team_a: SeriesTeam,
        team_b: SeriesTeam,
        best_of: i32,
        rng: &mut Xoshiro256StarStar,
        stage: SeriesStage,
    ) -> SeriesResult {
        assert!(
            best_of == 1 || best_of == 3 || best_of == 5,
            "bestOf 仅支持 bo1/bo3/bo5（当前 {best_of}）"
        );

        let mut maps = Vec::new();
        let (mut a_wins, mut b_wins) = (0, 0);
        let target = best_of / 2 + 1;

        while a_wins < target && b_wins < target {
            // 每图重新采样双方单场发挥（与精确路径同源）+ 逐人战绩（TOP20 数据源）
            let win_prob_a = WinRateCalculator::win_rate(
                &team_a.roster,
                &team_b.roster,
                tier,
                rng,
                None,
                None,
                team_structure(&team_a),
                team_structure(&team_b),
            );
            let (score_a, score_b) = Self::score_pair(win_prob_a, rng);
            let winner_sig = if score_a > score_b {
                team_a.signature.clone()
            } else {
                team_b.signature.clone()
            };
            let lines = Self::roster_lines(
                &team_a.roster,
                &team_b.roster,
                &team_a.signature,
                &team_b.signature,
                score_a,
                score_b,
                rng,
            );
            maps.push(MapScore {
                map_number: maps.len() as i32 + 1,
                team_a_score: score_a,
                team_b_score: score_b,
                winner_sig: winner_sig.clone(),
                lines,
            });
            if winner_sig == team_a.signature {
                a_wins += 1;
            } else {
                b_wins += 1;
            }
        }

        let winner_sig = if a_wins > b_wins {
            team_a.signature.clone()
        } else {
            team_b.signature.clone()
        };
        let loser_sig = if a_wins > b_wins {
            team_b.signature.clone()
        } else {
            team_a.signature.clone()
        };
        SeriesResult {
            tier,
            team_a_id: team_a.id,
            team_b_id: team_b.id,
            team_a_sig: team_a.signature,
            team_b_sig: team_b.signature,
            best_of,
            maps,
            winner_sig,
            loser_sig,
            stage,
            replayable: false, // 纯 NPC 后台粗 tick：不参与玩家回放
            live_feedback: Vec::new(),
            live_states: Vec::new(),
            outcome_analysis: None,
        }
    }

    /// 模拟一张地图：胜率（含场内指令/体况）→ 比分 → 10 人战绩。
    /// 公开供赛事引擎的**图间决策循环**逐图调用。
    pub fn simulate_map(
        map_number: i32,
        tier: TourneyTier,
        team_a: &SeriesTeam,
        team_b: &SeriesTeam,
        rng: &mut Xoshiro256StarStar,
        directives_a: Option<MatchDirectives>,
        directives_b: Option<MatchDirectives>,
    ) -> MapScore {
        // 每图重新采样双方单场发挥（同一系列赛各图表现独立）
        let win_prob_a = WinRateCalculator::win_rate(
            &team_a.roster,
            &team_b.roster,
            tier,
            rng,
            directives_a,
            directives_b,
            team_structure(team_a),
            team_structure(team_b),
        );
        let (score_a, score_b) = Self::score_pair(win_prob_a, rng);
        // 胜者由最终比分推导 → 比分与胜者永不矛盾（含加时翻盘情形）
        let winner_sig = if score_a > score_b {
            team_a.signature.clone()
        } else {
            team_b.signature.clone()
        };
        let lines = Self::roster_lines(
            &team_a.roster,
            &team_b.roster,
            &team_a.signature,
            &team_b.signature,
            score_a,
            score_b,
            rng,
        );
        MapScore {
            map_number,
            team_a_score: score_a,
            team_b_score: score_b,
            winner_sig,
            lines,
        }
    }

    /// 比分模型：MR12 十三胜制（= Kotlin `scorePair`）。
    ///
    /// - 进入加时的概率由双方实力接近度决定（`overtime_chance`）；
    /// - MR4 加时（`simulate_overtime`），加时胜者即比分胜者；
    /// - 常规时间：先掷骰定胜负，败方得分由「胜方胜率」单调收敛。
    fn score_pair(win_prob_a: f64, rng: &mut Xoshiro256StarStar) -> (i32, i32) {
        if rng.roll_bp(Self::overtime_chance(win_prob_a)) {
            // 12-12 平 → MR4 加时；加时段比分谁大谁赢整图
            let (ot_a, ot_b) = Self::simulate_overtime(win_prob_a, rng);
            return (
                Self::MR12_WIN_SCORE - 1 + ot_a,
                Self::MR12_WIN_SCORE - 1 + ot_b,
            );
        }
        let a_wins = rng.roll_bp(win_prob_a);
        let winner_prob = if a_wins { win_prob_a } else { 1.0 - win_prob_a }; // 败方得分以胜方胜率为基准（对称）
        let loser_score = round_to_int(11.5 - 15.0 * (winner_prob - 0.5))
            .clamp(Self::LOSER_SCORE_MIN, Self::LOSER_SCORE_MAX);
        if a_wins {
            (Self::MR12_WIN_SCORE, loser_score)
        } else {
            (loser_score, Self::MR12_WIN_SCORE)
        }
    }

    /// MR4 加时：从 12-12 起，双方各打 8 回合封顶，先到 4 回合的一方赢得加时；
    /// 一轮打满仍平（4-4）则继续下一轮。每回合以 `win_prob_a` 为 A 队胜率。
    fn simulate_overtime(win_prob_a: f64, rng: &mut Xoshiro256StarStar) -> (i32, i32) {
        loop {
            let (mut a, mut b) = (0, 0);
            // 一轮加时：先到 4 回合即止，且不超过 8 回合（双方各 4 回合）
            while a < Self::OVERTIME_TARGET
                && b < Self::OVERTIME_TARGET
                && a + b < Self::OVERTIME_ROUNDS
            {
                if rng.roll_bp(win_prob_a) {
                    a += 1;
                } else {
                    b += 1;
                }
            }
            if a != b {
                return (a, b);
            }
            // a == b == 4：本轮平，再来一轮
        }
    }

    /// 为双方 10 人生成一张地图的 K/D/A（= Kotlin `rosterLines`）。
    ///
    /// 守恒约束：双方总击杀 ≈ 总回合 × 10；赢队总击杀 = 输队总死亡；
    /// 助攻 ≈ 本队击杀 × 0.25。击杀按「事件」采样（杀手 + 受害者独立轮盘）。
    fn roster_lines(
        team_a: &[&PlayerCharacter],
        team_b: &[&PlayerCharacter],
        team_a_sig: &str,
        team_b_sig: &str,
        score_a: i32,
        score_b: i32,
        rng: &mut Xoshiro256StarStar,
    ) -> Vec<PlayerLine> {
        let total_rounds = score_a + score_b;
        let a_wins = score_a > score_b;
        let win_kills = round_to_int(total_rounds as f64 * Self::WIN_KILL_RATE); // 赢队总击杀
        let lose_kills = round_to_int(total_rounds as f64 * Self::KILLS_PER_ROUND) - win_kills; // 输队总击杀（守恒）

        let a_weights = Self::weights_of(team_a, rng); // 每人单场发挥权重
        let b_weights = Self::weights_of(team_b, rng);
        // 死亡分布**均匀化**（2026 复审校准）：击杀按枪械权重分配（强者多杀），
        // 但死亡若也按同权重分配，强选手 DPR 反被推高、Rating 被系统性压低
        // （实测 TOP20 榜首 1.19 vs 真实 1.3+，且榜单区分度不足）。
        // 真实 HLTV：顶尖选手 DPR 0.55~0.65，与均值差距远小于击杀差距——
        // 死亡近似均匀（每回合 5 人分摊）。
        let uniform_victims = vec![1.0; 5];

        // 击杀事件双向配对：A 杀 B ⇔ A 的击杀 / B 的死亡
        let (a_kills, b_deaths) = Self::kill_exchange(
            &a_weights,
            &uniform_victims,
            if a_wins { win_kills } else { lose_kills },
            rng,
        );
        let (b_kills, a_deaths) = Self::kill_exchange(
            &b_weights,
            &uniform_victims,
            if a_wins { lose_kills } else { win_kills },
            rng,
        );

        // 助攻基数 = 本队实际击杀数（随胜负对换）
        let a_assists = allocate_kills(
            round_to_int(a_kills.iter().sum::<i32>() as f64 * Self::ASSIST_RATE),
            &a_weights,
            rng,
        );
        let b_assists = allocate_kills(
            round_to_int(b_kills.iter().sum::<i32>() as f64 * Self::ASSIST_RATE),
            &b_weights,
            rng,
        );

        // 先 A 队 5 人，后 B 队 5 人（含每图 ADR/KAST 估值）
        Self::lines_of(
            team_a,
            team_a_sig,
            &a_kills,
            &a_deaths,
            &a_assists,
            total_rounds,
        )
        .into_iter()
        .chain(Self::lines_of(
            team_b,
            team_b_sig,
            &b_kills,
            &b_deaths,
            &b_assists,
            total_rounds,
        ))
        .collect()
    }

    /// 双向击杀事件：模拟 total 次击杀，每个事件独立采样杀手（攻方轮盘）与受害者（守方轮盘）。
    /// 攻方击杀分布与守方死亡分布逐事件对应，总数精确守恒。
    fn kill_exchange(
        attacker_weights: &[f64],
        victim_weights: &[f64],
        total: i32,
        rng: &mut Xoshiro256StarStar,
    ) -> (Vec<i32>, Vec<i32>) {
        let mut kills = vec![0; attacker_weights.len()];
        let mut deaths = vec![0; victim_weights.len()];
        if total <= 0 || attacker_weights.is_empty() || victim_weights.is_empty() {
            return (kills, deaths);
        }
        let attacker_sum: f64 = attacker_weights.iter().sum();
        let victim_sum: f64 = victim_weights.iter().sum();
        for _ in 0..total {
            kills[pick_index(attacker_weights, attacker_sum, rng)] += 1;
            deaths[pick_index(victim_weights, victim_sum, rng)] += 1;
        }
        (kills, deaths)
    }

    /// 把逐人击杀/死亡/助攻打包成 PlayerLine（roster 顺序），并按图内 K/D/A 估值 ADR/KAST。
    fn lines_of(
        roster: &[&PlayerCharacter],
        team_sig: &str,
        kills: &[i32],
        deaths: &[i32],
        assists: &[i32],
        total_rounds: i32,
    ) -> Vec<PlayerLine> {
        roster
            .iter()
            .enumerate()
            .map(|(i, pc)| {
                let kpr = kills[i] as f64 / total_rounds as f64;
                let dpr = deaths[i] as f64 / total_rounds as f64;
                let apr = assists[i] as f64 / total_rounds as f64;
                PlayerLine {
                    player_name: pc.name.clone(),
                    team_sig: team_sig.to_string(),
                    kills: kills[i],
                    deaths: deaths[i],
                    assists: assists[i],
                    adr: RatingCalculator::adr_of(kpr, apr, dpr),
                    kast: RatingCalculator::kast_of(kpr, dpr, apr),
                }
            })
            .collect()
    }

    /// 每人的分配权重 = **单场枪械发挥**（2026 复审校准：击杀分布回归真实量级）。
    ///
    /// 旧实现用 aim⁸ 高次幂放大：顶尖枪男 (aim 95) 与普通选手 (aim 75) 击杀权重
    /// 相差 ~15 倍，单场 KPR 被推到 1.2~1.5、年度 rating 1.5+（TOP20 数据膨胀，
    /// 用户实测 TOP13 显示 1.52，真实 2025 榜首 donk 年度 rating 1.41 已破纪录）。
    /// 真实击杀比：donk 0.94 历史级、顶尖 ~0.85、普通 ~0.65、IGL ~0.55，枪男/IGL
    /// 权重比约 1.4~1.7:1。改用**线性枪械组合**（aim 主导 0.55 + ak 0.25 + awp 0.20，
    /// 无道具加成——道具是辅助行为不直接产生击杀，单场波动 `g*4.0` 与 RNG 消费
    /// 次序不变，每人一次 gaussian）。
    fn weights_of(roster: &[&PlayerCharacter], rng: &mut Xoshiro256StarStar) -> Vec<f64> {
        if roster.is_empty() {
            return Vec::new();
        }
        roster
            .iter()
            .map(|pc| {
                let g = gaussian(rng); // 单场发挥波动（与原实现一致）
                let aim = pc.skill.aim as f64 / 100.0;
                let ak = pc.weapon.ak as f64 / 100.0;
                let awp = pc.weapon.awp as f64 / 100.0;
                let gun = (aim * 0.55 + ak * 0.25 + awp * 0.20) * 120.0;
                (gun + g * 4.0).max(Self::MIN_WEIGHT)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::attributes::{
        BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
    };
    use csc_entities::role::Role;

    fn squad(name: &str, base_power: i32) -> Vec<PlayerCharacter> {
        (0..5)
            .map(|i| PlayerCharacter {
                id: csc_util::id::PlayerId(i),
                name: format!("{name}{i}"),
                age: 24,
                role: Role::Rifler,
                base: BaseAttributes {
                    reaction: base_power,
                    stability: base_power,
                    endurance: base_power,
                    stamina: base_power,
                    health: base_power,
                },
                skill: SkillAttributes {
                    aim: base_power,
                    leader: base_power,
                    communication: base_power,
                    clutch: base_power,
                },
                pro: ProAttributes {
                    mentality: base_power,
                    confidence: base_power,
                    team_spirit: base_power,
                    loyalty: base_power,
                    morale: base_power,
                },
                weapon: WeaponAttributes {
                    position: Role::Rifler,
                    ak: base_power,
                    awp: base_power,
                    pistol: base_power,
                    smoke: base_power,
                    utility: base_power,
                },
                potential: 100,
                fatigue: 0.0,
                injury: None,
                career: None,
                team: None,
                retired: false,
            })
            .collect()
    }

    fn build<'a>(
        id: u32,
        name: &str,
        power: i32,
        pool: &'a mut Vec<PlayerCharacter>,
    ) -> SeriesTeam<'a> {
        pool.clear();
        *pool = squad(name, power);
        SeriesTeam {
            id: TeamId(id),
            signature: format!("{name}|{name}0,{name}1,{name}2,{name}3,{name}4"),
            roster: pool.iter().collect(),
            cohesion: 1.0,
        }
    }

    #[test]
    fn overtime_chance_peak_at_even() {
        // p=0.5 → C(24,12)·(0.25)^12 ≈ 0.1611
        let p = MatchSimulator::overtime_chance(0.5);
        assert!((p - 0.1611).abs() < 0.001, "势均力敌加时概率应≈16%: {p}");
        // 悬殊 → 趋近 0
        assert!(MatchSimulator::overtime_chance(0.95) < 1e-6);
        assert!(MatchSimulator::overtime_chance(0.5) > MatchSimulator::overtime_chance(0.7));
    }

    #[test]
    fn bo1_series_result() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let mut pool_a = Vec::new();
        let mut pool_b = Vec::new();
        let ta = build(0, "A", 85, &mut pool_a);
        let tb = build(1, "B", 75, &mut pool_b);
        let r = MatchSimulator::simulate_series(
            TourneyTier::T1,
            ta,
            tb,
            1,
            &mut rng,
            SeriesStage::Group,
        );
        assert_eq!(r.maps.len(), 1);
        assert!(
            r.maps[0].team_a_score == 13 || r.maps[0].team_b_score == 13,
            "MR12 胜者 13 分"
        );
        assert_ne!(r.maps[0].team_a_score, r.maps[0].team_b_score);
        assert!(r.winner_sig == "A|A0,A1,A2,A3,A4" || r.winner_sig == "B|B0,B1,B2,B3,B4");
        assert_eq!(
            r.loser_sig,
            if r.winner_sig == "A|A0,A1,A2,A3,A4" {
                "B|B0,B1,B2,B3,B4"
            } else {
                "A|A0,A1,A2,A3,A4"
            }
        );
    }

    #[test]
    fn kills_conservation() {
        let mut rng = Xoshiro256StarStar::seed(7);
        for _ in 0..20 {
            let mut pool_a = Vec::new();
            let mut pool_b = Vec::new();
            let ta = build(0, "A", 80, &mut pool_a);
            let tb = build(1, "B", 80, &mut pool_b);
            let r = MatchSimulator::simulate_series(
                TourneyTier::T1,
                ta,
                tb,
                1,
                &mut rng,
                SeriesStage::Group,
            );
            let map = &r.maps[0];
            let total_rounds = map.team_a_score + map.team_b_score;
            let a_lines: Vec<&PlayerLine> = map
                .lines
                .iter()
                .filter(|l| l.team_sig.starts_with("A"))
                .collect();
            let b_lines: Vec<&PlayerLine> = map
                .lines
                .iter()
                .filter(|l| l.team_sig.starts_with("B"))
                .collect();
            let a_kills: i32 = a_lines.iter().map(|l| l.kills).sum();
            let b_kills: i32 = b_lines.iter().map(|l| l.kills).sum();
            let a_deaths: i32 = a_lines.iter().map(|l| l.deaths).sum();
            let b_deaths: i32 = b_lines.iter().map(|l| l.deaths).sum();
            // 守恒：总击杀 ≈ 回合 × 10（round 取整误差 ≤ 1）
            let expected = (total_rounds as f64 * MatchSimulator::KILLS_PER_ROUND).round() as i32;
            assert!(
                (a_kills + b_kills - expected).abs() <= 1,
                "总击杀守恒: {a_kills}+{b_kills} vs {expected}"
            );
            // 赢队击杀 = 输队死亡（逐队守恒）
            assert_eq!(a_kills, b_deaths, "A 击杀 = B 死亡");
            assert_eq!(b_kills, a_deaths, "B 击杀 = A 死亡");
            // 10 人
            assert_eq!(map.lines.len(), 10);
        }
    }

    #[test]
    fn overtime_scores_above_13() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let mut found_ot = false;
        for _ in 0..200 {
            let mut pool_a = Vec::new();
            let mut pool_b = Vec::new();
            let ta = build(0, "A", 80, &mut pool_a);
            let tb = build(1, "B", 80, &mut pool_b);
            let r = MatchSimulator::simulate_series(
                TourneyTier::T1,
                ta,
                tb,
                1,
                &mut rng,
                SeriesStage::Group,
            );
            let map = &r.maps[0];
            if map.team_a_score > 13 || map.team_b_score > 13 {
                found_ot = true;
                // 加时比分：12 + ot，ot ∈ 4..8
                let (hi, lo) = if map.team_a_score > map.team_b_score {
                    (map.team_a_score, map.team_b_score)
                } else {
                    (map.team_b_score, map.team_a_score)
                };
                // 加时比分 = 12 + 当轮 ot 得分：胜方 16..20、败方 12..18（败方 ot≥0，
                // 4:0 横扫时败方 ot=0 → 12；旧断言 `lo>=13` 过严，2026 校准后 RNG
                // 序列可能命中横扫，改为正确不变量 lo>=12 且败方低于胜方）
                assert!((16..=20).contains(&hi), "加时比分: {hi}");
                assert!(lo >= 12 && lo < hi, "败方加时得分 12+ot: {lo}");
                break;
            }
        }
        assert!(found_ot, "200 场应至少出现一次加时（p≈16% 时几乎必然）");
    }

    #[test]
    fn quick_series_has_lines_for_top20() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let mut pool_a = Vec::new();
        let mut pool_b = Vec::new();
        let ta = build(0, "A", 85, &mut pool_a);
        let tb = build(1, "B", 75, &mut pool_b);
        let r = MatchSimulator::simulate_quick_series(
            TourneyTier::T1,
            ta,
            tb,
            3,
            &mut rng,
            SeriesStage::Group,
        );
        assert!(
            r.maps.len() == 2 || r.maps.len() == 3,
            "bo3 可 2:0 或 2:1: {}",
            r.maps.len()
        );
        assert_eq!(r.winner_sig, "A|A0,A1,A2,A3,A4", "85 队应胜 75 队");
        for m in &r.maps {
            assert_eq!(
                m.lines.len(),
                10,
                "2026 修订：quick 路径也生成逐人战绩（TOP20 竞争数据源）"
            );
            assert!(m.team_a_score == 13 || m.team_b_score == 13);
        }
    }

    /// 跨语言 golden：Kotlin `MatchSimulator.simulateMap` 权威输出
    /// （A 队 5×RIFLER 全 80 vs B 队 5×RIFLER 全 70、T1、seed=42；tools/gen_sim_golden.kt）。
    ///
    /// **2026 复审校准更新**：击杀权重 aim⁸ → 线性枪械组合后，队内全员属性相同
    /// 时权重等比（公式单调）→ 分配比例不变，仅 A0 的击杀随轮盘方差微移（20→21）。
    #[test]
    fn golden_simulate_map() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let mut pool_a = Vec::new();
        let mut pool_b = Vec::new();
        let ta = build(0, "A", 80, &mut pool_a);
        let tb = build(1, "B", 70, &mut pool_b);
        let map = MatchSimulator::simulate_map(1, TourneyTier::T1, &ta, &tb, &mut rng, None, None);
        assert_eq!((map.team_a_score, map.team_b_score), (13, 7));
        assert_eq!(map.winner_sig, "A|A0,A1,A2,A3,A4");
        let l0 = &map.lines[0];
        assert_eq!(
            (l0.player_name.as_str(), l0.kills, l0.deaths, l0.assists),
            ("A0", 21, 17, 4)
        );
        assert_eq!(map.lines.len(), 10);
    }

    /// 回归（2026 试玩）：击杀分配按枪械输出而非综合 power——
    /// IGL 枪械差（aim/ak 低）击杀权重应显著低于同队步枪手。
    #[test]
    fn igl_kill_weight_lower_than_rifler() {
        use csc_entities::attributes::{
            BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
        };
        let mk =
            |role: Role, aim: i32, ak: i32, awp: i32, leader: i32, util: i32| PlayerCharacter {
                id: csc_util::id::PlayerId(0),
                name: format!("{role:?}"),
                age: 24,
                role,
                base: BaseAttributes {
                    reaction: 60,
                    stability: 60,
                    endurance: 60,
                    stamina: 60,
                    health: 60,
                },
                skill: SkillAttributes {
                    aim,
                    leader,
                    communication: 60,
                    clutch: aim,
                },
                pro: ProAttributes {
                    mentality: 70,
                    confidence: 60,
                    team_spirit: 60,
                    loyalty: 50,
                    morale: 60,
                },
                weapon: WeaponAttributes {
                    position: role,
                    ak,
                    awp,
                    pistol: 60,
                    smoke: util,
                    utility: util,
                },
                potential: 90,
                fatigue: 0.0,
                injury: None,
                career: None,
                team: None,
                retired: false,
            };
        let igl = mk(Role::Igl, 40, 40, 40, 95, 95); // 指挥：枪械差、指挥/道具高
        let rifler = mk(Role::Rifler, 90, 90, 90, 40, 50); // 步枪：枪械强
        let roster = vec![&igl, &rifler];
        let mut rng = Xoshiro256StarStar::seed(42);
        let w = MatchSimulator::weights_of(&roster, &mut rng);
        assert!(
            w[1] > w[0],
            "步枪手击杀权重应高于 IGL（枪械输出主导）: IGL={:.1} Rifler={:.1}",
            w[0],
            w[1]
        );
    }

    /// 跨语言 golden：overtimeChance(0.5) 位模式。
    #[test]
    fn golden_overtime_chance() {
        assert_eq!(
            MatchSimulator::overtime_chance(0.5).to_bits(),
            0x3fc4a18e00000000
        );
        // 约 16.11%（C(24,12)·0.25^12）
        assert!((MatchSimulator::overtime_chance(0.5) - 0.1611).abs() < 0.001);
    }
}
