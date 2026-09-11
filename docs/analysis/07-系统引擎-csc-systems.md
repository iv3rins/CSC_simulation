# 07 · 系统引擎：csc-systems

> 第 4 层：五+引擎（转会 / 人口 / 伤病 / 化学 / 经济 / 人生事件）。
> 只做**状态变更与编排**，公式全部委托 `csc-simulation`（纯函数规则层）。

---

## 一、Struct：`TransferEngine`（src/transfer/）

### 模块结构

- `transfer/mod.rs` — 引擎门面 + 事件类型；
- `transfer/market.rs` — 转会窗市场生成（挂起报价）；
- `transfer/execution.rs` — 转会执行（双端归属 + 费用 + 事件）；
- `transfer/tests.rs` — 集成测试（`boost_power` / `vrs_with_teams` 共享自 `test_fixtures`）。

### 测试共享 fixture（2026-09 新增）

`src/test_fixtures.rs`（`#[cfg(test)]`，**唯一事实源**）：

| 函数 | 签名 | 说明 |
|---|---|---|
| `boost_power` | `pub fn boost_power(world: &mut World, player_id: PlayerId)` | 玩家属性拉满到 T1 门槛（power ≥ 85） |
| `vrs_with_teams` | `pub fn vrs_with_teams(teams: &[(&str, i32, i32, Vec<&str>)]) -> VrsEngine` | 用 (name, ranking, points, roster) 构造确定性 VrsEngine |

历史上 `npc_market.rs` / `transfer/mod.rs` / `transfer/tests.rs` 各自复制过逐字符相同的
拷贝（risk_diagnose Jaccard=1.00）——一旦单边修改，不同测试会在不同强度选手上断言。
提取后三处测试共享同一实现。

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `generate_offers` | `pub fn generate_offers(world: &World, vrs: &VrsEngine, date: &str) -> Vec<TransferOffer>` | 生成候选（决策批次输入，offer 自带确定性 point_id；在 market.rs） |
| `execute_choices` | `pub fn execute_choices(world: &mut World, vrs: &mut VrsEngine, clock: &SimClock, offers: &[TransferOffer], decisions: &[PlayerDecision], rng: &mut Xoshiro256StarStar) -> Result<Vec<TransferEvent>, SimError>` | 消费决策（STAY / 目标签名）执行转会——**缺 clock/offers 参数、vrs 为 &mut、返回 Result<Vec<TransferEvent>, SimError>**（execution.rs:27-34） |
| `buyout` | `pub fn buyout(...)` | 买断合同（execution.rs） |
| `run_npc_market` | `pub fn run_npc_market(world, vrs, rng, date, window) -> Vec<WorldEvent>` | NPC 转会市场结算（npc_market.rs:35） |
| `refresh_contacts` | `pub fn refresh_contacts(world: &mut World, vrs: &VrsEngine, date: &str)` | 刷新招募情报（transfer_contacts） |
| `contract_summary` | `pub fn contract_summary(world, player_id) -> Result<String, SimError>` | 合同摘要 |
| `market_insight` | `pub fn market_insight(...) -> Result<TransferInsight, SimError>` | 转会市场洞察（结果解释层；market.rs:246-250） |

> ⚠️ `process_transfer_window` **全仓无实现**（仅 mod.rs:9 注释提及，旧文档误列为门面）。

**常量**：`TransferEngine::STAY = "STAY"`（留队选项 id）、`LEAVE = "LEAVE"`（合同到期主动离开）。

### Struct：`TransferEvent`

`{ player_id, player_name, from_team: Option<String>, from_team_id: Option<TeamId>, to_team, to_team_id, contract_years, salary, fee, date }`——转会结果事件。

### Struct：`TransferInsight`

`{ player_name, contract_years, salary, reputation, current_team, current_team_rank, months_unsigned, eligible_teams_full_price, eligible_teams_discounted, explanation }`——市场洞察（为什么有/没有报价）。

### NPC 转会市场

**无 `NpcMarketEngine` struct、无 `tick` 方法**——NPC 市场实际是 `TransferEngine::run_npc_market`（npc_market.rs:35，单函数，无跨调用状态）。

---

## 二、Struct：`PopulationEngine`（src/population.rs）

世界人口（新秀入世 / 退役 / 阵容补齐）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `apply_annual_turnover` | `pub fn apply_annual_turnover(world: &mut World, vrs, rng, year, profile: Option<&RatingProfile>, text) -> Vec<WorldEvent>` | 年度人口更替（新秀入世 + 自然退役 + 阵容补齐，返回事件流） |
| `roster_names` | `pub fn roster_names(world: &World, team_id: TeamId) -> Vec<String>` | 阵容名字（签名组成） |

### Struct：`PopulationEvent` / `RetirementRecord` / `IntakeRecord`

- `PopulationEvent`：人口事件（新秀/退役统一形态）；
- `RetirementRecord { player_id, name, age, from_team }`：退役记录；
- `IntakeRecord { player_id, name, age, into_team }`：新秀入世记录。

---

## 三、Struct：`InjuryEngine`（src/injury.rs）

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `PLAY_THROUGH` / `REST` | `&'static str` | 决策选项 id（带伤上场 / 休养） |
| `monthly_tick` | `pub fn monthly_tick(world: &mut World, rng, batch: &mut Vec<DecisionPoint>, date: &str, journal: Option<&mut WorldJournal>)` | 月度伤病 tick（新伤/恢复；决策点入 `batch`，事件入 `journal`） |
| `apply_decision` | `pub fn apply_decision(world: &mut World, player_id: PlayerId, option_id: &str)` | 应用伤病决策 |

---

## 四、Struct：`ChemistryEngine`（src/chemistry.rs）

队内关系（队友失误反应 / 凝聚力收敛）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `monthly_recovery` | `pub fn monthly_recovery(world: &mut World)` | 关系向默认收敛（时间治愈一切） |
| `apply_blunder_reaction` | `pub fn apply_blunder_reaction(world, player, teammate, reaction: BlunderReaction)` | 队友失误反应效果 |
| `conflict` | `pub fn conflict(world, player, teammate, severity)` | 队内冲突 |
| `cohesion_of` | `pub fn cohesion_of(team) -> f64` | 凝聚力查询 |

---

## 五、Struct：`EconomyEngine`（src/economy.rs）+ `InvestmentEngine`

队伍/个人经济（薪资 / 转会费 / 奖金 / 投资 / 饰品）：

### EconomyEngine

| 方法 | 签名 | 说明 |
|---|---|---|
| `monthly_tick` | `pub fn monthly_tick(world: &mut World, player_id: PlayerId, date: &str, batch: &mut Vec<WorldEvent>, rng: &mut Xoshiro256StarStar)` | 月度收支（**5 参**，economy.rs:49-55） |
| `monthly_income` | `pub fn monthly_income(world: &mut World, player_id)` | 个人月度收入 |
| `monthly_salary` | `pub fn monthly_salary(world: &mut World, player_id, year)` | 发薪 |
| `award_prize_share` | `pub fn award_prize_share(world: &mut World, player_id: PlayerId, pool: i64, placement: i32, year: i32)` | 奖金发放（**按选手 + 奖金池 + 名次 + 年份**，economy.rs:22-28） |
| `annual_sponsor_check` | `pub fn annual_sponsor_check(world, rng, text) -> Vec<WorldEvent>` | 年度代言检查 |
| `apply_sponsor_decision` | `pub fn apply_sponsor_decision(world, player_id, option_id, year, point)` | 代言决策应用（ACCEPT/DECLINE；**5 参含 year/point**，economy.rs:189-195） |

### InvestmentEngine（src/economy.rs 尾部）

| 常量/方法 | 值/签名 | 说明 |
|---|---|---|
| `INVEST_COST` | `50_000` | 投资自己成本 |
| `SKIN_STANDARD_COST` / `SKIN_RARE_COST` | `20_000` / `80_000` | 饰品价格档 |
| `SKIN_STANDARD_POOL` / `SKIN_RARE_POOL` | 12 / 10 个饰品名 | 饰品池 |
| `invest_self` | `pub fn invest_self(world, player_id) -> Result<...>` | 投资自己（属性成长） |
| `buy_skin` | `pub fn buy_skin(world, player_id, param) -> Result<...>` | 购买饰品（随机稀有度） |

> 与 `csc-simulation::finance` 的分工：finance = 玩家个人财务规则（买断/投资/饰品）；
> economy = 队伍经济引擎（月度循环）。

---

## 六、Struct：`LifeEventsEngine`（src/life_events.rs）

场外人生事件（2026 新增，Sprint 5 重复治理）：

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `MONTHLY_CHANCE_PCT` | `72` | 每月触发概率（%），避免决策疲劳 |
| `KIND_COOLDOWN_MONTHS` | `3` | 同类型事件冷却月数 |
| `month_index` | `pub fn month_index(date: &str) -> i32` | 月序（year*12+month） |
| `cooling_down` | `pub fn cooling_down(last_kind, last_month, kind, month) -> bool` | 冷却判定 |
| `collect` | `pub fn collect(world: &mut World, vrs: &VrsEngine, date: &str, rng: &mut Xoshiro256StarStar, text: &TextBundle) -> Vec<DecisionPoint>` | 生成本月人生事件决策点（0 或 1 个） |
| `apply` | `pub fn apply(world: &mut World, decision: &PlayerDecision, rng) -> Vec<WorldEvent>` | 应用决策效果 |

**事件类型**（`LifeEventKind`）：`SecretTeamContact / MatchFixerContact / TeammatePowerStruggle /
TeamContact / Interview / TeammateInvite`。

**设计细节**：
- 按**职业阶段**分事件池（`kinds_for_phase(career_phase(age, reputation))`）——不同年龄/声望段面对不同人生问题；
- 接触方 **TeamId 化**（P2-4）：优先 `transfer_contacts`（含 team_id），空时**恢复 VRS 排名兜底**
  （经 `team_by_name` 反查）——**保持 RNG 消费序稳定**（kinds 长度不因 contacts 有无分叉，防止世界轨迹漂移）；
- 无队友时剔除 TeammatePowerStruggle/TeammateInvite；无接触方时剔除 TeamContact/SecretTeamContact；
- 冷却期保证同类事件不连发。

---

## 七、引擎间协作图（月度批次视角）

```
WorldDecisionBatch::run（csc-core/src/batch.rs，每月月末）
  ├─ InjuryEngine::monthly_tick     → 伤病事件
  ├─ 转会窗（每 4 月）→ TransferEngine::generate_offers → 决策点
  ├─ SponsorshipOffer              → 决策点（声誉门槛触发）
  ├─ LifeEventsEngine::collect     → 人生事件决策点（72% 概率）
  ├─ EconomyEngine::monthly_tick   → 收支
  └─ TransferEngine::run_npc_market → NPC 转会
```

所有决策点 → `DecisionSource` → `DecisionRecorder` → 日志 + 事件镜像。
