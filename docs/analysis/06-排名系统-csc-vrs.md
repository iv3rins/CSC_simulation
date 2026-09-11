# 06 · 排名系统：csc-vrs

> VRS（Valve Regional Standings）官方两层模型：**静态 Seed 分（第 1 层）+ 动态 ELO（第 2 层）**。
> 按队伍签名（signature）键控——阵容变化影响签名 → 影响 ELO 归属。

---

## 一、Struct：`VrsDatabase`（src/database.rs）

排名数据库（可序列化，随存档保存）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `months` | `Vec<String>` | 按时间排序的月份标签（如 `2026_01_05`） |
| `by_month` | `HashMap<String, Vec<VrsEntry>>` | month → 当月全部排名条目（初始基线，只读） |
| `history` | `HashMap<String, HashMap<String, i32>>` | signature → (month → ranking) 跨月历史 |
| `teams` | `HashMap<String, VrsTeamState>` | signature → 当前模拟状态（可变） |
| `match_history` | `Vec<MatchRecord>` | 比赛历史（Seed 重算用；reseed 时按窗口剪枝） |

### 方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `from_json_files` | `pub fn from_json_files(files: &[(String, String)]) -> Result<Self, SimError>` | 从 standings JSON 装配（文件 = (文件名, 内容) 对） |
| `parse_json_checked` | `pub fn parse_json_checked(json: &str, file_name: &str) -> Result<Vec<VrsEntry>, SimError>` | 语义校验解析（空队名/负积分/空阵容显式报错） |
| `parse_json` | `pub fn parse_json(json: &str) -> Vec<VrsEntry>` | 宽松解析 |
| `month_of` | `pub fn month_of(file_name: &str) -> String` | 文件名 → 月份标签 |
| `team_of` | `pub fn team_of(&self, signature: &str) -> Option<&VrsTeamState>` | 按签名查队伍状态 |
| `all_teams` | `pub fn all_teams(&self) -> Vec<&VrsTeamState>` | 全部队伍 |
| `signature_of` | `pub fn signature_of(team_name: &str, roster: &[String]) -> String` | 签名生成（静态） |
| `entries_of` | `pub fn entries_of(&self, month: &str) -> Vec<VrsEntry>` | 某月排名条目 |
| `ranking_of` | `pub fn ranking_of(&self, entry: &VrsEntry, month: &str) -> Option<i32>` | 某月排名 |
| `consecutive_top_n` | `pub fn consecutive_top_n(...)` | 连续 N 月 TOP 判定 |
| `seed_points_of` | `pub fn seed_points_of(&self, signature) -> Option<i32>` | 查询 Seed 分 |
| `match_history_len` | `pub fn match_history_len(&self) -> usize` | 比赛历史条数 |
| `upsert_state` | `pub fn upsert_state(&mut self, signature: String, state: VrsTeamState)` | 插入/更新队伍状态 |
| `apply_match` | `pub fn apply_match(&mut self, ...)` | 记录比赛（ELO + 历史） |
| `apply_match_result` | `pub fn apply_match_result(...)` | 按状态对记录 |
| `reseed` | `pub fn reseed(&mut self, window: &TimeWindow)` | 按窗口重算 Seed 分 |
| `apply_roster_change` | `pub fn apply_roster_change(&mut self, signature: &str, new_roster: &[String]) -> bool` | 阵容变更 |
| `settle_rosters` | `pub fn settle_rosters(&mut self)` | 结算全部阵容 |
| `departed_count` | `pub fn departed_count(&self) -> usize` | 离队人数（3 人判定） |

### Struct：`VrsTeamState`

`{ team_name, points(ELO), ranking, roster, last_settled_roster, seed_points }`。

### Struct：`MatchRecord`（modifiers.rs:12-23）

`{ winner_sig: String, loser_sig: String, timestamp: i64, prize_pool: i64, lan: bool }`——比赛历史记录。

---

## 二、Struct：`VrsEngine`（src/engine.rs）

排名引擎门面（状态持有 = VrsDatabase）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `from_json_files` | `pub fn from_json_files(files: &[(String, String)]) -> Result<Self, csc_util::SimError>` | 从 standings 装配 |
| `from_database` | `pub fn from_database(database: VrsDatabase) -> Self` | 装配 |
| `database` | `pub fn database(&self) -> &VrsDatabase` | 只读视图（**无 database_mut**） |
| `team_of` | `pub fn team_of(&self, signature: &str) -> Option<&VrsTeamState>` | 查队伍状态 |
| `all_teams` | `pub fn all_teams(&self) -> Vec<&VrsTeamState>` | 全部队伍状态 |
| `ranking_entries` | `pub fn ranking_entries(&self) -> Vec<VrsEntry>` | 当前排名 |
| `apply_match` | `pub fn apply_match(&mut self, record: &MatchRecord)` | 比赛结算（ELO 更新 + 记录历史） |
| `apply_match_result` | `pub fn apply_match_result(&mut self, winner: &VrsTeamState, loser: &VrsTeamState, venue)` | 按状态对结算 |
| `apply_roster_change` | `pub fn apply_roster_change(&mut self, signature: &str, new_roster: &[String]) -> bool` | 阵容变更（3 人离队惩罚） |
| `apply_roster_change_verified` | `pub fn apply_roster_change_verified(&mut self, old_sig: &str, new_roster: &[String])` | 校验后的阵容变更 |
| `settle_roster_window` | `pub fn settle_roster_window(&mut self)` | 窗口内阵容结算 |
| `reseed` | `pub fn reseed(&mut self, window: &TimeWindow)` | 滚动窗口重算 Seed 分 |
| `expected_win_rate` | `pub fn expected_win_rate(&self, winner_points: i32, loser_points: i32) -> f64` | 期望胜率（ELO 公式） |
| `tier_venue` / `tier_prize_pool` | 赛事等级 → 场地/奖金池 | 赛事配置 |
| `t1_team_count` / `t1_open_qualifier_slots` | T1 队伍数 / 公开预选名额 | 排程参数 |
| `team_tier_of_ranking` / `team_tier_of` / `tier_map_of` | 排名/条目 → TeamTier | 队伍档位映射 |
| `t1_invitees` / `t2_eligible` / `t3_eligible` / `t4_eligible` / `major_field` | 各档位参赛名单 | 邀请模型数据源 |
| `signature_of` | `pub fn signature_of(team_name: &str, roster: &[String]) -> String` | 签名生成（静态） |
| `signature_of_entry` | `pub fn signature_of_entry(entry: &VrsEntry) -> String` | 条目签名 |
| `to_entry` | `pub fn to_entry(t: &VrsTeamState) -> VrsEntry` | 状态 → 条目 |

**注意**：csc-server 每局一个引擎实例（不是全局共享）；存档含完整 VrsDatabase。

---

## 三、Struct：`VrsEntry`（src/entry.rs）

`{ rank: i32, team_name: String, points: i32, roster: Vec<String> }`——排名条目（跨月基线）。

---

## 四、Seed 计算（src/modifiers.rs）与 ELO 结算（src/scoring.rs）

**官方两层模型参数**——注意：modifiers.rs **没有** `SEED_WINDOW_MONTHS/ELO_K/tier_weight/stage_weight/region_bonus/roster_change_penalty/decay` 等常量（旧文档虚构，已修正）：

### modifiers.rs 实际常量

| 常量 | 值 | 说明 |
|---|---|---|
| `BUCKET_SIZE` | `10` | Seed 分桶大小 |
| `OUTLIER_COUNT` | `5` | 离群值数量 |
| `MIN_SEEDED_RANK` / `MAX_SEEDED_RANK` | `400` / `2000` | Seed 排名钳制 |
| `MAX_PRIZE_POOL` | `1_000_000` | 奖金池上限 |
| `SEED_FACTOR_WEIGHTS` | — | Seed 因子权重 |

### modifiers.rs 实际方法

`curve_function(x)` / `power_function(x)` / `remap_value_clamped(...)` /
`nth_highest(values, n)` / `compute_seed_values(...)` / `seed_to_elo(raw_seeds)`。

### scoring.rs（ELO 动态层，Glicko 退化）

| 常量/方法 | 值/签名 | 说明 |
|---|---|---|
| `q()` | `ln(10)/400` | ELO 系数 |
| `FIXED_RD` | `75` | 固定评级偏差 |
| `expected(winner_p, loser_p)` / `expected_win_rate(winner_points, loser_points)` | 期望胜率 | — |
| `win_delta(winner_p, loser_p, info)` / `lose_delta(...)` | Δ 分 | **随预期胜率自适应，无 tier/stage K 权重** |
| `apply_match_with_info(winner, loser, info)` | — | 按 info 结算 |

> `SEED_MONTHS=6`（滚动窗口回看月数）在 `csc-core/src/season.rs:43`（SeasonDirector），不在 csc-vrs。
> `ROSTER_CLEAR_DEPARTURES=3`（3 人离队清零判定）在 database.rs:504。

---

## 五、工作方式（Seed + ELO 两层）

```
初始：standings JSON → VrsDatabase（真实排名基线 + 历史）
运行中：
  1. 每场比赛结束 → VrsEngine::apply_match / apply_match_result（→ scoring.rs）：
     - ELO 层：Glicko 退化模型，Δ 随预期胜率自适应（无 tier/stage K 权重）
     - 记录 MatchRecord（供 Seed 重算）
  2. 每月末（advance_month_finish）→ VrsEngine::reseed(rolling_window(6))：
     - 按 6 个月窗口内比赛经 compute_seed_values / seed_to_elo 重算静态 Seed 分
     - 排名 = Seed + ELO 综合排序
  3. 同时 sync_vrs_cache：VRS 实时排名回写 Team.vrs_ranking/vrs_value 展示缓存
     （2026 修复：排名不再静止）
```

---

## 六、与 TOP20 的关系

- VRS 管**队伍排名**；TOP20 管**选手年度榜单**（见 08 篇 `Top20Evaluator`）；
- TOP20 权重：Rating 0.60 / 荣誉 0.15 / 季后赛 0.25（top20.rs:7-9 头部注释与常量一致，C2 已闭环）。
