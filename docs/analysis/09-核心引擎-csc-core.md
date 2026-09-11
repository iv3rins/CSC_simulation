# 09 · 核心引擎：csc-core

> 第 6 层：总引擎（接线层）+ GameState 聚合根 + 查询层 + 客户端视图。
> **csc-core 只做三件事：装配 / 推进 / 存档**；查询层是表现层/服务端的只读边界。

## 2026-09-11 实施补录：已完成的有限边界

`WorldDecisionBatch` 已拆出真实 collect/submit 边界，原 `run` 复用两者。**T02/T03 仍未验收**：赛事/赛制/地图栈没有 continuation，月/日接口仍为同步接口。不得把 `advance_unit` 当作 step，或对故事玩家自动代答赛事。

| 分类 | 实际 API / 所属文件 | 契约 |
|---|---|---|
| Struct | `batch::PendingWorldBatch { date, points, transfer_window_due }` | 可序列化的收集产物，必须与收集后的真实世界/RNG 一起保存；未接 GameState，不是月度 continuation |
| Function | `WorldDecisionBatch::collect(ctx, rng, transfer_window_due) -> PendingWorldBatch` | 伤病/市场/人生/代言收集只执行一次，返回即退栈 |
| Function | `WorldDecisionBatch::submit(ctx, &pending, decisions, rng) -> Result<(), SimError>` | 校验日期/全集；world/VRS/journal/decision/RNG 候选事务，任何业务 Err 不交换；批次消费与幂等由上层管理 |
| Function | `Engine::set_narrative_content(content) -> Result<(), SimError>` | 验证内容；空进度采用新版本，已开始进度要求版本匹配；失败不改配置 |
| Function | `Engine::fork_snapshot() -> Result<Engine, SimError>` | 复用快照恢复，保留 text/rating_profile/narrative 配置；供服务端候选提交 |
| Function | `Engine::apply_story_choice(choice_id) -> Result<AppliedOutcome, SimError>` | 调用共享 `ChoiceContext` 效果事务，传播失败 |

演员只把 `teammate` 绑定真实队友；当前无可靠 captain/coach 实体标识，二者保留概括身份。`narrative_facts` 不再用当前队伍反推个人历史：旧比赛事件没有参赛 PlayerId，当前返回 0/0 作为已证明场次下界，**不是已验证没有历史比赛**。赛后弧暂不开放，需 T07 在正式结算补稳定个人参赛事实。

验证：`cargo test -p csc-core -p csc-decision --lib --locked --offline` exit 0，87 + 18 项通过，见 [日志](implementation-2026-09-11-core-lib.log)。新增测试涵盖 collect/serde/submit 与同步结果一致、后续业务失败无早期训练/日志/RNG 提交、拒绝后可重交、角色绑定、内容不匹配、候选配置和旧执行档拒绝。完整赛事停点/重启/多月等价仍属于未完成验收。

事务暂复制 touched 聚合（含已有日志），没有复制赛事/档案；这是可理解的正确性边界，但长生涯高频提交的成本仍需以后优化，不声称性能任务已经完成。

---

## 一、Struct：`Engine`（src/engine.rs）

总引擎（封装边界：全部字段私有，可变推进必须经门面方法）。

### 装配

| 方法 | 签名 | 说明 |
|---|---|---|
| `empty` | `pub fn empty(seed: u64, clock: SimClock) -> Self` | 空世界装配（测试） |
| `load_from_standings` | `pub fn load_from_standings(files: &[(String, String)], calibration: &CalibrationAssets<'_>, seed: u64, clock: SimClock) -> Result<Self, SimError>` | 从真实 standings 装配（128 队开局；**零 IO——文件内容由调用方读入**，engine.rs:773-778） |
| `plan_opening_month` | `pub fn plan_opening_month(&mut self, month_idx: i32)` | 物化开局月赛事（LIVE 闸门） |

### 推进（阶段管线）

| 方法 | 签名 | 说明 |
|---|---|---|
| `advance_month` | `pub fn advance_month(&mut self, decision: &mut dyn DecisionSource) -> Result<(), SimError>` | 推进一个月（先结算当前月 → plan → 结算新月 → finish） |
| `settle_month` | `pub fn settle_month(&mut self, decision) -> Result<(), SimError>` | 结算当前月已登记赛事 |
| `advance_month_plan` | `pub fn advance_month_plan(&mut self, decision) -> Result<(), SimError>` | 进入新月 + 登记 Future（阶段 1） |
| `advance_month_finish` | `pub fn advance_month_finish(&mut self, decision, restore_to_end: bool) -> Result<(), SimError>` | 月终收束（阶段 3） |
| `run_season` | `pub fn run_season(&mut self, months: i32, decision) -> Result<(), SimError>` | 连续推进 N 月 |
| `advance_day` | `pub fn advance_day(&mut self, decision)` | 日级推进（LIVE 逐日经过比赛日；**无 days 参数**，engine.rs:822-825） |
| `run_days` | `pub fn run_days(&mut self, days: i32, decision)` | 按天循环推进（engine.rs:865） |

> ⚠️ `Engine::advance_season` **不存在**——FIFA 式赛季推进在服务端 `GameEntry::advance_season_blocking`（csc-server/game/mod.rs:563）。

### 存档 / 读档

| 方法 | 签名 | 说明 |
|---|---|---|
| `snapshot` | `pub fn snapshot(&self) -> GameState` | 完整快照（含 rng_state/decisions/journal/archive/world/vrs/events/yearly_rating/top20_history/scheduled_records/sim_version） |
| `restore` | `pub fn restore(&mut self, state: GameState)` | 完整读档（顺序契约：先还原 arena 再反解锁） |
| `restore_with_profile` | `pub fn restore_with_profile(&mut self, state, rating_profile)` | 读档 + 恢复新秀实力分布配置 |

### 只读访问器（封装边界）

| 方法 | 说明 |
|---|---|
| `world() / vrs() / tournaments() / clock() / journal() / archive() / decision_log()` | 各子系统只读视图 |
| `query()` | `-> WorldQuery<'_>`（查询层） |
| `client_state()` | `-> ClientState`（轻量视图，20 年 ≈1MB） |
| `player()` | `-> Option<PlayerId>`（主角） |
| `month()` / `rng_snapshot()` | 状态查询 |

### 业务门面

| 方法 | 签名 | 说明 |
|---|---|---|
| `transfer_offers` | `pub fn transfer_offers(&self, player_id) -> Result<Vec<TransferOffer>, SimError>` | 转会候选（合同期内报错） |
| `sign_transfer` | `pub fn sign_transfer(&mut self, player_id, team_signature) -> Result<TransferEvent, SimError>` | 主动签约 |
| `make_statement` | `pub fn make_statement(&mut self, player_id, tone) -> Result<(), SimError>` | 主动发言（CONFIDENT/HUMBLE/PROVOKE） |
| `set_match_plan` | `pub fn set_match_plan(&mut self, player_id, style)` | 赛前 BP / 战术预案 |
| `buyout_player` | `pub fn buyout_player(&mut self, player_id) -> Result<i64, SimError>` | 买断自己合同 |
| `invest_self` / `buy_skin` | 资金运用门面 | 投资自己 / 购买饰品 |
| `player_contract` | `pub fn player_contract(&self, player_id) -> Result<String, SimError>` | 合同摘要 |
| `plan_training` | `pub fn plan_training(&mut self, player_id, focus)` | 排定训练计划 |
| `year_end_awards` | `pub fn year_end_awards(&mut self, year, top) -> Vec<PlayerId>` | 年度颁奖（TOP N） |
| `seed_entities_from_standings` | `pub fn seed_entities_from_standings(&mut self, standings, baseline, ratings)` | 从 standings 播种实体 |
| `create_protagonist` | `pub fn create_protagonist(&mut self, name: &str, tier: Tier, team_id: TeamId, rng: &mut Xoshiro256StarStar, role: Option<Role>) -> Result<PlayerId, SimError>` | 创建主角（**含 team_id，参数顺序不同**，engine.rs:897-904） |
| `auto_decision` | `pub fn auto_decision() -> AutoDecisionSource` | 自动决策源工厂 |
| `skip_live_today` | `pub fn skip_live_today(&mut self) -> usize` | 标记今日 LIVE 跳过 |
| `mark_fixture_watched` | `pub fn mark_fixture_watched(&mut self, event_name, fixture_id) -> bool` | 标记已看完 |
| `rng_mut` / `journal_mut` / `tournaments_mut` / `world_mut` / `vrs_mut` | 外部操作专用可变入口（服务端资金运用等） | — |

### Struct：`CalibrationAssets<'a>`

`{ baseline: Option<&str>, ratings: Option<&str>, rating_profile: Option<&str>, text: Option<&str> }`
——可选校准资产（JSON 内容，零 IO 由调用方读入）。

---

## 二、Struct：`GameState`（src/state.rs）

**完整存档聚合根**（无对象引用、无副作用、serde 派生、`deny_unknown_fields`）。

### 字段（18 项，state.rs:57-98）

| 字段 | 类型 | 说明 |
|---|---|---|
| `version` | `u32` | 存档格式版本（当前 **v11**；源码权威 `GameState::CURRENT_VERSION`） |
| `month` | `i32` | 赛季月份计数 |
| `last_contract_year` | `i32` | 合同结转锚点 |
| `locks` | `Vec<LockRecord>` | 赛事队伍锁 |
| `sim_year/sim_month/sim_day` | `i32/u32/u32` | 时钟 |
| `rng_state` | `[u64; 4]` | RNG 状态 |
| `decisions` | `Vec<PlayerDecision>` | 决策日志 |
| `journal` | `Vec<WorldEvent>` | 事件流 |
| `archive` | `HashMap<PlayerId, Vec<SeasonRecord>>` | 生涯档案 |
| `world` | `World` | 实体 arena |
| `vrs` | `VrsDatabase` | 排名库 |
| `events` | `Vec<TournamentResult>` | 赛事结果 |
| `yearly_rating` | `YearlyRatingTracker` | 年度累计器 |
| `top20_history` | `Vec<Top20YearBoard>` | 历届 TOP20 |
| `scheduled_records` | `Vec<ScheduledTournamentRecord>` | 排程记录 |
| `sim_version` | `u32` | 世界级模拟器版本标签（指纹链） |
| `narrative` | `NarrativeProgress` | v10 增进度；v11 增分支/关闭弧/结构化选择历史 |
| `execution` | `ExecutionState` | v10 草稿形状；当前仅 Idle 可以恢复，旧 Awaiting 明确拒绝 |

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `from_json_str` | `pub fn from_json_str(json: &str) -> Result<Self, SimError>` | 解析（拒绝未知版本/未知字段） |
| `migrate` | `pub fn migrate(json: &str) -> Result<Self, SimError>` | 旧版迁移 |
| `migrate_state` | `pub fn migrate_state(self) -> Self` | 就地迁移（v<5 荣誉量纲 ×0.1 归一） |
| `CURRENT_VERSION` | `u32 = 11` | v11 叙事路径与历史字段；不代表完整可暂停世界已完成 |
| `validate_execution` | `pub fn validate_execution(&self) -> Result<(), SimError>` | 拒绝旧版缺少 continuation 的 Awaiting 草稿 |

### Struct：`LockRecord`

`{ event_name, end_day, start_epoch_day, end_epoch_day, team_ids: Vec<TeamId> }`
——赛事队伍锁（绝对日序表达，跨月语义正确）。

### 版本演进（v1–v11）

| 版本 | 内容 |
|---|---|
| v3 | `pending_training`（主动训练） |
| v4 | `transfer_contacts`（队伍招募情报） |
| v5 | TOP20 荣誉量纲校准（MVP=1.0/EVP≤0.5，旧档 ×0.1） |
| v6 | `season_goal / season_goal_year` |
| v7 | `PlayerFinance.skins / last_invest_year`（资金运用） |
| v8 | `CareerInfo.match_style / public_stance`（主动操作） |
| v9 | `scheduled_records`（future 赛程权威记录）+ `sim_version`（世界级模拟器指纹版本标签） |
| v10 | `narrative` 和 `execution` 草稿；不得把 Awaiting 字段视为实现了持久暂停 |
| v11 | `next_scenes` / `closed_arcs` / `choice_history`；旧空闲档补空，旧执行草稿拒绝 |
| sim_version | 指纹版本标签（serde default 补 1） |

---

## 三、Struct：`SeasonDirector`（src/season.rs）

推进状态 + 阶段管线。

### 常量

| 常量 | 值 | 说明 |
|---|---|---|
| `SEED_MONTHS` | `6` | VRS Seed 窗口回看月数 |
| `TRANSFER_WINDOW_MONTHS` | `4` | 转会窗间隔（月） |
| `MONTHS_PER_YEAR` | `12` | 每年月数 |

### 方法

`new(clock_year, rng)` / `restore(&GameState)` / `rng_snapshot()`。

### 阶段管线（advance_month 8 步）

```
1. 时钟进新月 + 清上月锁（SeasonLoop::clear_locks）
2. 应用玩家主动排定的训练计划（apply_pending_training）
3. 【跨年时】YearlySettlement::settle（颁奖/归档/成长/人口/薪资/合同）
4. SeasonLoop::plan_month（排程登记 + 队伍级互斥锁）
5. SeasonLoop::settle_month（赛事结算）
6. VRS reseed（滚动窗口）+ sync_vrs_cache（排名缓存回写）
7. ChemistryEngine::monthly_recovery（关系收敛）
8. WorldDecisionBatch::run（伤病tick + 转会/代言/人生决策批次）
```

### 自由函数

- `sync_vrs_cache(world, vrs)`：VRS 实时排名回写 Team 缓存（2026 修复：排名不再静止）；
- `advance_month_plan / advance_month_finish / advance_month / run_season`：管线各阶段。

---

## 四、Struct：`WorldQuery<'a>`（src/query.rs）

查询层（**只读边界**——表现层/服务端协议唯一数据源）。

### 字段

`{ world, vrs, tournaments, journal, archive, decision_log, clock, text }`（全部只读引用）。

### 方法（14 个，query.rs:98-235）

| 方法 | 签名 | 说明 |
|---|---|---|
| `world_summary` | `-> WorldSummary` | 世界概览 |
| `rankings` | `-> Vec<VrsEntry>` | VRS 排名 |
| `player_profile` | `(&self, name) -> Option<PlayerProfile>` | 玩家档案 |
| `career` | `(&self, player_id) -> Option<&CareerInfo>` | 生涯信息 |
| `career_totals` | `-> Option<SeasonTotals>` | 生涯合计 |
| `career_ending` | `-> Option<(PlayerId, String, CareerEnding)>` | 生涯结局（唯一实现） |
| `career_of` | `-> Option<...>` | 选手档案查询 |
| `journal_all` | `-> Vec<WorldEvent>` | 全量事件 |
| `journal_since` | `(seq) -> Vec<WorldEvent>` | 增量事件 |
| `journal_of` / `journal_of_player` | 按名/按 ID 查事件 | — |
| `event_results` | `-> Vec<TournamentResult>` | 赛事结果 |
| `decision_history` | `-> Vec<PlayerDecision>` | 决策历史 |
| `top20` | `-> Top20YearBoard` | 当前年度 TOP20 |

> ⚠️ 旧文档误列的 `top20_of` / `top20_history` / `calendar_results_for_year` /
> `season_calendar_plan` / `world_stories` **不属 WorldQuery**：
> `top20_of` 是自由函数（query.rs:269）；`calendar_results_for_year`/`season_calendar_plan`
> 在 client.rs:443/420；`world_stories` 是自由函数（world_stories.rs:59）。

### Struct（查询值对象）

- `WorldSummary { date, month, team_count, npc_count, player_count, event_count, journal_cursor, decisions_logged }`；
- `PlayerProfile { name, age, role, team, power, fatigue, injury, reputation, salary, contract_years, cash, career_earnings, sponsors, marks, marks_summary, career_wins, career_kills, career_deaths }`；
- `Top20Entry { rank, player_id, player_name, team_name, score, weighted_rating, ... }`。

---

## 五、客户端轻量视图（src/client.rs）

**性能契约**：GameState 是完整存档聚合根（20 年 ≈68MB），网页端绝不按月轮询它。

### Struct：`ClientState`

`{ next_milestone, view_version, month, date, sim_year, ... }`——轻量视图
（主角 + roster + 主角赛事 + 年度标量；15 年 51.9MB → 1.0MB，约 2%）。

### Struct（投影值对象）

| 类型 | 说明 |
|---|---|
| `ClientWorld` | 主角 + roster + 全队名次引用 |
| `ClientSeason` | 加权 Rating / 图数 / 击杀 / 荣誉 |
| `ClientTournamentResult` | 主角赛事（含 bracket 槽位 + replayable 标记） |
| `ClientBracketSlot` | 阶段/BO/live/入场仪式 |
| `MajorPresentation` | Major 入场仪式数据 |
| `CareerMilestone` | 生涯下一重要节点（NEXT/LIVE 倒计时） |
| `PlayerCalendarEvent` | 主角年度赛历条目（future/active/completed/cancelled） |
| `ClientFixture` / `ClientFixtureScore` | 对阵 + 比分 |
| `EventAggregate` | 赛事对象化聚合（参赛队 + fixtures + 冠军 + 主角参与度，R2） |
| `EventTeam` / `EventChampion` | 聚合子对象 |
| `LiveGateFixture` / `LivePlayerSlot` | 今日 LIVE 闸门投影 |

### 关键函数

- `client_state_from(...)`：投影装配（唯一入口）；
- `player_calendar_events(state, year, plan)`：主角赛历；
- `event_aggregate(state, event_name)`：赛事聚合；
- `scheduled_status_label(status)`：状态文案。

---

## 六、其它模块

| 模块 | 说明 |
|---|---|
| `batch.rs` | `WorldDecisionBatch::run`（月度世界决策批次） |
| `loop_.rs` | `SeasonLoop`（赛事锁管理 + 月排程/结算编排）+ `TournamentLock` |
| `settlement.rs` | `YearlySettlement::settle`（跨年结算：颁奖/归档/成长/人口/薪资/合同；G2 赛季目标 Option 化） |
| `nan.rs` | `assert_no_nan`（debug 钩子：advance/settle 后全状态 NaN 扫描） |
| `insights.rs` | `MatchInsight` / `TeamStatusInsight` / `match_insight` / `team_status_insight`（结果解释层） |
| `world_stories.rs` | `WorldStories` / `TeamStory` / `Rivalry` / `world_stories`（世界叙事：王朝/重建/宿敌） |
| `context.rs` | `WorldContext`（推进期的全部可变引用聚合） |
| `state.rs` | 见上 |
| `client.rs` | 见上 |
