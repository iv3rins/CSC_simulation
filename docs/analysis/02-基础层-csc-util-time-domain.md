# 02 · 基础层：csc-util / csc-time / csc-domain

> 第 0 层三件套 + 文案包。零依赖或仅依赖 serde；被所有上层 crate 消费。

---

## 一、csc-util（工具：RNG / ID / 错误 / 数学 / 采样 / 签名）

### 1. Struct：`Xoshiro256StarStar`（csc-util/src/rng.rs）

确定性随机生成器（xoshiro256**），Kotlin `DeterministicRandom` 位级直译。
状态 = 4 × u64，**可序列化存档**（serde derive）。

| 方法 | 签名 | 说明 |
|---|---|---|
| `seed` | `pub fn seed(seed: u64) -> Self` | splitmix64 展开 4 字（Kotlin 同值常量） |
| `from_state` | `pub fn from_state(state: [u64; 4]) -> Self` | 从快照恢复（读档） |
| `snapshot` | `pub fn snapshot(&self) -> [u64; 4]` | 快照（存档） |
| `next_u64` | `pub fn next_u64(&mut self) -> u64` | 核心原语（= Kotlin `nextLong` 位模式） |
| `next_bits` | `pub fn next_bits(&mut self, bit_count: u32) -> i32` | 取高 n 位（n ∈ 1..=32） |
| `next_i32` | `pub fn next_i32(&mut self) -> i32` | 低 32 位截断（= Kotlin `nextInt()`） |
| `next_i32_bound` | `pub fn next_i32_bound(&mut self, until: i32) -> i32` | `[0, until)`；2 的幂分支 + 拒绝采样；**u32 同构修正负随机数 bug** |
| `next_i32_in` | `pub fn next_i32_in(&mut self, from: i32, until: i32) -> i32` | `[from, until)`；Kotlin **stdlib 双参数**实现（底层原语与单参数不同） |
| `next_double` | `pub fn next_double(&mut self) -> f64` | 53 位尾数 `[0,1)`（仅连续值抽样用） |
| `roll_bp` | `pub fn roll_bp(&mut self, p: f64) -> bool` | **整数化概率判定**：`next_u64() % 10000 < round(p×10000)`；恰耗 1 个 next_u64 |
| `next_bp` | `pub fn next_bp(&mut self) -> u64` | 掷基点骰 `[0,10000)`（掷骰与判定解耦，供纯函数阈值比较） |
| `next_float` | `pub fn next_float(&mut self) -> f32` | 24 位尾数 `[0,1)` |
| `next_bool` | `pub fn next_bool(&mut self) -> bool` | `next_bits(1) != 0` |

**Trait 实现**：`impl Default`——默认种子 = 42（rng.rs:203-208，与 Kotlin `Engine.DEFAULT_SEED` 一致）。

**确定性契约**：
- 布尔概率判定**一律走 `roll_bp`**（整数化，跨语言对齐）；`next_double` 仅用于连续值抽样；
- 单参数 `next_i32_bound` 与双参数 `next_i32_in` 底层原语**不同**，调用点必须与 Kotlin 一一对应；
- golden 测试锁死位级一致（seed 42/7/0 的 long/double/float/int 序列 + 快照恢复）。

### 2. Struct：`PlayerId` / `TeamId`（csc-util/src/id.rs）

`u32` 新类型稳定 ID；`NONE = u32::MAX` 哨兵（0 是合法 arena 索引）。
- 实现：`Debug/Clone/Copy/PartialEq/Eq/Hash/PartialOrd/Ord/Serialize/Deserialize/Display`；
- 引用键一律 ID（名字降级为展示字段）；ID 即 `World` arena 的 Vec 索引，由 World 分配（无独立分配器）。

### 3. Enum：`SimError`（csc-util/src/error.rs）

统一错误类型（外部输入边界：坏资产/坏存档/协议错误显式报错，不 panic、不静默）。
主要构造：`SimError::asset(file, reason)`（双参数）/ `SimError::save(msg)` / `SimError::protocol(msg)`，
`Display + std::error::Error`；辅助 `parse_json_checked`（资产语义校验）。

### 4. 自由函数（csc-util/src/sampling.rs / math_utils.rs 等）

| 函数 | 签名 | 说明 |
|---|---|---|
| `gaussian` | `pub fn gaussian(rng: &mut Xoshiro256StarStar) -> f64` | Box-Muller 标准正态（单实现，M5 收敛点；**在 sampling.rs:15**，math_utils.rs 无） |
| `sigmoid` | `pub fn sigmoid(x: f64) -> f64` | 1/(1+e^-x)（math_utils.rs:8） |
| `pick_index` | `pub fn pick_index(weights: &[f64], weight_sum: f64, rng: &mut Xoshiro256StarStar) -> usize` | 加权取点（**多一个预计算权重和参数**，sampling.rs:25） |
| `round_to_int` | `pub fn round_to_int(x: f64) -> i32` | 四舍五入（math_utils.rs:18） |
| `canonical` | `csc-util/src/canonical.rs` | 存档**键排序规范化** JSON（`canonical_json_bytes`）——**无 crc**；CRC32 在 server 层（`PersistedGame.state_crc32`，game/mod.rs:112），指纹在 csc-core/tests/fingerprint.rs（FNV-1a64） |
| `signatures` | `csc-util/src/signatures.rs` | 队伍签名（Kotlin 格式 `队名|选手名排序 join(",")` 的单一事实源） |
| `seed_chain::round_seed` | `pub fn round_seed(world_seed, match_id, map_id, round_number, decision_seq, sim_version) -> u64` | LIVE 逐回合 seed 派生（决策进入 seed 链） |
| `sampling` | `csc-util/src/sampling.rs` | 加权/均匀采样辅助 |

---

## 二、csc-time（模拟时钟）

### 1. Struct：`SimClock`（csc-time/src/clock.rs）

整数三字段 `(year, month, day)` 的日历线程，语义与 Java `LocalDate` 完全一致
（月末截断、UTC 天首）。**serde 可序列化**（存档形态）。

| 方法 | 签名 | 说明 |
|---|---|---|
| `of` | `pub fn of(year: i32, month: u32, day: u32) -> Self` | 指定日期创建 |
| `from_standings_month` | `pub fn from_standings_month(label: &str) -> Result<Self, String>` | 解析 `2026_05_04` 标签（坏标签显式报错） |
| `now` | `pub fn now(&self) -> (i32, u32, u32)` | 当前日期 |
| `now_epoch_seconds` | `pub fn now_epoch_seconds(&self) -> i64` | Unix 秒（UTC 天首） |
| `month_label` / `date_label` | `-> String` | `2026_05_04` / `2026-05-04` |
| `year` | `pub fn year(&self) -> i32` | 当前年 |
| `advance_days` | `pub fn advance_days(&mut self, days: i64)` | 推进 N 天 |
| `advance_months` | `pub fn advance_months(&mut self, months: i64)` | 日历月推进（floorDiv/floorMod + 月末截断） |
| `next_month_start` | `pub fn next_month_start(&mut self)` | 跳到下月 1 号 |
| `restore_to` | `pub fn restore_to(&mut self, year, month, day)` | 任意回退（读档） |
| `rolling_window` | `pub fn rolling_window(&self, months_back: i64) -> TimeWindow` | 滚动窗口（VRS 时间衰减用） |

### 2. Struct：`TimeWindow`（csc-time/src/window.rs）

闭区间 `[start_epoch_sec, end_epoch_sec]`；`new(start, end)`、`start()`、`end()`、`contains()`。

### 3. 自由函数（csc-time/src/civil.rs）

`days_from_civil(y,m,d) -> i64` / `civil_from_days(z) -> (y,m,d)` / `days_in_month(y,m) -> u32`
——Howard Hinnant 算法，Java LocalDate 同源。

`month_end_label("YYYY-MM-DD") -> "YYYY-MM-<月末>"`（2026-09 新增）
——取日期所在月的最后一天，解析失败回退原串。**唯一事实源**：服务端
`transfer_market_from_pending` 与历史 `csc-server/game/mod.rs::month_end`
曾各自实现一份，提取到本 crate 后共享。

---

## 三、csc-domain（跨系统值对象，零依赖）

### 1. 核心枚举

| Enum | 变体 | 用途 |
|---|---|---|
| `Tier`（tier.rs） | `Tier0..Tier4` + `TierRanges`/`TierTable`（tier.rs:35,205） | 队伍/选手实力档位（资产加载用）；**无 Ranked 结构**（`RankedTeam` 是 csc-tournaments/scheduler.rs:21 的 type alias `(i32,i32,TeamId)`） |
| `TourneyTier`（tourney_tier.rs） | `Major / SuperElite / Elite / T1 / T2 / Qualify` | 赛事等级（胜率放大系数、荣誉权重） |
| `EventImportance`（event_importance.rs） | `Background(默认) / Important / Major / Championship` | 事件重要度（LIVE 闸门） |
| `TeamTier`（team_tier.rs） | `T1/T2/T3/T4` + `from_ranking` + 3 常量 | 队伍档位映射（排程/邀请用） |
| `SeriesStage`（**csc-simulation/src/series.rs:20**） | `Group / Playoff / Quarterfinal / Semifinal / Final / Unknown(默认)` | 系列赛阶段（**不在 csc-domain**） |
| `Region`（city.rs） | `Europe / NorthAmerica / SouthAmerica / Asia / Oceania` | VRS 区域积分 |

### 2. 核心 Struct

| Struct | 字段（要点） | 说明 |
|---|---|---|
| `Tournament`（tournament.rs） | `name / tier / date / end_date / prize_pool / format / importance / teams`；`Organizer`(:14)/`InvitePolicy`(:32) | 赛事模板 |
| `TournamentFormat`（tournament_format.rs） | `bracket_type / teams / groups / bo1_groups / playoffs / best_of` | 赛制描述 |
| `City`（city.rs） | `name / country / region` | 城市（选手国籍/主场） |
| `AttrRange`（attr_range.rs） | `lo / hi`；`new()` 为 const fn + `assert!`——**仅 const 上下文编译期触发，运行期调用是运行期 panic**；`contains/len/mid` | 属性区间 |
| `MatchResult`（match_result.rs） | `winner/loser/score/maps/date/event`；`MatchVenue`(:14)/`venue_lan`(:60) | 单场结果（含稳定 ID） |
| `TierProfile`（tier_profile.rs） | 各档位属性分布参数 + `tier_best_of()` | 档位画像 |
| `VrsEntry`（vrs_entry.rs） | `rank/team_name/points/roster` | VRS 排名条目（真身在 csc-domain） |
| `RealTop20`（real_top20.rs） | 真实 2025 TOP20 选手先验 + `RealTop20Year`/`RealTop20Index` 查询 | 开局校准 |
| `SeasonGoal`（season_goal.rs） | 赛季目标枚举（`name/from_name`） | 玩家主动设定 |
| `Live*`（live.rs） | `LiveEconomy/UtilityPlan/WeaponClass/BombSite/Pace/Side/LiveMatchPhase` | LIVE 逐回合领域类型 |

### 3. 关键方法

- `TierProfile::tier_best_of(tier) -> i32`：各等级赛事 BO 数（BO1/BO3/BO5）；
- `AttrRange::new(lo, hi)`：const fn + assert（编译期/运行期双语义，见上）；
- 赛季目标判定在 `csc-core/settlement.rs:244-304`（仅 met/text，**无 `reward` 数值方法**）。

---

## 四、csc-text（文案包）

### Struct：`TextBundle`（csc-text/src/lib.rs）

叙事文案单一事实源：`assets/text/zh-CN.json`（**211 键**：text 208 + lists 3；`模块.场景.变体` 命名 + `{0}` 占位符）。

| 方法 | 签名 | 说明 |
|---|---|---|
| `get` | `pub fn get(&self, key: &str) -> &str` | 取原文（缺键回退） |
| `format` | `pub fn format(&self, key: &str, args: &[&str]) -> String` | 取词 + 占位符替换 |

- 编译期嵌入默认包（wasm32 兼容零 IO）；服务端/CLI 启动时可按键覆盖（改文案不改代码）。
