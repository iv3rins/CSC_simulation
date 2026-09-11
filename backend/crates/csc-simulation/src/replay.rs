//! 回合级比赛回放生成器（Watch/Play Match 模块的确定性数据源）。
//!
//! 架构契约（与 csc-simulation 纯函数层一致）：
//! - **零 IO**：消费 [`SeriesResult`]，产出 [`MatchReplay`]（serde 可序列化）；
//! - **不消耗主 RNG**：回放种子 = `fnv1a64(serde_json(SeriesResult))`——同一场
//!   比赛永远生成同一回放（存档加载/多次请求逐位一致），且对既有模拟的
//!   RNG 消耗顺序**零影响**（golden 测试不受扰动）；
//! - **播放模式无关**：沉浸式（逐回合文字直播）/ 交互式（快进+高光卡）/
//!   纯模拟（一键出结果）只是回放数据的消费方式差异，比分与战绩永远
//!   以 [`SeriesResult`] 为准——回放层不产生任何新的比赛事实。
//!
//! 生成内容（HLTV 布局对齐）：
//! - 地图 BP 流程（bo1 单图 / bo3 与 bo5 的 ban-pick 序列，2026 真实 7 图池）；
//! - 逐回合胜负序列（**逐步剩余采样**，严格收敛到最终比分，含半场/加时切分）；
//! - 每回合经济类型近似（手枪局 / ECO / 强起 / 全买）与击杀事件流
//!   （杀手/受害者/武器/首杀/自杀），事件级高光（残局/四杀/Ace/AWP 连杀）；
//! - 图级与系列级高光（翻盘 / 图点 / 赛点）索引（交互式模式的高光卡数据源）。

use serde::{Deserialize, Serialize};

use csc_util::pick_index;
use csc_util::rng::Xoshiro256StarStar;

use crate::series::SeriesResult;

/// 2026 真实竞技图池（HLTV Esports World Cup 2026 BP 实况：Anubis/Ancient/
/// Dust2/Inferno/Mirage/Nuke/Cache）。
pub const MAP_POOL_2026: [&str; 7] = [
    "Anubis", "Ancient", "Dust2", "Inferno", "Mirage", "Nuke", "Cache",
];

/// MR12 制：前 12 回合为上半场（下半场 13..=24；加时每轮 8 回合）。
const HALF_ROUNDS: i32 = 12;

/// 回合经济类型（近似模型：手枪局/ECO/强起/半起/全买；纯叙事标注）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RoundType {
    /// 手枪局（每半场/每轮加时首回合）
    Pistol,
    /// 经济局（省枪存钱）
    Eco,
    /// 半起（手枪+少量道具）
    HalfBuy,
    /// 强起（起枪但无甲/少甲）
    Force,
    /// 全买（满装备）
    FullBuy,
}

/// 武器类别（叙事分类；真实武器名由前端本地化渲染）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WeaponKind {
    Rifle,
    Awp,
    Pistol,
    Smg,
    Shotgun,
    Grenade,
    Knife,
}

/// 单条回合事件（击杀/自杀；回合首杀标记）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundEvent {
    /// 杀手昵称（None = 自杀/环境）
    pub killer: Option<String>,
    /// 受害者昵称
    pub victim: Option<String>,
    pub weapon: WeaponKind,
    /// 自杀事件（killer 为 None）
    pub suicide: bool,
    /// 本回合首杀
    pub first_kill: bool,
}

/// 高光类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HighlightKind {
    /// 残局（回合末段 1v2+ 连续击杀翻盘）
    Clutch,
    /// 单回合四杀
    QuadKill,
    /// 单回合五杀
    Ace,
    /// 单回合 2+ AWP 击杀
    AwpSpree,
    /// 落后 ≥4 分后翻盘赢下图
    Comeback,
    /// 图点（胜方拿到第 12 分）
    MapPoint,
    /// 赛点（系列赛胜方赢下决胜图）
    MatchPoint,
}

/// 高光（挂在具体回合；`player` = 主角选手昵称，None = 图级/系列级）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub kind: HighlightKind,
    pub player: Option<String>,
    /// 结构化细节（clutch 的 1vX 人数 / 连杀数 / AWP 击杀数）
    pub detail: i32,
}

/// 单回合回放。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundReplay {
    /// 图内回合号（从 1 开始）
    pub round_no: i32,
    /// 本回合胜方签名
    pub winner_sig: String,
    /// 本回合后双方比分
    pub a_score: i32,
    pub b_score: i32,
    /// 半场：1 = 上半场（1..=12），2 = 下半场（13..=24），3+ = 加时轮
    pub half: i32,
    pub round_type: RoundType,
    /// 击杀事件流（5..8 条，叙事层）
    pub events: Vec<RoundEvent>,
    /// 本回合高光（无则 None）
    pub highlight: Option<Highlight>,
}

/// BP 动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VetoAction {
    Pick,
    Ban,
    Decider,
}

/// 一步 BP。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VetoStep {
    /// 执行方签名（Decider 无执行方 = 剩图）
    pub team_sig: Option<String>,
    pub action: VetoAction,
    pub map: String,
}

/// 单图回放。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapReplay {
    pub map_number: i32,
    pub map_name: String,
    /// 本图的 pick/decider 步骤（ban 步骤仅供 BP 全流程展示，见 `veto` 全量）
    pub veto: Vec<VetoStep>,
    pub rounds: Vec<RoundReplay>,
    pub a_score: i32,
    pub b_score: i32,
    /// 该图胜方签名
    pub winner_sig: String,
}

/// 高光索引（交互式模式：按播放进度弹高光卡）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighlightRef {
    pub map_number: i32,
    pub round_no: i32,
    pub kind: HighlightKind,
    pub player: Option<String>,
}

/// 整场系列赛的回合级回放。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchReplay {
    pub best_of: i32,
    /// BP 全流程（HLTV 风格 7 步；含 ban 与 decider）
    pub veto: Vec<VetoStep>,
    pub maps: Vec<MapReplay>,
    /// 全系列高光索引（按播放顺序）
    pub highlights: Vec<HighlightRef>,
}

// —— 确定性种子 ——

/// FNV-1a 64（跨平台稳定；供回放种子派生）。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 回放种子输入：只包含比赛事实字段。
///
/// LIVE 反馈是展示/决策记录，不应改变既有回合回放；显式投影也保留旧版
/// `SeriesResult` JSON 字段顺序，避免新增协议字段漂移 golden replay。
#[derive(serde::Serialize)]
struct ReplaySeedInput<'a> {
    tier: csc_domain::tourney_tier::TourneyTier,
    team_a_id: csc_util::id::TeamId,
    team_b_id: csc_util::id::TeamId,
    team_a_sig: &'a str,
    team_b_sig: &'a str,
    best_of: i32,
    maps: &'a [crate::series::MapScore],
    winner_sig: &'a str,
    loser_sig: &'a str,
    stage: crate::series::SeriesStage,
    replayable: bool,
}

/// 回放种子 = 系列赛事实的 FNV 哈希（同一场比赛 → 同一回放）。
fn replay_seed(series: &SeriesResult) -> u64 {
    let input = ReplaySeedInput {
        tier: series.tier,
        team_a_id: series.team_a_id,
        team_b_id: series.team_b_id,
        team_a_sig: &series.team_a_sig,
        team_b_sig: &series.team_b_sig,
        best_of: series.best_of,
        maps: &series.maps,
        winner_sig: &series.winner_sig,
        loser_sig: &series.loser_sig,
        stage: series.stage,
        replayable: series.replayable,
    };
    let bytes = serde_json::to_vec(&input).unwrap_or_default();
    fnv1a64(&bytes)
}

// —— 生成器 ——

/// 回合级回放生成器——纯函数，确定性。
pub struct ReplayBuilder;

impl ReplayBuilder {
    /// 从系列赛结果生成整场回放（best_of 1/3/5）。
    pub fn build(series: &SeriesResult) -> MatchReplay {
        let mut rng = Xoshiro256StarStar::seed(replay_seed(series));
        let a_first = rng.next_bool(); // true = A 先手 BP
        let veto = Self::veto_flow(series, a_first);
        // veto 中 pick/decider 依序即为地图顺序
        let map_names: Vec<String> = veto
            .iter()
            .filter(|s| s.action != VetoAction::Ban)
            .map(|s| s.map.clone())
            .collect();

        let mut maps = Vec::new();
        let mut highlights = Vec::new();
        let mut a_wins = 0;
        let mut b_wins = 0;
        let target = series.best_of / 2 + 1;

        for ms in &series.maps {
            let name = map_names
                .get(ms.map_number as usize - 1)
                .cloned()
                .unwrap_or_else(|| Self::random_map(&mut rng));
            let map_veto = veto
                .iter()
                .filter(|s| s.map == name && s.action != VetoAction::Ban)
                .cloned()
                .collect();
            let map = Self::build_map(
                series,
                MapCtx {
                    map_number: ms.map_number,
                    map_name: &name,
                    map_veto,
                    a_won_before: a_wins,
                    b_won_before: b_wins,
                    target,
                },
                &mut rng,
            );
            if map.map.winner_sig == series.team_a_sig {
                a_wins += 1;
            } else {
                b_wins += 1;
            }
            highlights.extend(map.highlights.iter().cloned());
            maps.push(map.map);
        }

        MatchReplay {
            best_of: series.best_of,
            veto,
            maps,
            highlights,
        }
    }

    /// 地图 BP 流程（HLTV 风格）。
    ///
    /// - bo1：先手队随机选 1 图；
    /// - bo3：A ban → B ban → A pick → B pick → A ban → B ban → 剩图 decider；
    /// - bo5：A ban → B ban → A ban → B ban → A pick → B pick → 剩图 decider。
    fn veto_flow(series: &SeriesResult, a_first: bool) -> Vec<VetoStep> {
        let mut pool: Vec<String> = MAP_POOL_2026.iter().map(|s| s.to_string()).collect();
        let (a_sig, b_sig) = if a_first {
            (series.team_a_sig.clone(), series.team_b_sig.clone())
        } else {
            (series.team_b_sig.clone(), series.team_a_sig.clone())
        };
        // BP 流程自身也需确定性随机（与回放主体共享种子派生，但独立 RNG 流）
        let mut rng = Xoshiro256StarStar::seed(replay_seed(series) ^ 0x9e37_79b9_7f4a_7c15);
        let take = |pool: &mut Vec<String>, rng: &mut Xoshiro256StarStar| {
            let i = rng.next_i32_bound(pool.len() as i32) as usize;
            pool.swap_remove(i)
        };

        let mut steps = Vec::new();
        match series.best_of {
            1 => steps.push(VetoStep {
                team_sig: Some(a_sig),
                action: VetoAction::Pick,
                map: take(&mut pool, &mut rng),
            }),
            3 => {
                let plan: [(String, VetoAction); 6] = [
                    (a_sig.clone(), VetoAction::Ban),
                    (b_sig.clone(), VetoAction::Ban),
                    (a_sig.clone(), VetoAction::Pick),
                    (b_sig.clone(), VetoAction::Pick),
                    (a_sig.clone(), VetoAction::Ban),
                    (b_sig.clone(), VetoAction::Ban),
                ];
                for (team, action) in plan {
                    steps.push(VetoStep {
                        team_sig: Some(team),
                        action,
                        map: take(&mut pool, &mut rng),
                    });
                }
                steps.push(VetoStep {
                    team_sig: None,
                    action: VetoAction::Decider,
                    map: pool.pop().expect("bo3 应剩 1 图"),
                });
            }
            5 => {
                let plan: [(String, VetoAction); 6] = [
                    (a_sig.clone(), VetoAction::Ban),
                    (b_sig.clone(), VetoAction::Ban),
                    (a_sig.clone(), VetoAction::Ban),
                    (b_sig.clone(), VetoAction::Ban),
                    (a_sig.clone(), VetoAction::Pick),
                    (b_sig.clone(), VetoAction::Pick),
                ];
                for (team, action) in plan {
                    steps.push(VetoStep {
                        team_sig: Some(team),
                        action,
                        map: take(&mut pool, &mut rng),
                    });
                }
                steps.push(VetoStep {
                    team_sig: None,
                    action: VetoAction::Decider,
                    map: pool.pop().expect("bo5 应剩 1 图"),
                });
            }
            _ => {}
        }
        steps
    }

    fn random_map(rng: &mut Xoshiro256StarStar) -> String {
        MAP_POOL_2026[rng.next_i32_bound(MAP_POOL_2026.len() as i32) as usize].to_string()
    }

    /// 构建单图回放：回合序列（严格收敛到最终比分）+ 事件流 + 高光。
    fn build_map(series: &SeriesResult, ctx: MapCtx, rng: &mut Xoshiro256StarStar) -> BuiltMap {
        let ms = &series.maps[ctx.map_number as usize - 1];
        let total = ms.team_a_score + ms.team_b_score;
        let final_winner_a = ms.team_a_score > ms.team_b_score;
        let mut a_left = ms.team_a_score;
        let mut b_left = ms.team_b_score;
        let mut a_now = 0;
        let mut b_now = 0;
        let mut max_deficit = 0i32; // 最终胜方曾落后最大分差
        let mut comeback_round: Option<i32> = None;

        let mut rounds = Vec::with_capacity(total as usize);
        for round_no in 1..=total {
            let p = a_left as f64 / (a_left + b_left) as f64;
            let a_wins_round = rng.roll_bp(p);
            if a_wins_round {
                a_left -= 1;
                a_now += 1;
            } else {
                b_left -= 1;
                b_now += 1;
            }

            // 翻盘追踪：最终胜方的落后分差（负数 = 领先）
            let deficit = if final_winner_a {
                b_now - a_now
            } else {
                a_now - b_now
            };
            if deficit > max_deficit {
                max_deficit = deficit;
            }
            if max_deficit >= 4 && deficit <= 0 && comeback_round.is_none() {
                comeback_round = Some(round_no); // 落后 4+ 后首次追平
            }

            let half = Self::half_of(round_no);
            let round_type = Self::round_type_of(round_no, a_wins_round, &rounds, rng);
            let winner_sig = if a_wins_round {
                series.team_a_sig.clone()
            } else {
                series.team_b_sig.clone()
            };
            let events = Self::round_events(series, &ms.lines, a_wins_round, rng);
            let highlight = Self::round_highlight(&events);
            rounds.push(RoundReplay {
                round_no,
                winner_sig,
                a_score: a_now,
                b_score: b_now,
                half,
                round_type,
                events,
                highlight,
            });
        }

        // 图级/系列级高光
        let mut highlights: Vec<HighlightRef> = Vec::new();
        // 图点：最终胜方拿到第 12 分的回合
        for r in &rounds {
            let score = if final_winner_a { r.a_score } else { r.b_score };
            if score == HALF_ROUNDS {
                highlights.push(HighlightRef {
                    map_number: ctx.map_number,
                    round_no: r.round_no,
                    kind: HighlightKind::MapPoint,
                    player: None,
                });
                break;
            }
        }
        // 翻盘
        if max_deficit >= 4
            && let Some(rn) = comeback_round
        {
            highlights.push(HighlightRef {
                map_number: ctx.map_number,
                round_no: rn,
                kind: HighlightKind::Comeback,
                player: None,
            });
        }
        // 系列赛点：本图后系列总分达到 target
        let (a_after, b_after) = if final_winner_a {
            (ctx.a_won_before + 1, ctx.b_won_before)
        } else {
            (ctx.a_won_before, ctx.b_won_before + 1)
        };
        if a_after == ctx.target || b_after == ctx.target {
            highlights.push(HighlightRef {
                map_number: ctx.map_number,
                round_no: total,
                kind: HighlightKind::MatchPoint,
                player: None,
            });
        }
        // 事件级高光索引
        for r in &rounds {
            if let Some(h) = &r.highlight {
                highlights.push(HighlightRef {
                    map_number: ctx.map_number,
                    round_no: r.round_no,
                    kind: h.kind,
                    player: h.player.clone(),
                });
            }
        }

        BuiltMap {
            map: MapReplay {
                map_number: ctx.map_number,
                map_name: ctx.map_name.to_string(),
                veto: ctx.map_veto,
                rounds,
                a_score: ms.team_a_score,
                b_score: ms.team_b_score,
                winner_sig: ms.winner_sig.clone(),
            },
            highlights,
        }
    }

    /// 半场号：1..=12 上半场；13..=24 下半场；加时每 8 回合一轮。
    fn half_of(round_no: i32) -> i32 {
        if round_no <= HALF_ROUNDS {
            1
        } else if round_no <= 2 * HALF_ROUNDS {
            2
        } else {
            3 + (round_no - 2 * HALF_ROUNDS - 1) / 8
        }
    }

    /// 回合经济类型近似：半场/加时首回合 = 手枪局；
    /// 败方按连败局数 ECO(1) → 半起/强起(2) → 全买(≥3)；胜方全买。
    fn round_type_of(
        round_no: i32,
        a_wins_round: bool,
        rounds: &[RoundReplay],
        rng: &mut Xoshiro256StarStar,
    ) -> RoundType {
        let pistol = round_no == 1
            || round_no == HALF_ROUNDS + 1
            || (round_no > 2 * HALF_ROUNDS && (round_no - 2 * HALF_ROUNDS - 1) % 8 == 0);
        if pistol {
            return RoundType::Pistol;
        }
        // 败方连败局数：往前数「上一回合赢家 == 本回合败方」的连续局数
        let loser_is_a = !a_wins_round;
        let mut losses = 0;
        for r in rounds.iter().rev() {
            let prev_a_won = r.a_score > r.b_score;
            // 上一回合赢家是 A ⇔ 本回合败方是 B（连败延续）；反之亦然
            if prev_a_won != loser_is_a {
                losses += 1;
            } else {
                break;
            }
        }
        match losses {
            0 => RoundType::FullBuy,
            1 => {
                if rng.roll_bp(0.25) {
                    RoundType::Force
                } else {
                    RoundType::Eco
                }
            }
            2 => {
                if rng.roll_bp(0.35) {
                    RoundType::FullBuy
                } else if rng.roll_bp(0.5) {
                    RoundType::HalfBuy
                } else {
                    RoundType::Force
                }
            }
            _ => RoundType::FullBuy,
        }
    }

    /// 一回合击杀事件流：5..8 条；杀手/受害者按双方总 KDA 权重采样；
    /// 自杀罕见（0.6%）；首杀标记第一条。
    fn round_events(
        series: &SeriesResult,
        lines: &[crate::series::PlayerLine],
        a_wins_round: bool,
        rng: &mut Xoshiro256StarStar,
    ) -> Vec<RoundEvent> {
        if lines.len() < 10 {
            // 无战绩数据（不应发生：回放仅玩家对局）——空事件
            return Vec::new();
        }
        let n = 5 + rng.next_i32_bound(4); // 5..=8
        let a_lines = &lines[..5];
        let b_lines = &lines[5..];
        // 杀手权重：本队总击杀（胜方 x1.3，让赢队明星更出彩）
        let winner_is_a = if a_wins_round {
            series.team_a_sig.clone()
        } else {
            series.team_b_sig.clone()
        };
        let _ = winner_is_a;
        let a_mult = if a_wins_round { 1.3 } else { 1.0 };
        let b_mult = if a_wins_round { 1.0 } else { 1.3 };
        let a_kill_w: Vec<f64> = a_lines
            .iter()
            .map(|l| (l.kills as f64 + 1.0) * a_mult)
            .collect();
        let b_kill_w: Vec<f64> = b_lines
            .iter()
            .map(|l| (l.kills as f64 + 1.0) * b_mult)
            .collect();
        let a_sum: f64 = a_kill_w.iter().sum();
        let b_sum: f64 = b_kill_w.iter().sum();
        // 受害者权重：死亡分布（+0.6 平权防 0 死板凳）
        let a_death_w: Vec<f64> = a_lines.iter().map(|l| l.deaths as f64 + 0.6).collect();
        let b_death_w: Vec<f64> = b_lines.iter().map(|l| l.deaths as f64 + 0.6).collect();
        let a_dsum: f64 = a_death_w.iter().sum();
        let b_dsum: f64 = b_death_w.iter().sum();

        let mut events = Vec::with_capacity(n as usize);
        for i in 0..n {
            let killer_is_a = rng.roll_bp(0.5);
            let (killer, victim) = if killer_is_a {
                (
                    a_lines[pick_index(&a_kill_w, a_sum, rng)]
                        .player_name
                        .clone(),
                    b_lines[pick_index(&b_death_w, b_dsum, rng)]
                        .player_name
                        .clone(),
                )
            } else {
                (
                    b_lines[pick_index(&b_kill_w, b_sum, rng)]
                        .player_name
                        .clone(),
                    a_lines[pick_index(&a_death_w, a_dsum, rng)]
                        .player_name
                        .clone(),
                )
            };
            let suicide = rng.roll_bp(0.006);
            events.push(RoundEvent {
                killer: if suicide { None } else { Some(killer) },
                victim: if suicide { None } else { Some(victim) },
                weapon: Self::weapon_of(rng),
                suicide,
                first_kill: i == 0,
            });
        }
        events
    }

    fn weapon_of(rng: &mut Xoshiro256StarStar) -> WeaponKind {
        const W: [(f64, WeaponKind); 7] = [
            (0.58, WeaponKind::Rifle),
            (0.15, WeaponKind::Awp),
            (0.10, WeaponKind::Pistol),
            (0.07, WeaponKind::Smg),
            (0.03, WeaponKind::Shotgun),
            (0.05, WeaponKind::Grenade),
            (0.02, WeaponKind::Knife),
        ];
        let r = rng.next_double();
        let mut acc = 0.0;
        for (p, kind) in W {
            acc += p;
            if r < acc {
                return kind;
            }
        }
        WeaponKind::Rifle
    }

    /// 回合内事件级高光：Ace > QuadKill > Clutch > AwpSpree（每回合至多一个）。
    fn round_highlight(events: &[RoundEvent]) -> Option<Highlight> {
        let mut counts: Vec<(&str, i32)> = Vec::new();
        let mut awp_hits = 0;
        for e in events {
            if let Some(k) = e.killer.as_deref() {
                if let Some(c) = counts.iter_mut().find(|(n, _)| *n == k) {
                    c.1 += 1;
                } else {
                    counts.push((k, 1));
                }
            }
            if e.weapon == WeaponKind::Awp && !e.suicide {
                awp_hits += 1;
            }
        }
        let (player, kills) = counts.iter().max_by_key(|(_, c)| *c)?;
        // 残局：回合末段同杀手连续击杀 ≥2 且回合击杀 ≥6（人少残局感）
        let late_clutch = {
            let last_k = events
                .iter()
                .rev()
                .find(|e| !e.suicide)
                .and_then(|e| e.killer.clone());
            if let Some(lk) = last_k {
                let tail = events
                    .iter()
                    .rev()
                    .take_while(|e| !e.suicide && e.killer.as_deref() == Some(lk.as_str()))
                    .count();
                tail >= 2 && events.len() >= 6
            } else {
                false
            }
        };
        if *kills >= 5 {
            Some(Highlight {
                kind: HighlightKind::Ace,
                player: Some(player.to_string()),
                detail: *kills,
            })
        } else if *kills >= 4 {
            Some(Highlight {
                kind: HighlightKind::QuadKill,
                player: Some(player.to_string()),
                detail: *kills,
            })
        } else if late_clutch {
            Some(Highlight {
                kind: HighlightKind::Clutch,
                player: Some(player.to_string()),
                detail: 2,
            })
        } else if awp_hits >= 3 {
            Some(Highlight {
                kind: HighlightKind::AwpSpree,
                player: Some(player.to_string()),
                detail: awp_hits,
            })
        } else {
            None
        }
    }
}

/// build_map 的中间产物（回放本体 + 该图高光索引）。
struct BuiltMap {
    map: MapReplay,
    highlights: Vec<HighlightRef>,
}

/// 单图回放构建上下文。
struct MapCtx<'a> {
    map_number: i32,
    map_name: &'a str,
    map_veto: Vec<VetoStep>,
    a_won_before: i32,
    b_won_before: i32,
    target: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::{MapScore, PlayerLine};

    fn line(name: &str, team: &str, k: i32, d: i32) -> PlayerLine {
        PlayerLine {
            player_name: name.to_string(),
            team_sig: team.to_string(),
            kills: k,
            deaths: d,
            assists: (k / 4).max(1),
            adr: 60.0 + f64::from(k) * 2.0,
            kast: 60.0 + f64::from(k - d) * 3.0,
        }
    }

    fn map_lines(a_sig: &str, b_sig: &str) -> Vec<PlayerLine> {
        vec![
            line("a1", a_sig, 20, 12),
            line("a2", a_sig, 18, 13),
            line("a3", a_sig, 16, 14),
            line("a4", a_sig, 12, 15),
            line("a5", a_sig, 10, 16),
            line("b1", b_sig, 19, 13),
            line("b2", b_sig, 15, 14),
            line("b3", b_sig, 13, 15),
            line("b4", b_sig, 11, 16),
            line("b5", b_sig, 9, 17),
        ]
    }

    fn series(score_a: i32, score_b: i32, best_of: i32) -> SeriesResult {
        let a_sig = "A | a1,a2,a3,a4,a5".to_string();
        let b_sig = "B | b1,b2,b3,b4,b5".to_string();
        let mk = |no: i32, sa: i32, sb: i32| MapScore {
            map_number: no,
            team_a_score: sa,
            team_b_score: sb,
            winner_sig: if sa > sb {
                a_sig.clone()
            } else {
                b_sig.clone()
            },
            lines: map_lines(&a_sig, &b_sig),
        };
        let maps = if best_of == 1 {
            vec![mk(1, score_a, score_b)]
        } else {
            vec![mk(1, 13, 9), mk(2, 13, 11)]
        };
        SeriesResult {
            tier: csc_domain::tourney_tier::TourneyTier::T1,
            team_a_id: csc_util::id::TeamId(0),
            team_b_id: csc_util::id::TeamId(1),
            team_a_sig: a_sig.clone(),
            team_b_sig: b_sig.clone(),
            best_of,
            maps,
            winner_sig: a_sig,
            loser_sig: b_sig,
            stage: crate::series::SeriesStage::Unknown,
            replayable: true,
            live_feedback: Vec::new(),
            live_states: Vec::new(),
            outcome_analysis: None,
        }
    }

    #[test]
    fn round_scores_converge_to_final() {
        for &(a, b) in &[(13, 7), (13, 11), (16, 14), (13, 0), (19, 17)] {
            let s = series(a, b, 1);
            let replay = ReplayBuilder::build(&s);
            let m = &replay.maps[0];
            assert_eq!(m.a_score, a, "A 终分 {a}:{b}");
            assert_eq!(m.b_score, b, "B 终分 {a}:{b}");
            assert_eq!(m.rounds.len() as i32, a + b, "回合数 = 比分和");
            let last = m.rounds.last().unwrap();
            assert_eq!((last.a_score, last.b_score), (a, b), "末回合比分 = 终分");
            for w in m.rounds.windows(2) {
                assert!(w[1].a_score >= w[0].a_score, "A 比分单调");
                assert!(w[1].b_score >= w[0].b_score, "B 比分单调");
                assert_eq!(
                    w[1].a_score + w[1].b_score,
                    w[0].a_score + w[0].b_score + 1,
                    "每回合恰好 +1 回合"
                );
            }
        }
    }

    #[test]
    fn deterministic_same_series_same_replay() {
        let s = series(13, 7, 1);
        assert_eq!(
            ReplayBuilder::build(&s),
            ReplayBuilder::build(&s),
            "同一比赛必须生成同一回放"
        );
    }

    #[test]
    fn halves_and_round_types() {
        let s = series(13, 9, 1);
        let m = &ReplayBuilder::build(&s).maps[0];
        let half1: Vec<_> = m.rounds.iter().filter(|r| r.half == 1).collect();
        assert_eq!(half1.len(), 12, "上半场 12 回合");
        assert_eq!(m.rounds[0].round_type, RoundType::Pistol, "首回合手枪局");
        assert_eq!(
            m.rounds[12].round_type,
            RoundType::Pistol,
            "下半场首回合手枪局"
        );
        for r in &m.rounds {
            assert!((5..=8).contains(&r.events.len()), "每回合 5..8 事件");
            assert!(r.events[0].first_kill, "首事件标记首杀");
        }
    }

    #[test]
    fn overtime_half_marking() {
        let s = series(16, 14, 1);
        let m = &ReplayBuilder::build(&s).maps[0];
        assert_eq!(m.rounds.len(), 30);
        assert!(m.rounds.iter().any(|r| r.half >= 3), "应有加时轮");
    }

    #[test]
    fn bo3_two_maps_and_veto() {
        let s = series(13, 9, 3);
        let replay = ReplayBuilder::build(&s);
        assert_eq!(replay.maps.len(), 2, "2:0 只打两图");
        assert_ne!(replay.maps[0].map_name, replay.maps[1].map_name);
        // 两图名字都来自 2026 图池
        for m in &replay.maps {
            assert!(MAP_POOL_2026.contains(&m.map_name.as_str()));
        }
    }

    #[test]
    fn bo3_decider_map() {
        let mut s = series(13, 9, 3);
        s.maps = vec![
            MapScore {
                map_number: 1,
                team_a_score: 13,
                team_b_score: 9,
                winner_sig: s.team_a_sig.clone(),
                lines: map_lines(&s.team_a_sig, &s.team_b_sig),
            },
            MapScore {
                map_number: 2,
                team_a_score: 9,
                team_b_score: 13,
                winner_sig: s.team_b_sig.clone(),
                lines: map_lines(&s.team_a_sig, &s.team_b_sig),
            },
            MapScore {
                map_number: 3,
                team_a_score: 13,
                team_b_score: 11,
                winner_sig: s.team_a_sig.clone(),
                lines: map_lines(&s.team_a_sig, &s.team_b_sig),
            },
        ];
        s.winner_sig = s.team_a_sig.clone();
        s.loser_sig = s.team_b_sig.clone();
        let replay = ReplayBuilder::build(&s);
        assert_eq!(replay.maps.len(), 3);
        let names: Vec<&str> = replay.maps.iter().map(|m| m.map_name.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(
            names[0] != names[1] && names[1] != names[2] && names[0] != names[2],
            "三图不重"
        );
        assert!(
            replay.maps[2].rounds.last().unwrap().a_score
                > replay.maps[2].rounds.last().unwrap().b_score,
            "第三图 A 胜"
        );
    }

    #[test]
    fn highlights_present_on_blowout() {
        let s = series(13, 3, 1);
        let replay = ReplayBuilder::build(&s);
        assert!(
            replay
                .highlights
                .iter()
                .any(|h| h.kind == HighlightKind::MapPoint),
            "大胜局应有图点高光"
        );
        assert!(!replay.highlights.is_empty(), "高光索引非空");
    }

    #[test]
    fn serde_roundtrip() {
        let s = series(13, 7, 1);
        let replay = ReplayBuilder::build(&s);
        let json = serde_json::to_string(&replay).unwrap();
        let back: MatchReplay = serde_json::from_str(&json).unwrap();
        assert_eq!(replay, back);
    }

    /// golden：锁死生成器行为（防无意变更导致回放漂移）。
    /// 基准：A 13:7 B（bo1），seed 派生自 series 序列化。
    #[test]
    fn golden_replay_shape() {
        let s = series(13, 7, 1);
        let replay = ReplayBuilder::build(&s);
        let m = &replay.maps[0];
        assert_eq!(m.rounds.len(), 20);
        // 首回合 B 赢（0:1，手枪局）
        assert_eq!(m.rounds[0].a_score, 0);
        assert_eq!(m.rounds[0].b_score, 1);
        assert_eq!(m.rounds[0].winner_sig, s.team_b_sig);
        assert_eq!(m.rounds[0].round_type, RoundType::Pistol);
        // 末回合 13:7
        assert_eq!(m.rounds[19].a_score, 13);
        assert_eq!(m.rounds[19].b_score, 7);
        // 图点挂在胜方第 12 分回合
        assert_eq!(
            replay.highlights[0],
            HighlightRef {
                map_number: 1,
                round_no: 18,
                kind: HighlightKind::MapPoint,
                player: None,
            }
        );
        // 事件结构完整
        for e in &m.rounds[3].events {
            assert!(e.killer.is_some() || e.suicide);
        }
    }

    #[test]
    fn fnv1a_known_vector() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn round_types_follow_economy() {
        // 构造 13:3：开局 B 连输 → A 连赢；B 应出现 ECO/强起，A 基本全买
        let s = series(13, 3, 1);
        let m = &ReplayBuilder::build(&s).maps[0];
        let b_eco = m
            .rounds
            .iter()
            .filter(|r| r.winner_sig.starts_with("A"))
            .any(|r| {
                matches!(
                    r.round_type,
                    RoundType::Eco | RoundType::HalfBuy | RoundType::Force
                )
            });
        assert!(b_eco, "连败方应出现经济局");
    }
}
