# 08 · 赛事系统：csc-tournaments

> 第 5 层：赛事全生命周期（排程 → 邀请 → 逐场 series 模拟 → 结算编排）。
> 只做**编排**：排程委托 scheduler、对局委托 conductor、结算公式全在 settlement（零公式在引擎）。

---

## 一、Struct：`TournamentEngine`（src/engine.rs）

赛事子系统 sub-engine（编排壳）。

### 字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `scheduler` | `TournamentScheduler` | 排程器（排程产物状态） |
| `settlement` | `SeriesSettlement` | 结算层（年度 Rating 累计器等跨调用状态） |
| `events` | `Vec<TournamentResult>` | 已完整模拟的赛事 |
| `last_scheduled` | `Option<ScheduledTournament>` | 最近排程（队伍级互斥锁用） |
| `scheduled_records` | `Vec<ScheduledTournamentRecord>` | 已排程确认赛事权威记录（future 赛程事实源） |

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `new` | `pub fn new() -> Self` | 新建 |
| `restore_from` | `pub fn restore_from(tracker: YearlyRatingTracker, top20_history, events, scheduled_records) -> Self` | 读档恢复 |
| `scheduled_tournaments` | `pub fn scheduled_tournaments(&self) -> Vec<ScheduledTournament>` | 全部排程赛事 |
| `participants_of` | `pub fn participants_of(&self, tier: TourneyTier) -> Vec<TeamId>` | 指定等级参赛队伍 |
| `results` / `results_ref` | `-> Vec<TournamentResult>` / `&[TournamentResult]` | 赛事结果（后者零 clone） |
| `event_count` | `pub fn event_count(&self) -> usize` | 只读计数（高频调用防 clone） |
| `settlement` / `settlement_mut` | 结算层访问器 | — |
| `yearly_rating_snapshot` | `-> YearlyRatingTracker` | 年度累计器快照（存档） |
| `top20_history` | `-> Vec<Top20YearBoard>` | 历届 TOP20 榜单 |
| `restore_settlement` | `pub fn restore_settlement(&mut self, tracker)` | 读档恢复结算层 |
| `scheduled_records_snapshot` | `-> Vec<ScheduledTournamentRecord>` | 排程记录快照 |
| `skip_live_today` | `pub fn skip_live_today(&mut self, team, year, month, day) -> usize` | 标记今日 pending 对阵为 skipped（持久语义，不改结算） |
| `mark_fixture_watched` | `pub fn mark_fixture_watched(&mut self, event_name, fixture_id) -> bool` | 标记已看完（幂等） |
| `simulate_event` | `pub fn simulate_event(&mut self, ctx, decision, rng) -> Result<(), SimError>` | 模拟一场赛事（内部按 format 分派赛制引擎） |
| `plan_event_with_ranking` | `pub fn plan_event_with_ranking(...)` | 按排名排程赛事（engine.rs:372；**无 plan_month 方法**——月排程入口是 `SeasonLoop::plan_month`（csc-core/loop_.rs:84）+ 本引擎排程门面） |

---

## 二、Struct：`TournamentScheduler`（src/scheduler.rs）

赛事排程器（2026 真实赛历密度：25 T1 桶 + 60 T2 + 96 T3）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `new` | `pub fn new() -> Self` | 新建 |
| `schedule_t1` / `schedule_t2` / `schedule_t3` / `schedule_for` | `pub fn schedule_*(&mut self, ...)` | 按等级排程（scheduler.rs:70/153/186/229；**无 schedule_month**） |
| `scheduled_tournaments` | `pub fn scheduled_tournaments(&self) -> Vec<ScheduledTournament>` | 已排程列表 |
| `participants_of` | `pub fn participants_of(&self, tier) -> Vec<TeamId>` | 参赛队 |

### Struct：`RankedTeam`

`{ team_id, signature, ranking }`——排程输入（按 VRS 排名邀请）。

---

## 三、赛制引擎（src/format/）

| 模块 | 类型 | 说明 |
|---|---|---|
| `bracket.rs` | `BracketResult`、`TeamEntry` | 通用淘汰赛抽象 |
| `swiss.rs` | `SwissStage` | 瑞士轮（Major 小组赛：3-0/3-1/3-2 晋级，0-3/1-3/2-3 出局） |
| `swiss_playoff.rs` | `SwissPlayoffBracket` | 瑞士轮 + 季后赛组合 |
| `double_elim.rs` | `DoubleElimGroupsBracket` | 双败淘汰（分组双败） |
| `single_elim.rs` | `SingleElimPlayoff` | 单败淘汰 |
| `mod.rs` | — | 赛制分派 |
| `tests_common.rs` | `runner_a_wins()`（`#[cfg(test)]`，2026-09 新增） | 赛制测试共享 runner：固定 A 胜的伪造 MatchRunner（**唯一事实源**）。历史上 double_elim/swiss/swiss_playoff/single_elim 各自内嵌几乎逐字符相同的 `runner()` 拷贝（Jaccard 高达 1.00） |

**对局执行**：每场 series 经 `MatchRunner` 注入 `conductor::run_lod_series_with_importance` 闭包
（engine.rs:723/754/793）——对局级 LOD + 玩家队伍的图间决策循环
（MatchIntervention/TeammateBlunder 全在 conductor，引擎零感知）；
`run_lod_series` 是默认 importance 包装（conductor.rs:45-83）。

---

## 四、Struct：`SeriesSettlement`（src/settlement.rs）

结算层（生涯回写 / 疲劳 / 奖金 / 荣誉 / 年度累计）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `settle_series` | `pub fn settle_series(ctx, result: &SeriesResult, rng)` | 系列赛结算（生涯回写/疲劳/奖金） |
| `settle_quick_series` | `pub fn settle_quick_series(...)` | 快速系列赛结算（NPC 路径） |
| `award_event_honours` | `pub fn award_event_honours(...)` | 赛事荣誉（MVP/EVP/最佳阵容） |
| `award_prize_money` | `pub fn award_prize_money(...)` | 奖金发放 |
| `record_participation` | `pub fn record_participation(...)` | 参赛记录 |
| `yearly_rating_snapshot` | `-> YearlyRatingTracker` | 年度累计器快照 |
| `record_honour` | `pub fn record_honour(player, is_mvp, points)` | 荣誉积分（MVP=1.0/EVP≤0.5 Rating 等值单位） |
| `restore_yearly_rating` / `restore_top20_history` | 读档 | — |

---

## 五、Struct：`Top20Evaluator`（src/top20.rs）

年度选手 TOP20 榜单：

| 常量/方法 | 签名 | 说明 |
|---|---|---|
| `WEIGHT_RATING / WEIGHT_HONOR / WEIGHT_PLAYOFF` | `0.60 / 0.15 / 0.25` | 权重（top20.rs:7-9 头部注释与常量一致，C2 已闭环） |
| `evaluate` | `pub fn evaluate(world: &World, tracker: &YearlyRatingTracker, year: i32) -> Vec<Top20Score>` | 计算年度候选得分 |
| `evaluate_with_wildcards` | `pub fn evaluate_with_wildcards(...) -> Top20YearBoard` | **活代码**：外卡机制完整实现（top20.rs:222-306），被 query.rs:276 与 settlement.rs:420 生产调用；`wildcard_hit`(:169) 被 :253 调用；`WILDCARD_*` 常量 :111-120 全部参与；测试断言 :707/:847（C1 已过时） |
| `commentary` | `pub fn commentary(entry) -> Top20Commentary` | 入选理由文案 |

### Struct：`Top20Score` / `Top20BoardEntry` / `Top20YearBoard`

- `Top20Score { player_id, score, weighted_rating, honor_points, playoff_rating, maps, mvp_count, evp_count, wildcard }`；
- `Top20BoardEntry { rank, player_id, player_name, score, weighted_rating, honor_points, playoff_rating, maps, mvp_count, evp_count, wildcard }`；
- `Top20YearBoard { year, entries }`。

> ✅ **C1/C2 已闭环（2026-08 工作树）**：外卡机制是**活代码**（非半死代码）；
> 权重注释 0.60/0.15/0.25 与常量一致；`routes/mod.rs:660` 的 `filter(|e| !e.wildcard)`
> 有业务语义（统计正常选手数判定跨年回退），非无效过滤。

---

## 六、其它模块

| 模块 | 说明 |
|---|---|
| `calendar.rs` | `SeasonCalendar`（月份 → 赛事模板 + 时间片；排程蓝本） |
| `scheduled.rs` | `ScheduledTournament` / `ScheduledTournamentRecord` / `ScheduledStatus`（future/active/completed/cancelled）/ `FixtureStatus` / `TournamentResult` / `ScheduledFixture`（含 watched/skipped/score） |
| `conductor.rs` | `run_lod_series` / `run_lod_series_with_importance`——对局级 LOD 编排（1256 行，最复杂编排模块） |
| `honours.rs` | 荣誉体系（年度 MVP/EVP/最佳阵容） |
| `invite_model.rs` | 邀请模型（直邀 + 预选；基点阈值纯函数） |
| `yearly_rating.rs` | `YearlyRatingTracker`（跨月累计：rating_all/maps_all/honor_points） |
| `top20.rs` | 见上 |
| `settlement.rs` | 见上 |

---

## 七、赛事状态机（scheduled.rs）

```
ScheduledStatus:
  Future（已排程待开赛）→ Active（进行中）→ Completed（已完赛）
                                    ↘ Cancelled（参赛不足）
FixtureStatus: Pending → Completed / Bye（scheduled.rs:196-204）
  ——⚠️ 无 skipped 变体；`skipped` 是 ScheduledFixture 的 bool 字段（scheduled.rs:184）
  （跳过 LIVE 但结果仍由月末确定性结算）
```

**LIVE 闸门**：开赛日当天主角队伍 pending 对阵可被 `live/today` 观测 →
玩家进入 LIVE（`POST /live`）逐回合决策 或 跳过（`skip`）→ 月末统一确定性结算。
