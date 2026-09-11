# 04 · 规则层：csc-simulation（纯函数规则）

> 第 2 层：**一切数值公式的所在地**——无状态、纯函数、无跨调用状态。
> 系统引擎（csc-systems / csc-tournaments）只做编排与状态变更，公式全在本 crate。
> 确定性要求：随机消费序固定（跨语言可复现），布尔判定走 `roll_bp`。

---

## 一、胜率：`WinRateCalculator`（src/win_rate.rs）

```
胜率 = sigmoid(K × tier_factor × (当场合力差 + 心态差×0.20 + 结构差×50))，clamp [0.05, 0.95]
```

### 常量

| 常量 | 值 | 说明 |
|---|---|---|
| `K` | `0.030` | sigmoid 陡峭度 |
| `MENTALITY_WEIGHT` | `0.20` | 心态权重 |
| `STRUCTURE_SCALE` | `50.0` | 结构因子放大 |

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `tier_factor` | `pub fn tier_factor(tier: TourneyTier) -> f64` | Major 1.35 / SuperElite 1.20 / Elite 1.10 / T1 1.05 / T2 0.90 / Qualify 0.75 |
| `win_rate` | `pub fn win_rate(self_roster: &[&PlayerCharacter], opponent: &[&PlayerCharacter], tier: TourneyTier, rng: &mut Xoshiro256StarStar, directives_self: Option<MatchDirectives>, directives_opp: Option<MatchDirectives>, structure_self: f64, structure_opp: f64) -> f64` | 双方胜率 |

### 自由函数：`match_power`

`pub fn match_power(roster: &[&PlayerCharacter], rng: &mut Xoshiro256StarStar) -> f64`
——每选手按 σ 采样单场实际发挥（`VolatilityModel::actual_power`）+ 体况折扣（`ConditionModel::effective_power`）。

**随机消耗序**：我方每选手 1 个 gaussian（=2 nextDouble）→ 对手每选手 1 个 gaussian。

---

## 二、Rating：`RatingCalculator`（src/rating.rs）

选手 Rating 计算（HLTV 2.0 风格）：

### Struct：`RoundStats` / `MatchStats`

- `RoundStats { kpr, dpr, kast, adr, apr }`（**每回合指标**，rating.rs:5-16）；
- `MatchStats { kills, deaths, assists, rounds_played, ... }`（整场数据，rating.rs:20-30）+ `to_round_stats()`；
- `impact` 是**方法**（`pub fn impact(kpr, apr)`）非字段。

### 常量（HLTV 2.0 权重）

`IMPACT_KPR_WEIGHT=2.13 / IMPACT_APR_WEIGHT=0.42 / IMPACT_OFFSET=0.41`；
`WEIGHT_KAST=0.00738764 / WEIGHT_KPR=0.35912389 / WEIGHT_DPR=-0.5329508 /
WEIGHT_IMPACT=0.2372603 / WEIGHT_ADR=0.0032397 / RATING_BASE=0.1587 / RATING_ERROR_CORRECTION=0.01`；
`PRO_PLAYER_RATINGS`（43 名真实职业选手参考 Rating）。

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `impact` | `pub fn impact(kpr: f64, apr: f64) -> f64` | 影响力 |
| `rating_of_round` | `pub fn rating_of_round(kpr, dpr, kast, adr, apr) -> f64` | 单图 Rating |
| `rating_of` | `pub fn rating_of(round: &RoundStats) -> f64` | RoundStats → Rating |
| `rating_of_match` | `pub fn rating_of_match(match_stats: &MatchStats) -> f64` | 整场 Rating |
| `rating_of_counts` | `pub fn rating_of_counts(kills, deaths, assists, rounds) -> f64` | 计数 → Rating |
| `adr_of` / `kast_of` | 派生统计 | ADR / KAST |
| `nearest_pro_player` | `pub fn nearest_pro_player(target: f64) -> &'static str` | 最近参考选手（解说用） |

---

## 三、成长：`GrowthModel`（src/growth.rs）

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `DECLINE_START_AGE` | `25` | 下滑起始年龄 |
| `development_rate` | `pub fn development_rate(age: i32) -> f64` | 年龄曲线（18-22 上升，25+ 下滑） |
| `decline_probability` | `pub fn decline_probability(pc: &PlayerCharacter) -> f64` | 衰退概率（年龄/潜力判定） |
| `apply_yearly_growth` | `pub fn apply_yearly_growth(pc: &mut PlayerCharacter, rng: &mut Xoshiro256StarStar) -> bool` | 年度成长（潜力驱动 + 训练加成；返回是否成长） |

## 四、训练：`TrainingModel`（src/training.rs）

### Enum：`TrainingFocus`

`AIM / UTILITY / CLUTCH / PHYSICAL / MENTAL / COMMUNICATION / REST`；
`name() / label() / description() -> &'static str`、`from_name(&str) -> Self`。

### Struct：`TrainingContext` / `TrainingOutcome`

- `TrainingContext { team_cohesion: f64, season_goal: Option<SeasonGoal> }`（`neutral()` 构造默认）；
- `TrainingOutcome { ... }`（训练结果：属性增量/疲劳/伤病风险）。

### 常量

`MONTHLY_GAIN=1 / HARD_WORK_CHANCE=0.15 / FATIGUE_COST=12.0 / REST_RECOVERY=22.0 /
OVERTRAIN_THRESHOLD=80.0 / OVERTRAIN_INJURY_CHANCE=0.06 / ENERGY_SENSITIVITY=0.7 / DIMINISHING_MIN=0.25`。

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `apply_training` | `pub fn apply_training(pc: &mut PlayerCharacter, focus: TrainingFocus, rng: &mut Xoshiro256StarStar, ctx: TrainingContext) -> TrainingOutcome` | 应用训练效果 |
| `auto_focus_for` | `pub fn auto_focus_for(pc: &PlayerCharacter) -> TrainingFocus` | 自动推荐焦点 |

REST 为恢复焦点（疲劳/伤病恢复）；训练计划存 `CareerInfo.pending_training`（存档可复现），
月首由 `season.rs::apply_pending_training` 统一应用。

---

## 五、比赛模拟（两个引擎并存）

### 1. `MatchSimulator`（src/match_simulator.rs）——批量预生成（NPC quick 路径 / 旧回放）

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `MR12_WIN_SCORE` | `13` | 十三胜制 |
| `OVERTIME_TARGET` / `OVERTIME_ROUNDS` | `4` / `8` | 加时 |
| `KILLS_PER_ROUND` / `WIN_KILL_RATE` / `ASSIST_RATE` | `6.8` / `3.5` / `0.25` | 击杀模型 |
| `LOSER_SCORE_MIN` / `LOSER_SCORE_MAX` | `4` / `11` | 败方比分区间 |
| `overtime_chance` | `pub fn overtime_chance(win_prob_a: f64) -> f64` | 加时概率 |
| `simulate_series` | `pub fn simulate_series(team_a: SeriesTeam, team_b: SeriesTeam, tier, best_of, rng) -> SeriesResult` | 完整系列赛 |
| `simulate_quick_series` | `pub fn simulate_quick_series(...)` | 快速系列赛（NPC 快路径） |

### 2. `LiveMatchEngine`（src/live_match.rs）——LIVE 逐回合确定性引擎（Phase 4a）

**核心思想**：比赛未来回合在玩家决策前**不存在**最终结果；每次调用
`simulate_next_round` 恰好生成一个回合事件；只有推进到 `LiveMatchPhase::MapEnd` 后
`finish_map` 才产出 `MapScore`。

**确定性铁律**：每个回合的 RNG 由 `round_seed(world_seed, match_id, map_id, round_number, decision_seq, SIMULATION_VERSION)` 派生——**玩家决策进入 seed 链**。

| 类型/方法 | 说明 |
|---|---|
| `SIMULATION_VERSION: u32 = 1` | LIVE 引擎版本（变更后新 LIVE 用新 seed 链，旧回放仍可读） |
| `LiveRoundState` | 单场 LIVE 运行时事实源（可序列化存档/恢复）：`match_id/series_id/map_id/map_number/phase/team_a/b_id/team_a/b_sig/best_of/world_seed/round_number/decision_seq/rng_counter/score_a/b/base_win_prob_a/economy_a/b/tactical_a/b/players_a/b/round_history` |
| `LiveMapConfig` | 编排层输入的确定性比赛配置：`match_id/series_id/map_id/map_number/team_a/b_id/team_a/b_sig/best_of/world_seed/base_win_prob_a/...` |
| `LivePlayerState` | 玩家运行时状态：`player_id/name/team_id/alive/health/armor/weapon/utility/kills/deaths/assists/confidence` |
| `LiveTacticalState` | 战术/士气解释层：`momentum/confidence/morale/round_stability/economy_pressure/pace/timeout_called` |
| `BombState` | `None / Carried{player} / Planted{site,ticks_left} / Defused / Exploded` |
| `start_map` | `pub fn start_map(config: LiveMapConfig) -> LiveRoundState` |
| `simulate_next_round` | `pub fn simulate_next_round(state: &mut LiveRoundState, decision: Option<LiveRoundDecision>) -> LiveRoundOutput`（live_match.rs:355；`LiveRoundOutput` 含 `event/map_finished/series_finished`；**无 LiveError 类型**） |
| `finish_map` | `pub fn finish_map(state: &mut LiveRoundState) -> MapScore`（live_match.rs:448，assert phase==MapEnd；**非 Option**） |
| `LiveRoundDecision` | 玩家决策（economy buy / util plan / pace / timeout 等） |
| `LiveRoundEvent` | 回合事件（击杀流/胜负/比分） |

---

## 六、经济与财务

### `FinanceModel`（src/finance.rs）

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `SPONSOR_REPUTATION_THRESHOLD` | `70` | 代言声誉门槛 |
| `BRANDS` | 10 个品牌 | 代言品牌池 |
| `prize_share_ratio` | `pub fn prize_share_ratio(placement: i32) -> f64` | 名次 → 奖金份额 |
| `prize_share` | `pub fn prize_share(pool: i64, placement: i32) -> i64` | 奖金分配 |
| `brand_requirement` | `pub fn brand_requirement(index: usize) -> i32` | 品牌声誉要求 |
| `sponsor_annual_value` | `pub fn sponsor_annual_value(reputation: i32) -> i64` | 代言年付 |
| `available_brands` / `top_brand` | 可选/顶级品牌 | — |
| `transfer_fee` | `pub fn transfer_fee(power: f64) -> i64` | 转会费（按实力） |
| `can_afford` | `pub fn can_afford(budget: i64, salary: i64, fee: i64) -> bool` | 负担判定 |
| `monthly_salary` | `pub fn monthly_salary(annual_salary: i64) -> i64` | 月薪 |

### `EconomyBuy` / `LiveEconomy`（csc-domain/src/live.rs）

LIVE 逐回合经济模型：`LiveEconomy { money, equipment: EconomyBuy, timeout_remaining, loss_bonus }`
（live.rs:164-170）；`Eco / HalfBuy / ForceBuy / FullBuy` 是 `EconomyBuy` **枚举变体**（live.rs:139-144），非字段。

---

## 七、其它规则模块

| 模块 | 核心 API | 说明 |
|---|---|---|
| `condition.rs` | `ConditionModel::effective_power(pc, raw_power, resting) -> f64`、`fatigue_cost(maps, lan)`、`fatigue_penalty(fatigue)`、`injury_penalty(injury, resting)`、`injury_chance(age, health, fatigue, injury_prone)`、`roll_kind(rng)`、`roll_severity(rng)` | 体况折扣（疲劳/伤病）+ 伤病掷骰 |
| `volatility.rs` | `VolatilityModel::sigma_of(pc) -> f64`、`actual_power(pc, normal) -> f64`、`age_factor(age)`；`BASE_VOLATILITY=30.0` | 单场发挥波动（神经刀 σ） |
| `form.rs` | `FormModel::apply_series_outcome(pc, won)`、`yearly_reputation_decay(pc)`、`apply_champion_bonus(pc)`、`apply_mvp_bonus(pc)`；常量 `MORALE_WIN_DELTA=2 / MORALE_LOSS_DELTA=-3 / REPUTATION_CHAMPION_BONUS=3 / REPUTATION_MVP_BONUS=6 / REPUTATION_YEARLY_DECAY=0.85` | 士气/声誉变化 |
| `chemistry_model.rs` | `BlunderReaction`；`ChemistryModel` 7 方法：`reaction_of/relation_delta/teammate_morale_delta/self_morale_delta/roll_blunder_kind/cohesion_of/cohesion_factor`（chemistry_model.rs:48-101） | 队友失误反应模型 |
| `kills_alloc.rs` | `allocate_kills(total, weights, rng) -> Vec<i32>`（2026-09 提取） | 击杀按权重轮盘分配**唯一事实源**（历史上 `live_match.rs` 与 `match_simulator.rs` 各藏一份逐字符相同的拷贝，Jaccard=1.00） |
| `mark_effects.rs` | `MarkEffects` 5 方法：`power_bonus/win_rate_modifier/salary_multiplier/injury_prone_strength/default_playstyle`（mark_effects.rs:12-40） | 印记对属性的加成 |
| `directives.rs` | `MatchDirectives { style: Playstyle, bonus: i32 }`、`Playstyle`、`BlunderKind` | 场内指令值对象 |
| `series.rs` | `SeriesResult/SeriesStage/MapScore/PlayerLine` | 系列赛值对象 |
| `replay.rs` | `RoundReplay`(:109)/`MapReplay`(:146)/`MatchReplay`(:169)/`ReplayBuilder::build(series)`(:235)——**无 ReplayFrame** | 直播回放帧生成 |
| `transfer_rules.rs` | `TransferRules::fee/affordability/...`；`power_of`(:78) 便捷入口 | 转会费/可行性规则 |
| `calibration.rs` | `EcoCalibrator`(:119)/`CalibrationProfile`(:67)/`CalibrationReport`(:89)——**无 CalibrationModel** | 校准资产（roles_baseline/player_ratings/rating_profile）解析 |

---

## 八、设计要点

1. **纯函数**：所有模型无 `self` 状态或仅静态方法——公式可脱离引擎单独单测；
2. **随机消费序固定**：win_rate 的 gaussian 消费序在注释中显式声明（跨语言可复现的关键）；
3. **整数化概率**：布尔判定走 `roll_bp`，连续值抽样才用 `next_double`；
4. **两套比赛引擎**：批量 `MatchSimulator`（NPC 快路径）与逐回合 `LiveMatchEngine`
   （玩家 LIVE，决策进 seed 链）并存，职责边界清晰。
