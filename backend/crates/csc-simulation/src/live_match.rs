//! # LIVE 逐回合确定性引擎（Phase 4a 纯引擎层）
//!
//! 目标（`docs/LIVE-ENGINE-PLAN.md` §1/§3）：比赛未来回合在玩家决策前**不存在**最终结果。
//! 本模块只做**逐回合生成**——每次调用 [`LiveMatchEngine::simulate_next_round`] 恰好
//! 生成一个回合事件；只有推进到 [`LiveMatchPhase::MapEnd`] 后 `finish_map` 才产出
//! [`MapScore`]。这与旧的 `MatchSimulator::simulate_map`（一次性预生成整图比分）是
//! **两个不同引擎**：旧引擎保留给 NPC quick 路径与旧回放（Phase 4b 接线时并存）。
//!
//! 确定性铁律（§3.2）：每个回合的 RNG 由 `round_seed(world_seed, match_id, map_id,
//! round_number, decision_seq, SIMULATION_VERSION)` 派生，**玩家决策进入 seed 链**——
//! 同一存档 + 同一决策序列必然重现同一比赛；不同决策必然在 seed 与状态上产生分歧。
//!
//! 本文件是 Phase 4a 的**纯引擎**：不接线 conductor/server/展示层（那是 Phase 4b/4c）。

use serde::{Deserialize, Serialize};

use csc_domain::live::{
    BombSite, EconomyBuy, LiveEconomy, LiveMatchPhase, Pace, Side, UtilityInventory, UtilityPlan,
    WeaponClass,
};
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;
use csc_util::seed_chain::round_seed;
use csc_util::{pick_index, round_to_int};

use crate::kills_alloc::allocate_kills;

use crate::match_simulator::MatchSimulator;
use crate::rating::RatingCalculator;
use crate::series::{MapScore, PlayerLine};

/// LIVE 引擎模拟器版本号。变更后新 LIVE 使用新 seed 链，旧回放仍可读（§7.4）。
pub const SIMULATION_VERSION: u32 = 1;
/// MR12 十三胜制（与 `MatchSimulator::MR12_WIN_SCORE` 对齐）。
pub const MR12_WIN_SCORE: i32 = 13;
/// 半场回合数（12 回合后换边）。
pub const HALFTIME_ROUNDS: i32 = 12;
/// 分配权重下限（防 confidence 极低选手权重为 0 拿不到击杀）。
pub const MIN_WEIGHT: f64 = 1.0;
/// 回合击杀流的派生 seed 域标签（与 `round_seed` 的 `csc/live/round/v1` 区分，
/// 保证击杀流的 RNG 消费绝不干扰回合胜负/比分 seed 链）。
pub const KILL_FEED_DOMAIN: &str = "csc/live/killfeed/v1";

/// 一场地图的 K/D/A 分队伍分配结果（kills/deaths/assists × A/B 队）。
#[derive(Debug, Clone)]
struct KdaSplit {
    kills_a: Vec<i32>,
    deaths_a: Vec<i32>,
    assists_a: Vec<i32>,
    kills_b: Vec<i32>,
    deaths_b: Vec<i32>,
    assists_b: Vec<i32>,
}

/// 玩家在 LIVE 逐回合引擎里的运行时状态（含健康/护甲/武器/道具/战绩）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LivePlayerState {
    pub player_id: PlayerId,
    pub player_name: String,
    pub team_id: TeamId,
    pub alive: bool,
    pub health: u16,
    pub armor: u16,
    pub weapon: WeaponClass,
    pub utility: UtilityInventory,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub confidence: f64,
}

/// 战术/士气解释层状态（决策与回合概率的叙事输入）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LiveTacticalState {
    pub momentum: f64,
    pub confidence: f64,
    pub morale: f64,
    pub round_stability: f64,
    pub economy_pressure: f64,
    pub pace: Pace,
    pub timeout_called: bool,
}

impl LiveTacticalState {
    /// 战术状态对 A 队胜率的相对加成（小量，仅作叙事偏移）。
    fn win_adjust(self) -> f64 {
        ((self.momentum - 50.0) * 0.004
            + (self.confidence - 50.0) * 0.003
            + (self.morale - 50.0) * 0.002
            + (self.round_stability - 50.0) * 0.002
            - self.economy_pressure * 0.002)
            .clamp(-0.10, 0.10)
    }
}

/// 炸弹状态（Carried 携带者、Planted 安放点/剩余 tick；简化，不模拟轨迹）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BombState {
    None,
    Carried { player: PlayerId },
    Planted { site: BombSite, ticks_left: u16 },
    Defused,
    Exploded,
}

/// 单场 LIVE 的运行时事实源（可序列化存档/恢复，§7.3）。
///
/// `round_number` 语义：**下一回合编号**（比赛开始 = 1，`simulate_next_round` 消费后递增）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRoundState {
    pub match_id: String,
    pub series_id: String,
    pub map_id: String,
    pub map_number: i32,
    pub phase: LiveMatchPhase,
    pub team_a_id: TeamId,
    pub team_b_id: TeamId,
    pub team_a_sig: String,
    pub team_b_sig: String,
    pub best_of: i32,
    pub world_seed: u64,
    /// 下一回合编号（从 1 开始）。
    pub round_number: i32,
    /// 已提交决策的单调计数。
    pub decision_seq: u32,
    /// 已生成回合的单调计数（存档安全边界佐证）。
    pub rng_counter: u64,
    pub score_a: i32,
    pub score_b: i32,
    /// 双方基础胜率（A 视角；由编排层按阵容实力提供）。
    pub base_win_prob_a: f64,
    pub economy_a: LiveEconomy,
    pub economy_b: LiveEconomy,
    pub tactical_a: LiveTacticalState,
    pub tactical_b: LiveTacticalState,
    pub players_a: Vec<LivePlayerState>,
    pub players_b: Vec<LivePlayerState>,
    /// 已发生的回合事件（只含已完成回合，未来回合绝不预写）。
    pub round_history: Vec<LiveRoundEvent>,
}

/// `start_map` 输入：编排层提供的确定性比赛配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveMapConfig {
    pub match_id: String,
    pub series_id: String,
    pub map_id: String,
    pub map_number: i32,
    pub team_a_id: TeamId,
    pub team_b_id: TeamId,
    pub team_a_sig: String,
    pub team_b_sig: String,
    pub best_of: i32,
    pub world_seed: u64,
    /// A 队基础胜率（由编排层按 `WinRateCalculator` 阵容实力提供）。
    pub base_win_prob_a: f64,
    pub team_a_players: Vec<LivePlayerSlot>,
    pub team_b_players: Vec<LivePlayerSlot>,
}

/// 阵容占位（id + 昵称；Phase 4b 接线真实阵容）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LivePlayerSlot {
    pub id: PlayerId,
    pub name: String,
}

/// 玩家在单个回合前提交的决策（进入 seed 链）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LiveRoundDecision {
    None,
    CallTimeout,
    ChangePace(Pace),
    AggressiveOpening,
    PlayForTrade,
    Save,
    ForceBuy,
    ChangeUtilityPlan(UtilityPlan),
    Encourage(PlayerId),
    Criticize(PlayerId),
}

/// 回合事件类型（一次只发生一个，事件是刚刚生成的、不是未来列表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiveRoundEventKind {
    RoundPlayed,
    Halftime,
    MapEnd,
}

/// 单次击杀事件（实时击杀可视化下发的最小单元）。
///
/// 确定性：由回合派生 seed（`round_seed` 域分隔 `KILL_FEED_DOMAIN`）独立生成，
/// **不消费**回合胜负 RNG 序列——既不改历史回合的 winner/score，也让同一存档
/// 同一决策序列产出同一击杀流。击杀总数与 `finish_map` 的守恒口径同源
/// （回合级 ≈ KILLS_PER_ROUND，跨回合守恒到整图 KDA）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveKillEvent {
    /// 击杀方队伍（0 = A 队 / 1 = B 队）。
    pub team: u8,
    /// 击杀者（含昵称）。
    pub killer: LivePlayerRef,
    /// 受害者（含昵称）。
    pub victim: LivePlayerRef,
    /// 击杀所用武器大类（叙事层，随 killer 当前武器）。
    pub weapon: WeaponClass,
    /// 是否首杀（回合第一杀）。
    pub first_kill: bool,
}

/// 击杀事件里的选手身份引用（id + 昵称；不携带运行时健康/经济）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivePlayerRef {
    pub id: PlayerId,
    pub name: String,
    /// 队伍方（0 = A / 1 = B）。
    pub team: u8,
}

/// 单个回合的公开事件（客户端只收到当前回合事件 + 更新后的可见状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRoundEvent {
    /// 已完成回合编号。
    pub round_number: i32,
    /// 事件发生后的阶段。
    pub phase: LiveMatchPhase,
    pub event_kind: LiveRoundEventKind,
    pub winner_a: bool,
    pub score_a: i32,
    pub score_b: i32,
    pub side_a: Side,
    pub side_b: Side,
    /// 本回合的击杀事件流（实时击杀可视化；serde default 空数组向后兼容旧回合）。
    #[serde(default)]
    pub kills: Vec<LiveKillEvent>,
}

/// 单个回合的比分/半场快照（§2.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRoundScore {
    pub score_a: i32,
    pub score_b: i32,
    pub round_number: i32,
    pub half: u8,
    pub side_a: Side,
    pub side_b: Side,
    pub rounds_in_current_half: i32,
}

/// `simulate_next_round` 的输出（状态原地变更，output 只含事件与转换标记）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRoundOutput {
    pub event: LiveRoundEvent,
    pub map_finished: bool,
    /// Phase 4a 为单图引擎：series 由编排层处理，恒为 false。
    pub series_finished: bool,
}

/// LIVE 逐回合确定性引擎（无副作用纯引擎，关联函数均以 `&mut LiveRoundState` 为状态）。
pub struct LiveMatchEngine;

impl LiveMatchEngine {
    /// 地图是否已结束（服务端会话层可在调用推进前安全检查）。
    pub fn is_finished(state: &LiveRoundState) -> bool {
        matches!(
            state.phase,
            LiveMatchPhase::MapEnd | LiveMatchPhase::Completed
        )
    }

    /// 创建一张地图的初始运行时状态（阶段 = Warmup，未来回合结果不存在）。
    pub fn start_map(config: LiveMapConfig) -> LiveRoundState {
        let players_a = Self::init_players(&config, config.team_a_id, &config.team_a_players);
        let players_b = Self::init_players(&config, config.team_b_id, &config.team_b_players);
        LiveRoundState {
            match_id: config.match_id,
            series_id: config.series_id,
            map_id: config.map_id,
            map_number: config.map_number,
            phase: LiveMatchPhase::Warmup,
            team_a_id: config.team_a_id,
            team_b_id: config.team_b_id,
            team_a_sig: config.team_a_sig,
            team_b_sig: config.team_b_sig,
            best_of: config.best_of,
            world_seed: config.world_seed,
            round_number: 1,
            decision_seq: 0,
            rng_counter: 0,
            score_a: 0,
            score_b: 0,
            base_win_prob_a: config.base_win_prob_a,
            economy_a: Self::initial_economy(),
            economy_b: Self::initial_economy(),
            tactical_a: Self::initial_tactical(),
            tactical_b: Self::initial_tactical(),
            players_a,
            players_b,
            round_history: Vec::new(),
        }
    }

    fn init_players(
        config: &LiveMapConfig,
        team_id: TeamId,
        slots: &[LivePlayerSlot],
    ) -> Vec<LivePlayerState> {
        slots
            .iter()
            .map(|s| LivePlayerState {
                player_id: s.id,
                player_name: s.name.clone(),
                team_id,
                alive: true,
                health: 100,
                armor: 0,
                weapon: WeaponClass::Rifle,
                utility: UtilityInventory {
                    grenades: 1,
                    flashbangs: 2,
                    smokes: 1,
                    molotovs: 1,
                    he_grenades: 1,
                },
                kills: 0,
                deaths: 0,
                assists: 0,
                confidence: config.base_win_prob_a * 100.0,
            })
            .collect()
    }

    fn initial_economy() -> LiveEconomy {
        LiveEconomy {
            money: 800,
            equipment: EconomyBuy::Eco,
            timeout_remaining: 1,
            loss_bonus: 0,
        }
    }

    fn initial_tactical() -> LiveTacticalState {
        LiveTacticalState {
            momentum: 50.0,
            confidence: 50.0,
            morale: 50.0,
            round_stability: 50.0,
            economy_pressure: 0.0,
            pace: Pace::Default,
            timeout_called: false,
        }
    }

    /// 生成下一个回合（恰好一个）：应用决策 → 掷回合胜负 → 更新比分/经济 → 记录事件。
    ///
    /// 若已达 [`LiveMatchPhase::MapEnd`]，调用方应先 `finish_map`，本函数会拒绝继续。
    pub fn simulate_next_round(
        state: &mut LiveRoundState,
        decision: Option<LiveRoundDecision>,
    ) -> LiveRoundOutput {
        assert!(
            state.phase != LiveMatchPhase::MapEnd && state.phase != LiveMatchPhase::Completed,
            "LIVE 已结束，不得再生成回合"
        );

        Self::apply_decision(state, decision);

        // 进入 LIVE（Warmup / NextRound / RoundEnd / Halftime → Live）。
        state.phase = LiveMatchPhase::Live;

        let round_number = state.round_number;
        let side_a = Self::side_for(round_number);
        let side_b = side_a.opposite();

        // 回合确定性 RNG（决策已进入 seed 链）。
        let seed = round_seed(
            state.world_seed,
            &state.match_id,
            &state.map_id,
            round_number,
            state.decision_seq,
            SIMULATION_VERSION,
        );
        let mut rng = Xoshiro256StarStar::seed(seed);
        let win_a = Self::round_win_prob(state).clamp(0.05, 0.95);
        let a_wins = rng.roll_bp(win_a);

        if a_wins {
            state.score_a += 1;
        } else {
            state.score_b += 1;
        }
        Self::update_economies(state, a_wins);
        state.rng_counter += 1;

        let map_finished = state.score_a >= MR12_WIN_SCORE || state.score_b >= MR12_WIN_SCORE;
        let is_halftime = !map_finished && round_number == HALFTIME_ROUNDS;
        let event_kind = if map_finished {
            LiveRoundEventKind::MapEnd
        } else if is_halftime {
            LiveRoundEventKind::Halftime
        } else {
            LiveRoundEventKind::RoundPlayed
        };

        // R2：回合级击杀事件流（实时击杀可视化）。用派生 seed（域分隔）独立生成，
        // 不消费回合胜负 RNG 序列——历史回合 winner/score 不变，确定性不被扰动。
        let kills = Self::kill_feed_for(state, round_number, a_wins);

        let event = LiveRoundEvent {
            round_number,
            phase: if map_finished {
                LiveMatchPhase::MapEnd
            } else {
                LiveMatchPhase::RoundEnd
            },
            event_kind,
            winner_a: a_wins,
            score_a: state.score_a,
            score_b: state.score_b,
            side_a,
            side_b,
            kills,
        };
        // 只追加已发生的回合（未来回合绝不预写）。
        state.round_history.push(event.clone());

        if map_finished {
            state.phase = LiveMatchPhase::MapEnd;
        } else {
            state.phase = LiveMatchPhase::RoundEnd;
            state.round_number += 1; // 消费 1 → 下一回合编号
        }

        LiveRoundOutput {
            event,
            map_finished,
            series_finished: false,
        }
    }

    /// 地图结束后生成 [`MapScore`]（只有到 [`LiveMatchPhase::MapEnd`] 才允许调用）。
    ///
    /// K/D/A 口径与 `MatchSimulator` 对齐（2026-08-20 修复：此前逐人战绩恒为 0——
    /// `simulate_next_round` 只更新比分/经济，`LivePlayerState.kills/deaths/assists`
    /// 从未递增，导致 LIVE 比赛 PlayerLine 全零、ADR/KAST 被压到 0）。
    /// 本处用**独立派生 seed** 的 RNG 做守恒分配（KILLS_PER_ROUND/WIN_KILL_RATE/
    /// ASSIST_RATE 与 `MatchSimulator` 常量一致），不消费回合内 RNG、不改变
    /// `simulate_next_round` 的确定性消费序；同一存档同一决策序列产出同一 K/D/A。
    pub fn finish_map(state: &LiveRoundState) -> MapScore {
        assert!(
            state.phase == LiveMatchPhase::MapEnd,
            "只有 MAP_END 才能生成 MapScore（当前 {:?}）",
            state.phase
        );
        let winner_sig = if state.score_a > state.score_b {
            state.team_a_sig.clone()
        } else {
            state.team_b_sig.clone()
        };
        let total_rounds = (state.score_a + state.score_b).max(1);
        // 派生 RNG：以「已完成回合数 + 1」为域分隔（不会与任何已发生回合的 seed 碰撞），
        // decision_seq 进链保证不同决策序列产生不同战绩。
        let seed = round_seed(
            state.world_seed,
            &state.match_id,
            &state.map_id,
            total_rounds + 1,
            state.decision_seq,
            SIMULATION_VERSION,
        );
        let mut rng = Xoshiro256StarStar::seed(seed);
        let split = Self::allocate_kda(state, total_rounds, &mut rng);
        let KdaSplit {
            kills_a,
            deaths_a,
            assists_a,
            kills_b,
            deaths_b,
            assists_b,
        } = split;

        let lines = state
            .players_a
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let kpr = kills_a[i] as f64 / total_rounds as f64;
                let dpr = deaths_a[i] as f64 / total_rounds as f64;
                let apr = assists_a[i] as f64 / total_rounds as f64;
                PlayerLine {
                    player_name: p.player_name.clone(),
                    team_sig: state.team_a_sig.clone(),
                    kills: kills_a[i],
                    deaths: deaths_a[i],
                    assists: assists_a[i],
                    adr: RatingCalculator::adr_of(kpr, apr, dpr),
                    kast: RatingCalculator::kast_of(kpr, dpr, apr),
                }
            })
            .chain(state.players_b.iter().enumerate().map(|(i, p)| {
                let kpr = kills_b[i] as f64 / total_rounds as f64;
                let dpr = deaths_b[i] as f64 / total_rounds as f64;
                let apr = assists_b[i] as f64 / total_rounds as f64;
                PlayerLine {
                    player_name: p.player_name.clone(),
                    team_sig: state.team_b_sig.clone(),
                    kills: kills_b[i],
                    deaths: deaths_b[i],
                    assists: assists_b[i],
                    adr: RatingCalculator::adr_of(kpr, apr, dpr),
                    kast: RatingCalculator::kast_of(kpr, dpr, apr),
                }
            }))
            .collect();
        MapScore {
            map_number: state.map_number,
            team_a_score: state.score_a,
            team_b_score: state.score_b,
            winner_sig,
            lines,
        }
    }

    /// K/D/A 守恒分配（口径与 [`crate::match_simulator::MatchSimulator`] 对齐）：
    /// - 赢队总击杀 ≈ 总回合 × WIN_KILL_RATE，输队 = 总回合 × KILLS_PER_ROUND − 赢队
    ///   （双方总击杀守恒 ≈ 总回合 × KILLS_PER_ROUND）；
    /// - 死亡 = 对方击杀（镜像守恒：赢队击杀 = 输队死亡）；
    /// - 助攻 ≈ 本队击杀 × ASSIST_RATE；
    /// - 个体分配权重：LIVE 状态没有 `PlayerCharacter` 属性，用 `confidence`（0~100）
    ///   作为唯一可用个体差异因子（枪法强 → 击杀权重高），下限 1.0 防零权重。
    fn allocate_kda(
        state: &LiveRoundState,
        total_rounds: i32,
        rng: &mut Xoshiro256StarStar,
    ) -> KdaSplit {
        let a_wins = state.score_a > state.score_b;
        let win_kills = round_to_int(total_rounds as f64 * MatchSimulator::WIN_KILL_RATE);
        let total_kills = round_to_int(total_rounds as f64 * MatchSimulator::KILLS_PER_ROUND);
        let lose_kills = (total_kills - win_kills).max(0);
        let (kills_a, kills_b) = if a_wins {
            (win_kills, lose_kills)
        } else {
            (lose_kills, win_kills)
        };

        let w_a = Self::live_weights(&state.players_a);
        let w_b = Self::live_weights(&state.players_b);
        let k_a = allocate_kills(kills_a, &w_a, rng);
        let k_b = allocate_kills(kills_b, &w_b, rng);
        // 死亡 = 对方击杀的镜像分布（近似均匀 → 强者 DPR 不被推高，与 match_simulator 校准一致）。
        let d_a = Self::allocate_deaths(kills_b, &w_b, rng);
        let d_b = Self::allocate_deaths(kills_a, &w_a, rng);
        // 助攻 ≈ 本队击杀 × 0.25。
        let a_assists = allocate_kills(
            round_to_int(kills_a as f64 * MatchSimulator::ASSIST_RATE),
            &w_a,
            rng,
        );
        let b_assists = allocate_kills(
            round_to_int(kills_b as f64 * MatchSimulator::ASSIST_RATE),
            &w_b,
            rng,
        );
        KdaSplit {
            kills_a: k_a,
            deaths_a: d_a,
            assists_a: a_assists,
            kills_b: k_b,
            deaths_b: d_b,
            assists_b: b_assists,
        }
    }

    /// LIVE 选手击杀权重（confidence 归一 + 下限 1.0）。
    fn live_weights(players: &[LivePlayerState]) -> Vec<f64> {
        players
            .iter()
            .map(|p| (p.confidence / 100.0).max(MIN_WEIGHT))
            .collect()
    }

    /// 死亡分配：死亡由**对手**击杀产生，按对手权重轮盘分配（镜像守恒）。
    fn allocate_deaths(
        opponent_kills: i32,
        opponent_weights: &[f64],
        rng: &mut Xoshiro256StarStar,
    ) -> Vec<i32> {
        let mut counts = vec![0; opponent_weights.len()];
        if opponent_kills <= 0 || opponent_weights.is_empty() {
            return counts;
        }
        let weight_sum: f64 = opponent_weights.iter().sum();
        for _ in 0..opponent_kills {
            counts[pick_index(opponent_weights, weight_sum, rng)] += 1;
        }
        counts
    }

    /// 生成单个回合的击杀事件流（确定性、域分隔 seed，不扰动回合胜负 RNG）。
    ///
    /// 口径与 `finish_map` 守恒对齐：本回合击杀数 ≈ `KILLS_PER_ROUND`，赢队占
    /// `WIN_KILL_RATE` 比例；击杀者按队伍 confidence 权重轮盘分配，受害者从对方
    /// 队伍均匀镜像分配（击杀者队 = 受害队对方）。首杀 = 本回合第一击。
    /// 返回按时间序的 `LiveKillEvent`（前端按序渲染实时击杀 feed）。
    fn kill_feed_for(
        state: &LiveRoundState,
        round_number: i32,
        a_wins: bool,
    ) -> Vec<LiveKillEvent> {
        // 派生 seed：域分隔 + 回合 seed 折叠（与 round_seed 独立，绝不消费其 RNG）。
        let round_seed_base = round_seed(
            state.world_seed,
            &state.match_id,
            &state.map_id,
            round_number,
            state.decision_seq,
            SIMULATION_VERSION,
        );
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(KILL_FEED_DOMAIN.as_bytes());
        buf.extend_from_slice(&round_seed_base.to_le_bytes());
        let mut rng = Xoshiro256StarStar::seed(csc_util::seed_chain::fnv1a64(&buf));

        // 回合击杀数守恒（≈ KILLS_PER_ROUND；至少 1 保证 MVP 击杀非空）。
        let total_kills = round_to_int(MatchSimulator::KILLS_PER_ROUND).clamp(1, 10);
        let win_kills =
            round_to_int(total_kills as f64 * MatchSimulator::WIN_KILL_RATE).clamp(1, total_kills);
        let lose_kills = total_kills - win_kills;
        let (kills_a, kills_b) = if a_wins {
            (win_kills, lose_kills)
        } else {
            (lose_kills, win_kills)
        };

        let w_a = Self::live_weights(&state.players_a);
        let w_b = Self::live_weights(&state.players_b);

        // 逐击分配：击杀者按本队权重，受害者按对方权重（镜像）。
        let mut events: Vec<LiveKillEvent> = Vec::with_capacity(total_kills as usize);
        let mut kill_idx = 0;
        // A 队击杀 → 受害者是 B 队。
        Self::push_kills(
            &mut events,
            &mut kill_idx,
            kills_a,
            &state.players_a,
            &state.players_b,
            0,
            &w_a,
            &w_b,
            &mut rng,
        );
        // B 队击杀 → 受害者是 A 队。
        Self::push_kills(
            &mut events,
            &mut kill_idx,
            kills_b,
            &state.players_b,
            &state.players_a,
            1,
            &w_b,
            &w_a,
            &mut rng,
        );
        // 按时间交错（A/B 击杀交替出现更接近真实 feed；确定性：按 stable 序合并）。
        // 简化：维持 A 队先打、B 队后打的顺序也可，但为时序自然，按 (team, idx) 排序
        // 会让 A 全在前；改为按击杀序号交错。此处按当前生成序即为时间序（先 A 队
        // 后 B 队），保持稳定即可，不再重排。
        events
    }

    /// 填充一队击杀事件（击杀者队 → 受害者队镜像）。
    #[allow(clippy::too_many_arguments)]
    fn push_kills(
        events: &mut Vec<LiveKillEvent>,
        kill_idx: &mut i32,
        kills: i32,
        killers: &[LivePlayerState],
        victims: &[LivePlayerState],
        team: u8,
        killer_weights: &[f64],
        victim_weights: &[f64],
        rng: &mut Xoshiro256StarStar,
    ) {
        if killers.is_empty() || victims.is_empty() {
            return;
        }
        let killer_sum: f64 = killer_weights.iter().sum();
        let victim_sum: f64 = victim_weights.iter().sum();
        for _ in 0..kills {
            let ki = pick_index(killer_weights, killer_sum, rng);
            let vi = pick_index(victim_weights, victim_sum, rng);
            events.push(LiveKillEvent {
                team,
                killer: LivePlayerRef {
                    id: killers[ki].player_id,
                    name: killers[ki].player_name.clone(),
                    team,
                },
                victim: LivePlayerRef {
                    id: victims[vi].player_id,
                    name: victims[vi].player_name.clone(),
                    team: 1 - team,
                },
                weapon: killers[ki].weapon,
                first_kill: *kill_idx == 0,
            });
            *kill_idx += 1;
        }
    }

    /// 单队当前半场（1/2；加时 3+）。
    pub fn half_of(state: &LiveRoundState) -> u8 {
        (((state.round_number - 1) / HALFTIME_ROUNDS) + 1) as u8
    }

    /// 比分快照（供编排层/前端展示）。
    pub fn score_of(state: &LiveRoundState) -> LiveRoundScore {
        let half = Self::half_of(state);
        let side_a = Self::side_for(state.round_number);
        LiveRoundScore {
            score_a: state.score_a,
            score_b: state.score_b,
            round_number: state.round_number,
            half,
            side_a,
            side_b: side_a.opposite(),
            rounds_in_current_half: ((state.round_number - 1) % HALFTIME_ROUNDS) + 1,
        }
    }

    fn side_for(round_number: i32) -> Side {
        if round_number <= HALFTIME_ROUNDS {
            Side::T
        } else {
            Side::CT
        }
    }

    /// 回合 A 队胜率 = 基础胜率 + 双方战术/经济偏移（叙事量级，防极端）。
    fn round_win_prob(state: &LiveRoundState) -> f64 {
        let mut p = state.base_win_prob_a;
        p += state.tactical_a.win_adjust();
        p -= state.tactical_b.win_adjust();
        // 经济：A 全买 → +0.03；B 全买 → −0.03。
        p += match state.economy_a.equipment {
            EconomyBuy::FullBuy => 0.03,
            EconomyBuy::ForceBuy => 0.02,
            EconomyBuy::HalfBuy => 0.01,
            EconomyBuy::Eco => -0.02,
        };
        p -= match state.economy_b.equipment {
            EconomyBuy::FullBuy => 0.03,
            EconomyBuy::ForceBuy => 0.02,
            EconomyBuy::HalfBuy => 0.01,
            EconomyBuy::Eco => -0.02,
        };
        p
    }

    fn apply_decision(state: &mut LiveRoundState, decision: Option<LiveRoundDecision>) {
        let Some(decision) = decision else { return };
        // 决策进入 seed 链（单调递增）。
        state.decision_seq += 1;
        match decision {
            LiveRoundDecision::None => {}
            LiveRoundDecision::CallTimeout => {
                state.tactical_a.morale = (state.tactical_a.morale + 6.0).min(100.0);
                state.tactical_a.momentum = (state.tactical_a.momentum + 4.0).min(100.0);
                state.economy_a.timeout_remaining =
                    state.economy_a.timeout_remaining.saturating_sub(1);
                state.tactical_a.timeout_called = true;
            }
            LiveRoundDecision::ChangePace(pace) => {
                state.tactical_a.pace = pace;
            }
            LiveRoundDecision::AggressiveOpening => {
                state.tactical_a.momentum = (state.tactical_a.momentum + 5.0).min(100.0);
                state.tactical_a.economy_pressure =
                    (state.tactical_a.economy_pressure + 4.0).min(100.0);
            }
            LiveRoundDecision::PlayForTrade => {
                state.tactical_a.round_stability =
                    (state.tactical_a.round_stability - 3.0).max(0.0);
                state.tactical_a.momentum = (state.tactical_a.momentum + 2.0).min(100.0);
            }
            LiveRoundDecision::Save => {
                state.economy_a.loss_bonus += 1;
                state.tactical_a.economy_pressure =
                    (state.tactical_a.economy_pressure + 3.0).min(100.0);
            }
            LiveRoundDecision::ForceBuy => {
                state.economy_a.money = (state.economy_a.money - 3000).max(0);
                state.economy_a.equipment = EconomyBuy::FullBuy;
            }
            LiveRoundDecision::ChangeUtilityPlan(_plan) => {
                state.tactical_a.round_stability =
                    (state.tactical_a.round_stability + 1.0).min(100.0);
            }
            LiveRoundDecision::Encourage(id) => {
                state.tactical_a.morale = (state.tactical_a.morale + 5.0).min(100.0);
                state.tactical_a.confidence = (state.tactical_a.confidence + 3.0).min(100.0);
                if let Some(p) = state.players_a.iter_mut().find(|p| p.player_id == id) {
                    p.confidence = (p.confidence + 5.0).min(100.0);
                }
            }
            LiveRoundDecision::Criticize(id) => {
                state.tactical_a.morale = (state.tactical_a.morale - 4.0).max(0.0);
                if let Some(p) = state.players_a.iter_mut().find(|p| p.player_id == id) {
                    p.confidence = (p.confidence - 5.0).max(0.0);
                }
            }
        }
    }

    fn update_economies(state: &mut LiveRoundState, a_wins: bool) {
        if a_wins {
            state.economy_a.money += 2200;
            state.economy_b.money += 1400 + state.economy_b.loss_bonus * 500;
            state.economy_b.loss_bonus += 1;
            state.economy_a.loss_bonus = 0;
        } else {
            state.economy_b.money += 2200;
            state.economy_a.money += 1400 + state.economy_a.loss_bonus * 500;
            state.economy_a.loss_bonus += 1;
            state.economy_b.loss_bonus = 0;
        }
        // 用经济推断下一回合装备档位（简单阈值）。
        state.economy_a.equipment = Self::buy_tier(state.economy_a.money);
        state.economy_b.equipment = Self::buy_tier(state.economy_b.money);
    }

    fn buy_tier(money: i32) -> EconomyBuy {
        if money >= 6500 {
            EconomyBuy::FullBuy
        } else if money >= 4500 {
            EconomyBuy::ForceBuy
        } else if money >= 3000 {
            EconomyBuy::HalfBuy
        } else {
            EconomyBuy::Eco
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LiveMapConfig {
        LiveMapConfig {
            match_id: "m1".into(),
            series_id: "s1".into(),
            map_id: "map_1".into(),
            map_number: 1,
            team_a_id: TeamId(0),
            team_b_id: TeamId(1),
            team_a_sig: "A|a0,a1,a2,a3,a4".into(),
            team_b_sig: "B|b0,b1,b2,b3,b4".into(),
            best_of: 1,
            world_seed: 42,
            base_win_prob_a: 0.55,
            team_a_players: (0..5)
                .map(|i| LivePlayerSlot {
                    id: PlayerId(i),
                    name: format!("a{i}"),
                })
                .collect(),
            team_b_players: (0..5)
                .map(|i| LivePlayerSlot {
                    id: PlayerId(100 + i),
                    name: format!("b{i}"),
                })
                .collect(),
        }
    }

    /// 推进一张图直到 MAP_END（None 决策自动回合）。
    fn run_to_end(mut state: LiveRoundState) -> (LiveRoundState, usize) {
        let mut rounds = 0;
        while state.phase != LiveMatchPhase::MapEnd {
            assert!(rounds < 100, "不应超过 100 回合");
            let out = LiveMatchEngine::simulate_next_round(&mut state, None);
            rounds += 1;
            if out.map_finished {
                break;
            }
        }
        (state, rounds)
    }

    // —— 8.1 Live Test：逐回合推进，未来回合不存在 ——
    #[test]
    fn live_test_single_round_no_future_results() {
        let mut state = LiveMatchEngine::start_map(config());
        assert_eq!(state.phase, LiveMatchPhase::Warmup);
        assert!(state.round_history.is_empty(), "WARMUP 无任何回合事件");
        assert_eq!(state.round_number, 1, "下一回合编号 = 1");

        let out = LiveMatchEngine::simulate_next_round(&mut state, None);
        assert_eq!(out.event.round_number, 1, "只生成第 1 回合");
        assert_eq!(state.round_history.len(), 1, "history 只含 1 个已发生回合");
        assert!(!out.map_finished);
        assert_eq!(state.round_number, 2, "消费 1 → 下一回合编号 2");

        // 关键：第 2 回合尚未生成，state 中没有任何第 2 回合的胜负。
        assert!(
            state.round_history.iter().all(|e| e.round_number == 1),
            "状态不得含未来回合事件"
        );
        assert_eq!(state.score_a + state.score_b, 1, "比分只反映已完成回合");
    }

    #[test]
    fn live_test_progresses_to_map_end_then_finish_map() {
        let (state, rounds) = run_to_end(LiveMatchEngine::start_map(config()));
        assert_eq!(state.phase, LiveMatchPhase::MapEnd);
        assert_eq!(
            state.round_history.len(),
            rounds,
            "事件数与已发生回合数一致"
        );
        assert!(
            state.score_a == MR12_WIN_SCORE || state.score_b == MR12_WIN_SCORE,
            "MR12 胜者 13 分: {}:{}",
            state.score_a,
            state.score_b
        );

        let map = LiveMatchEngine::finish_map(&state);
        assert_eq!(map.map_number, 1);
        assert_eq!(map.team_a_score, state.score_a);
        assert_eq!(map.team_b_score, state.score_b);
        assert_eq!(map.lines.len(), 10, "双方 10 人战绩");
        assert!(
            map.winner_sig == state.team_a_sig || map.winner_sig == state.team_b_sig,
            "胜方签名合法"
        );
        // 2026-08-20 修复回归：LIVE 图内 K/D/A 不得全零（此前 simulate_next_round
        // 不递增 LivePlayerState.kills/deaths/assists，finish_map 输出全 0）。
        let total_kills: i32 = map.lines.iter().map(|l| l.kills).sum();
        let total_deaths: i32 = map.lines.iter().map(|l| l.deaths).sum();
        let total_assists: i32 = map.lines.iter().map(|l| l.assists).sum();
        assert!(
            total_kills > 0,
            "LIVE 图内总击杀必须 > 0（实际 {total_kills}）"
        );
        assert!(
            total_deaths > 0,
            "LIVE 图内总死亡必须 > 0（实际 {total_deaths}）"
        );
        assert!(
            total_assists > 0,
            "LIVE 图内总助攻必须 > 0（实际 {total_assists}）"
        );
        // 守恒：总击杀 ≈ 总回合 × KILLS_PER_ROUND（±1 取整误差）
        let expected = state.score_a + state.score_b;
        let total_rounds = (state.score_a + state.score_b).max(1);
        let expected_kills = round_to_int(total_rounds as f64 * MatchSimulator::KILLS_PER_ROUND);
        assert!(
            (total_kills - expected_kills).abs() <= 1,
            "总击杀应≈{expected_kills}（回合 {expected}）: {total_kills}"
        );
        // 守恒：总击杀 == 总死亡（镜像）
        assert_eq!(
            total_kills, total_deaths,
            "总击杀必须等于总死亡（镜像守恒）"
        );
        // 助攻 ≈ 总击杀 × 0.25（±1 取整误差）
        let expected_assists = round_to_int(total_kills as f64 * MatchSimulator::ASSIST_RATE);
        assert!(
            (total_assists - expected_assists).abs() <= 1,
            "总助攻应≈{expected_assists}（击杀 {total_kills}）: {total_assists}"
        );
    }

    #[test]
    fn live_test_halftime_side_switch() {
        let mut state = LiveMatchEngine::start_map(config());
        let mut saw_halftime = false;
        let mut rounds = 0;
        while state.phase != LiveMatchPhase::MapEnd && rounds < 100 {
            let round_no = state.round_number;
            let out = LiveMatchEngine::simulate_next_round(&mut state, None);
            if round_no == HALFTIME_ROUNDS && !out.map_finished {
                saw_halftime = true;
                assert_eq!(out.event.event_kind, LiveRoundEventKind::Halftime);
            }
            rounds += 1;
        }
        // 双方势均力敌，几乎必然打满 12+ 回合 → 半场换边必然出现。
        assert!(saw_halftime, "应在第 12 回合后触发半场换边");
    }

    // —— 8.2 Determinism Test：同 seed 同决策序列 → 两次结果逐位一致 ——
    #[test]
    fn determinism_same_seed_same_decision() {
        let run = || -> (Vec<LiveRoundEvent>, LiveRoundScore) {
            let mut s = LiveMatchEngine::start_map(config());
            while s.phase != LiveMatchPhase::MapEnd {
                let _ = LiveMatchEngine::simulate_next_round(&mut s, None);
            }
            (s.round_history.clone(), LiveMatchEngine::score_of(&s))
        };
        let (ev_a, sc_a) = run();
        let (ev_b, sc_b) = run();
        assert_eq!(ev_a, ev_b, "两次逐回合事件序列必须完全一致");
        assert_eq!(sc_a, sc_b, "最终比分必须一致");
    }

    #[test]
    fn finish_map_kda_is_deterministic_and_conserved() {
        // 同一状态 finish_map 两次 → K/D/A 逐位一致（派生 RNG 不进回合 seed 链）。
        let run = || -> MapScore {
            let mut s = LiveMatchEngine::start_map(config());
            while s.phase != LiveMatchPhase::MapEnd {
                let _ = LiveMatchEngine::simulate_next_round(&mut s, None);
            }
            LiveMatchEngine::finish_map(&s)
        };
        let m1 = run();
        let m2 = run();
        assert_eq!(
            m1.lines, m2.lines,
            "同状态 finish_map 两次 K/D/A 必须逐位一致"
        );
        // 10 人、双方 5 人。
        assert_eq!(m1.lines.len(), 10);
        let a_count = m1
            .lines
            .iter()
            .filter(|l| l.team_sig == "A|a0,a1,a2,a3,a4")
            .count();
        let b_count = m1
            .lines
            .iter()
            .filter(|l| l.team_sig == "B|b0,b1,b2,b3,b4")
            .count();
        assert_eq!(a_count, 5, "A 队 5 人");
        assert_eq!(b_count, 5, "B 队 5 人");
        // 每人 K/D/A 非负。
        assert!(
            m1.lines
                .iter()
                .all(|l| l.kills >= 0 && l.deaths >= 0 && l.assists >= 0)
        );
        // 胜负与比分一致：胜者 13 分。
        let winner = if m1.team_a_score > m1.team_b_score {
            "A|a0,a1,a2,a3,a4"
        } else {
            "B|b0,b1,b2,b3,b4"
        };
        assert_eq!(m1.winner_sig, winner);
        // 赢队总击杀 > 输队总击杀（WIN_KILL_RATE 3.5 > 3.3）。
        let win_kills: i32 = m1
            .lines
            .iter()
            .filter(|l| l.team_sig == m1.winner_sig)
            .map(|l| l.kills)
            .sum();
        let lose_kills: i32 = m1.lines.iter().map(|l| l.kills).sum::<i32>() - win_kills;
        assert!(
            win_kills > lose_kills,
            "赢队总击杀应大于输队: {win_kills} vs {lose_kills}"
        );
    }

    #[test]
    fn determinism_survives_serialize_roundtrip() {
        // 存档/恢复（§7.3）：保存后从同一状态 + 同一决策继续，事件必须一致。
        let mut s = LiveMatchEngine::start_map(config());
        let _ = LiveMatchEngine::simulate_next_round(&mut s, None);
        let _ = LiveMatchEngine::simulate_next_round(&mut s, None);
        // 模拟存档：序列化后反序列化。
        let json = serde_json::to_string(&s).unwrap();
        let mut restored: LiveRoundState = serde_json::from_str(&json).unwrap();
        let ev_direct =
            LiveMatchEngine::simulate_next_round(&mut s, Some(LiveRoundDecision::CallTimeout));
        let ev_restored = LiveMatchEngine::simulate_next_round(
            &mut restored,
            Some(LiveRoundDecision::CallTimeout),
        );
        assert_eq!(
            ev_direct.event, ev_restored.event,
            "恢复后同一决策事件必须一致"
        );
        assert_eq!(s.round_history, restored.round_history);
        assert_eq!(s.score_a, restored.score_a);
        assert_eq!(s.score_b, restored.score_b);
    }

    // —— 8.3 Divergence Test：不同决策产生状态分歧 ——
    #[test]
    fn divergence_different_decisions_diverge() {
        let diverges = |seed: u64| -> bool {
            let mut cfg = config();
            cfg.world_seed = seed;
            let mut a = LiveMatchEngine::start_map(cfg.clone());
            let mut b = LiveMatchEngine::start_map(cfg);
            // 首回合不同决策。
            let _ =
                LiveMatchEngine::simulate_next_round(&mut a, Some(LiveRoundDecision::CallTimeout));
            let _ = LiveMatchEngine::simulate_next_round(&mut b, Some(LiveRoundDecision::Save));
            // 决策 seq 不同或战术/经济状态不同。
            a.decision_seq != b.decision_seq
                || a.tactical_a != b.tactical_a
                || a.economy_a != b.economy_a
                || a.economy_b != b.economy_b
        };
        // 多个 seed 下应出现分歧（决策作用于 seed 链 + 状态，几乎必然分歧）。
        let diverged = (0..10).filter(|&s| diverges(s)).count();
        assert!(
            diverged >= 8,
            "多数 seed 下不同决策应产生状态分歧（实际 {diverged}/10）"
        );
    }

    // —— 8.4 LIVE != Replay Test ——
    #[test]
    fn live_not_replay_after_n_rounds_no_next_winner() {
        let mut state = LiveMatchEngine::start_map(config());
        // 只生成第 5 回合。
        for _ in 0..5 {
            let out = LiveMatchEngine::simulate_next_round(&mut state, None);
            assert!(!out.map_finished);
        }
        assert_eq!(state.round_history.len(), 5, "只发生 5 回合");
        // 状态中不得出现第 6 回合的任何胜负。
        assert!(
            state.round_history.iter().all(|e| e.round_number <= 5),
            "未来回合（>5）结果不得存在"
        );
        // 未到 MAP_END，不得产出 MapScore（finish_map 应 panic 断言）。
        let st = state.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            LiveMatchEngine::finish_map(&st)
        }));
        assert!(result.is_err(), "phase != MAP_END 时 finish_map 必须拒绝");
    }

    #[test]
    fn live_not_replay_map_score_only_after_map_end() {
        let (state, _rounds) = run_to_end(LiveMatchEngine::start_map(config()));
        assert_eq!(state.phase, LiveMatchPhase::MapEnd);
        let map = LiveMatchEngine::finish_map(&state);
        // 完成后生成的战绩行数与已发生回合数一致（10 人，非未来预生成）。
        assert_eq!(map.lines.len(), 10);
        assert_eq!(
            state.round_history.len() as i32,
            state.score_a + state.score_b
        );
    }

    // —— 状态机不变量 ——
    #[test]
    fn phase_transitions_are_sane() {
        let mut state = LiveMatchEngine::start_map(config());
        assert_eq!(state.phase, LiveMatchPhase::Warmup);
        let out = LiveMatchEngine::simulate_next_round(&mut state, None);
        assert!(!out.map_finished);
        assert_eq!(state.phase, LiveMatchPhase::RoundEnd);
        // 下一回合回到 LIVE。
        let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        assert_eq!(state.phase, LiveMatchPhase::RoundEnd);
        // 推进到结束。
        let (state, _) = run_to_end(state);
        assert_eq!(state.phase, LiveMatchPhase::MapEnd);
    }

    #[test]
    fn rng_counter_tracks_rounds() {
        let mut state = LiveMatchEngine::start_map(config());
        assert_eq!(state.rng_counter, 0);
        let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        assert_eq!(state.rng_counter, 1);
    }

    // —— 以下为测试工程师独立补充的边界/回归测试 ——

    /// 基于默认 config 生成指定 A 队基础胜率的配置。
    fn config_with_prob(prob: f64) -> LiveMapConfig {
        let mut cfg = config();
        cfg.base_win_prob_a = prob;
        cfg
    }

    // —— 审查点 a：极端胜率（0.0 / 1.0 / 负数 / >1）能否推进到 MAP_END ——
    #[test]
    fn extreme_win_prob_reaches_map_end() {
        // round_win_prob 返回值经 .clamp(0.05, 0.95) 兜底，
        // 任何极端输入都必须推进到 MAP_END，且不 panic（不会死循环）。
        for prob in [0.0_f64, 1.0_f64, -0.5_f64, 2.0_f64] {
            let mut state = LiveMatchEngine::start_map(config_with_prob(prob));
            let mut rounds = 0;
            while state.phase != LiveMatchPhase::MapEnd {
                assert!(rounds < 100, "极端胜率 {prob} 下仍应在 100 回合内结束");
                let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
                rounds += 1;
            }
            // MR12：胜者必达 13 分。
            assert!(
                state.score_a == MR12_WIN_SCORE || state.score_b == MR12_WIN_SCORE,
                "极端胜率 {prob} 下比分异常: {}:{}",
                state.score_a,
                state.score_b
            );
            // 即使 base_win_prob_a=0.0，clamp 后 A 仍有 5% 胜率，绝不出现无限连输导致挂起。
            assert_eq!(
                state.score_a + state.score_b,
                rounds,
                "已完成回合数必须等于事件数"
            );
        }
    }

    #[test]
    fn extreme_win_prob_deterministic() {
        // 极端胜率下确定性仍成立（同 seed 同推进 → 同比分）。
        let run = |prob: f64| -> (i32, i32) {
            let mut state = LiveMatchEngine::start_map(config_with_prob(prob));
            while state.phase != LiveMatchPhase::MapEnd {
                let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
            }
            (state.score_a, state.score_b)
        };
        assert_eq!(run(0.0), run(0.0), "base_win_prob_a=0.0 两次运行必须一致");
        assert_eq!(run(1.0), run(1.0), "base_win_prob_a=1.0 两次运行必须一致");
    }

    // —— 审查点 b：半场边界与换边 ——
    #[test]
    fn halftime_boundary_swaps_side() {
        // side_for 是私有函数；直接验证 round 12 前为 T，round 13 起为 CT。
        assert_eq!(LiveMatchEngine::side_for(1), Side::T);
        assert_eq!(LiveMatchEngine::side_for(HALFTIME_ROUNDS), Side::T);
        assert_eq!(LiveMatchEngine::side_for(HALFTIME_ROUNDS + 1), Side::CT);
        // half_of：第 13 回合属于第 2 个半场。
        let mut state = LiveMatchEngine::start_map(config_with_prob(0.5));
        // 推进到第 13 回合开始（round_number == 13）。
        while state.round_number <= HALFTIME_ROUNDS && state.phase != LiveMatchPhase::MapEnd {
            let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        }
        if state.phase != LiveMatchPhase::MapEnd {
            assert_eq!(
                state.round_number,
                HALFTIME_ROUNDS + 1,
                "应刚进入第 13 回合"
            );
            assert_eq!(
                LiveMatchEngine::half_of(&state),
                2,
                "第 13 回合应在第 2 半场"
            );
            let score = LiveMatchEngine::score_of(&state);
            assert_eq!(score.half, 2);
            assert_eq!(score.side_a, Side::CT, "第 13 回合 A 队应换到 CT");
            assert_eq!(score.side_b, Side::T, "第 13 回合 B 队应换到 T");
        }
        // 势均力敌下应能观察到第 12 回合的 HALFTIME 事件。
        let mut state2 = LiveMatchEngine::start_map(config_with_prob(0.5));
        let mut saw_halftime_event = false;
        let mut rounds = 0;
        while state2.phase != LiveMatchPhase::MapEnd && rounds < 100 {
            let round_no = state2.round_number;
            let out = LiveMatchEngine::simulate_next_round(&mut state2, None);
            if round_no == HALFTIME_ROUNDS && !out.map_finished {
                assert_eq!(out.event.event_kind, LiveRoundEventKind::Halftime);
                assert_eq!(
                    out.event.side_a,
                    Side::T,
                    "半场事件仍记录第 12 回合换边前的方"
                );
                saw_halftime_event = true;
            }
            rounds += 1;
        }
        assert!(saw_halftime_event, "势均力敌必然打满 12 回合并触发半场");
    }

    // —— 审查点 c：经济阈值边界 ——
    #[test]
    fn buy_tier_threshold_boundaries() {
        // buy_tier 私有函数：>=6500 FullBuy / >=4500 ForceBuy / >=3000 HalfBuy / else Eco。
        assert_eq!(LiveMatchEngine::buy_tier(2999), EconomyBuy::Eco);
        assert_eq!(LiveMatchEngine::buy_tier(3000), EconomyBuy::HalfBuy);
        assert_eq!(LiveMatchEngine::buy_tier(4499), EconomyBuy::HalfBuy);
        assert_eq!(LiveMatchEngine::buy_tier(4500), EconomyBuy::ForceBuy);
        assert_eq!(LiveMatchEngine::buy_tier(6499), EconomyBuy::ForceBuy);
        assert_eq!(LiveMatchEngine::buy_tier(6500), EconomyBuy::FullBuy);
        assert_eq!(LiveMatchEngine::buy_tier(6501), EconomyBuy::FullBuy);
        assert_eq!(LiveMatchEngine::buy_tier(i32::MAX), EconomyBuy::FullBuy);
        assert_eq!(LiveMatchEngine::buy_tier(i32::MIN), EconomyBuy::Eco);
        assert_eq!(LiveMatchEngine::buy_tier(0), EconomyBuy::Eco);
    }

    #[test]
    fn economy_money_grows_but_equipment_tier_caps_at_full_buy() {
        // 经济随胜/负增长，但装备档位有天花板（FullBuy），不会无限攀升。
        let mut state = LiveMatchEngine::start_map(config_with_prob(1.0)); // A 几乎必胜
        while state.phase != LiveMatchPhase::MapEnd {
            let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        }
        // 胜队经济应积累到 FullBuy 档。
        assert_eq!(state.economy_a.equipment, EconomyBuy::FullBuy);
        // 装备枚举没有更高级档位；money 本身可继续增长但不改变档位。
        assert!(state.economy_a.money >= 6500);
    }

    // —— 审查点 d：None 决策不递增 decision_seq，但 round_number 已保证 seed 分歧 ——
    #[test]
    fn none_decision_does_not_advance_decision_seq() {
        let mut state = LiveMatchEngine::start_map(config());
        assert_eq!(state.decision_seq, 0);
        // None 决策不进入 seed 链计数。
        let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        assert_eq!(state.decision_seq, 0, "None 决策不得递增 decision_seq");
        // 一个真实决策才递增。
        let _ =
            LiveMatchEngine::simulate_next_round(&mut state, Some(LiveRoundDecision::CallTimeout));
        assert_eq!(state.decision_seq, 1, "真实决策必须递增 decision_seq");
        // 同 seed 不同 None/决策 提交时机，round_number 在 seed 链里保证分歧。
        let mut a = LiveMatchEngine::start_map(config());
        let mut b = LiveMatchEngine::start_map(config());
        // a：先 None 后 CallTimeout；b：先 CallTimeout 后 None。
        let _ = LiveMatchEngine::simulate_next_round(&mut a, None);
        let _ = LiveMatchEngine::simulate_next_round(&mut a, Some(LiveRoundDecision::CallTimeout));
        let _ = LiveMatchEngine::simulate_next_round(&mut b, Some(LiveRoundDecision::CallTimeout));
        let _ = LiveMatchEngine::simulate_next_round(&mut b, None);
        assert_ne!(a.round_history, b.round_history, "不同提交时机应产生分歧");
    }

    // —— 审查点 e：序列化 roundtrip 覆盖全部关键字段（含 ID 与多决策状态） ——
    #[test]
    fn serialize_roundtrip_full_state() {
        let mut s = LiveMatchEngine::start_map(config());
        // 提交多个不同决策 + 推进多回合，制造可序列化的复杂状态。
        let _ = LiveMatchEngine::simulate_next_round(&mut s, Some(LiveRoundDecision::CallTimeout));
        let _ = LiveMatchEngine::simulate_next_round(&mut s, Some(LiveRoundDecision::ForceBuy));
        let _ = LiveMatchEngine::simulate_next_round(
            &mut s,
            Some(LiveRoundDecision::Encourage(PlayerId(0))),
        );
        // PlayerId/TeamId 必须可序列化（tuple struct → 裸数字，符合 ID 新类型契约）。
        assert_eq!(serde_json::to_string(&TeamId(0)).unwrap(), "0");
        assert_eq!(serde_json::to_string(&PlayerId(42)).unwrap(), "42");

        let json = serde_json::to_string(&s).unwrap();
        let mut restored: LiveRoundState = serde_json::from_str(&json).unwrap();
        // 全字段一致 = roundtrip 无损。
        assert_eq!(restored, s, "完整状态 roundtrip 必须无损");
        assert_eq!(restored.round_number, s.round_number);
        assert_eq!(restored.decision_seq, s.decision_seq);
        assert_eq!(restored.team_a_id, TeamId(0));
        assert_eq!(restored.team_b_id, TeamId(1));
        assert_eq!(restored.players_a[0].player_id, PlayerId(0));
        assert_eq!(restored.economy_a, s.economy_a);
        assert_eq!(restored.tactical_a, s.tactical_a);
        assert_eq!(restored.round_history, s.round_history);

        // 恢复后的状态仍能继续推进且事件一致（存档恢复契约）。
        let ev_direct = LiveMatchEngine::simulate_next_round(&mut s, Some(LiveRoundDecision::Save));
        let ev_restored =
            LiveMatchEngine::simulate_next_round(&mut restored, Some(LiveRoundDecision::Save));
        assert_eq!(ev_direct.event, ev_restored.event);
        assert_eq!(s.round_history, restored.round_history);
    }

    // —— 审查点 f 关联：多决策叠加（含 Save 对 loss_bonus / 经济档位的影响） ——
    #[test]
    fn multi_decision_accumulate_and_save_effect() {
        let mut state = LiveMatchEngine::start_map(config());
        // 连续提交多个决策：CallTimeout + Save + ChangePace。
        let _ =
            LiveMatchEngine::simulate_next_round(&mut state, Some(LiveRoundDecision::CallTimeout));
        assert_eq!(
            state.economy_a.timeout_remaining, 0,
            "CallTimeout 消耗 1 次暂停"
        );
        assert!(state.tactical_a.timeout_called);
        // Save 提升连败奖励并累积经济压力。注意：若本回合 A 获胜，loss_bonus 会被
        // update_economies 重置为 0（Save 的加成只对后续连败生效）；但经济压力必然累积。
        let _ = LiveMatchEngine::simulate_next_round(&mut state, Some(LiveRoundDecision::Save));
        assert!(
            state.tactical_a.economy_pressure > 0.0,
            "Save 应累积经济压力"
        );
        // Save 在 apply_decision 阶段直接递增 loss_bonus（这是确定性的、不经回合重置）。
        let mut st = LiveMatchEngine::start_map(config());
        LiveMatchEngine::apply_decision(&mut st, Some(LiveRoundDecision::Save));
        assert_eq!(st.economy_a.loss_bonus, 1, "Save 决策应直接递增 loss_bonus");
        // 但 simulate_next_round 里的 update_economies 可能因获胜而将 loss_bonus 重置为 0。
        // 用 A 几乎必输的配置验证连败累计路径。
        let mut loser = LiveMatchEngine::start_map(config_with_prob(0.0)); // A 几乎必输（clamp 后 5%）
        let mut consecutive_losses = 0;
        let mut rounds = 0;
        while loser.phase != LiveMatchPhase::MapEnd && rounds < 30 {
            let _ = LiveMatchEngine::simulate_next_round(&mut loser, Some(LiveRoundDecision::Save));
            rounds += 1;
            // 只要没有赢，Save 的 loss_bonus 就持续累计。
            if loser.economy_a.loss_bonus > consecutive_losses {
                consecutive_losses = loser.economy_a.loss_bonus;
            }
        }
        assert!(
            consecutive_losses >= 1,
            "A 几乎必输下 Save 应至少累计 1 次 loss_bonus"
        );
        // ChangePace 更新节奏。
        let _ = LiveMatchEngine::simulate_next_round(
            &mut state,
            Some(LiveRoundDecision::ChangePace(Pace::Fast)),
        );
        assert_eq!(state.tactical_a.pace, Pace::Fast);
        // 决策序列单调递增。
        assert_eq!(state.decision_seq, 3);
        // 多决策 + 存档恢复仍一致。
        let json = serde_json::to_string(&state).unwrap();
        let restored: LiveRoundState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, state);
    }

    // —— R2：回合级击杀事件流 ——

    #[test]
    fn live_round_event_contains_kill_feed() {
        let mut state = LiveMatchEngine::start_map(config());
        let out = LiveMatchEngine::simulate_next_round(&mut state, None);
        // 每个已发生回合都必须携带击杀事件流（实时击杀可视化数据源）。
        assert!(
            !out.event.kills.is_empty(),
            "回合必须有击杀事件流（R2 实时击杀）"
        );
        // 击杀数量守恒 ≈ KILLS_PER_ROUND（取整后 [1,10]）。
        let kills = out.event.kills.len();
        assert!(
            (1..=10).contains(&kills),
            "回合击杀数应在 [1,10]，实际 {kills}"
        );
        // 首杀标记：恰有一个 first_kill。
        assert_eq!(
            out.event.kills.iter().filter(|k| k.first_kill).count(),
            1,
            "恰有一个首杀"
        );
        // 每个击杀的 killer/victim 属于不同队伍（镜像守恒）。
        for k in &out.event.kills {
            assert_ne!(k.killer.team, k.victim.team, "击杀者与受害者必不同队");
            assert!(!k.killer.name.is_empty());
            assert!(!k.victim.name.is_empty());
        }
        // 击杀者队 = killer.team，受害者队 = 1 - killer.team。
        for k in &out.event.kills {
            assert_eq!(k.victim.team, 1 - k.killer.team);
        }
    }

    #[test]
    fn live_kill_feed_is_deterministic() {
        // 同一存档同一决策序列 → 同一击杀流（确定性铁律，不扰动回合胜负 RNG）。
        let mut a = LiveMatchEngine::start_map(config());
        let mut b = LiveMatchEngine::start_map(config());
        let out_a = LiveMatchEngine::simulate_next_round(&mut a, None);
        let out_b = LiveMatchEngine::simulate_next_round(&mut b, None);
        assert_eq!(out_a.event.kills, out_b.event.kills, "击杀流必须确定性");

        // 击杀流派生 seed 与回合胜负 RNG 分离：加击杀流不应改变历史 winner/score。
        assert_eq!(out_a.event.winner_a, out_b.event.winner_a);
        assert_eq!(out_a.event.score_a, out_b.event.score_a);
        assert_eq!(out_a.event.score_b, out_b.event.score_b);
    }

    #[test]
    fn live_kill_feed_survives_roundtrip() {
        // 击杀流进入事件存档后序列化往返一致（serde default 空数组兼容旧回合）。
        let mut state = LiveMatchEngine::start_map(config());
        let _ = LiveMatchEngine::simulate_next_round(&mut state, None);
        let json = serde_json::to_string(&state).unwrap();
        let restored: LiveRoundState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.round_history, state.round_history);
        // 旧存档（无 kills 字段的回合）读入补空数组。
        let legacy = r#"{"round_number":1,"phase":"ROUND_END","event_kind":"RoundPlayed","winner_a":true,"score_a":1,"score_b":0,"side_a":"T","side_b":"CT"}"#;
        let old: LiveRoundEvent = serde_json::from_str(legacy).unwrap();
        assert!(old.kills.is_empty(), "旧回合 kills 应补空数组");
    }
}
