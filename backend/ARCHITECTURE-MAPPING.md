# Rust + WASM + TypeScript 转写 · 架构映射方案

> 蓝本：`design/kotlin_prototype/csc_prototype`（18 包 / 86 文件 / ~10.8K 行 / R5=0 环）
> 原则：**不逐行翻译**，按 Rust 惯用范式重设计；业务语义 100% 一致；类型系统强制架构。
> 本文件是转写的总蓝本，模块级细节在转写时按 M0-M11 顺序逐模块产出。

**已确认决策（2026-08）：** 前端 = **Solid**；可复现性口径 = **统计一致**（概率整数化 + 语言内位级一致）；
crate 粒度 = **系统包合一**（transfer/population/injury/chemistry/economy 并入 `csc-systems`，workspace ~14 crate）。

**2026-08-13 更新（P0/P1 补全后状态）：**
- ✅ **ID 全覆盖已落地**：`WorldEvent` 全部变体 / `DecisionPoint` 全部变体 / `CareerArchive` /
  `LockRecord`/`TournamentLock` 均携带 `PlayerId`/`TeamId`（名字降级为展示字段，`SeasonRecord`
  自含 `player_name`）——B3 在 Rust 侧完成；
- ✅ **存档版本化已落地**：`GameState.version`（当前 v1）+ `deny_unknown_fields` +
  `from_json_str` 版本校验 + `migrate` 迁移挂载点；
- ✅ **概率整数化已落地（D2）**：`Xoshiro256StarStar::roll_bp`/`next_bp`（基点 1/10000）
  替换全部布尔概率分支；连续值抽样（Box-Muller/加权取点/属性生成）保留 `next_double`；
- ✅ **SimError 已落地（D6）**：`csc_util::SimError`（Asset/Save/Protocol）+ assets 语义校验
  （`VrsDatabase::parse_json_checked`：空队名/负积分/空阵容/非正排名显式报错；
  同名不同队合法）+ `Engine::load_from_standings` 返回 `Result`；
- ⚠️ 查询层已内建于 `csc-core/src/query.rs`（本文件下方 crate 图中"csc-query 未建"已过时）。

---

## 1. 目标与非目标

**目标**
- 核心模拟逻辑：Rust（纯逻辑库，无 IO，可同时编译 native 与 wasm32）
- 服务端：Rust（REST + WebSocket，JSON 协议）
- 前端：TypeScript（Solid/React，纯表现层）
- 跨语言可复现性：同种子 + 同决策序列 → **统计一致**的世界（语言内位级一致）
- 编译期锁死 Kotlin 时代靠纪律维持的依赖方向（cargo 禁止 crate 循环依赖）

**非目标（明确不做）**
- 不逐行翻译；不保留 Kotlin 的对象引用图（改用 ID + arena）
- 不做位级跨语言一致性（浮点 libm 差异不可避免，见 D2）
- 不复制 Kotlin 的"部分存档"妥协——Rust 侧一步到位完整存档（对应 docs/09 阶段 2）

## 2. 三端拓扑

```
┌────────────────────┐   REST(查询/存档) + WS(决策点推送/事件流/指令)   ┌─────────────────────┐
│  TS 前端 (Solid)   │ ◄────────────────────────────────────────────► │ Rust 服务端 (axum)   │
│  · 决策面板         │                                                │ · 每局一个线程(隔离)  │
│  · 赛事回放渲染      │                                                │ · HumanDecisionSource │
│  · 生涯/档案视图     │                                                │   经 channel 等待前端 │
└─────────┬──────────┘                                                └──────────┬──────────┘
          │ (可选) WASM 单机模式：同一核心编译到浏览器                              │
          ▼                                                                      ▼
┌────────────────────────────────────────────────────────────────────────────────────┐
│  csc-core 及以下：纯同步逻辑库（无 tokio/无 IO/无 std 依赖除外）                        │
│  domain → entities → simulation → systems → core → query（依赖方向由 cargo 强制）      │
└────────────────────────────────────────────────────────────────────────────────────┘
```

## 3. Kotlin 包 → Rust crate 映射

| Kotlin 包 | Rust crate | 说明 |
|---|---|---|
| `domain/` (8) | **`csc-domain`** | 值对象；`#[derive(Serialize, Deserialize)]`；零依赖 |
| `util/` (4) | **`csc-util`** | DeterministicRandom→`xoshiro256**` 实现（与 Kotlin 位级一致）+ ID |
| `time/` (2) | **`csc-time`** | SimClock/TimeWindow（**独立 crate**：Kotlin 中 `vrs→time`，不能并入 core） |
| `entities/` (19) | **`csc-entities`** | **arena + ID 引用**（替代对象图）；属性领域 |
| `simulation/` (13) | **`csc-simulation`** | 纯函数规则系统 |
| `vrs/` (5) | **`csc-vrs`** | 排名数据库；matchHistory **窗口剪枝**（修 B2） |
| `tournaments/` (9+6) | **`csc-tournaments`** | 赛事引擎 + format 赛制 |
| `transfer/` `population/` `injury/` `chemistry/` `economy/` | **`csc-systems`** | 5 个小系统合一 crate（内部分 mod）；减少 workspace 膨胀 |
| `decision/` (5) | **`csc-decision`** | 决策点枚举 + 决策日志（**独立**：`tournaments→decision`） |
| `events/` (1) | **`csc-events`** | WorldJournal（**独立**：`tournaments→events`） |
| `career/` (2) | **`csc-career`** | CareerArchive（**独立**：`tournaments→career`） |
| `query/` (1) | **`csc-query`** | 只读聚合（表现层 DTO 源）；**已内建于 `csc-core/src/query.rs`**（功能齐：summary/rankings/profile/career/journal_since/decision_history；独立 crate 未拆，避免为单一文件建 crate） |
| `core/` (Engine/GameState/season) | **`csc-core`** | 装配 + 推进编排 + GameState 聚合根 |
| — | **`csc-server`** | axum REST/WS；每局一线程 + channel 决策源 |
| — | **`csc-app`** | CLI 交互（阶段 3 A1） |
| — | **`csc-wasm`** | wasm32 绑定层（薄 API + 事件缓冲） |
| — | **`csc-verify`** | 回放校验器（种子+日志→重算→指纹比对，修 C2） |

**cargo workspace 依赖方向（编译期强制无环；2026-08 已按实际实现修正）：**
```
第 0 层（零依赖基础）
  csc-domain（值对象，含 VrsEntry）   csc-util（RNG/ID）   csc-time（时钟）
        ↑                                   ↑                  ↑
        └────────────────┬──────────────────┴──────────────────┘
第 1 层   csc-entities（domain+util）        csc-events（domain+entities）
        ↑                                   ↑
        └────────────────┬──────────────────┘
第 2 层   csc-simulation   csc-vrs（domain+time+util）   csc-career（entities+util）
        ↑                 ↑                               ↑
        └────────────────┬┴──────────────────────────────┘
第 3 层   csc-decision（entities+events+simulation+util）
                        ↑
第 4 层   csc-systems（五引擎合一：transfer/population/injury/chemistry/economy）
                        ↑
第 5 层   csc-tournaments（赛事系统，依赖 systems 的 chemistry/economy）
                        ↑
第 6 层   csc-core（总引擎 + GameState 聚合根）  ← csc-query（内建于 csc-core/src/query.rs）
                        ↑
      csc-verify（M8） csc-server（M9） csc-app（M10） csc-wasm（M10）——均已于 2026-08-13 建库
```
> 与蓝本初稿的两处修正：① `VrsEntry` 值对象下沉 `csc-domain`——`csc-entities`/`csc-simulation`
> 不再反向依赖排名引擎 `csc-vrs`（值对象下沉、引擎上浮）；② 赛事系统依赖子系统
> （`csc-tournaments → csc-systems`）而非初稿的反向——赛事结算/化学反应需消费子系统引擎。
> Kotlin 里靠审查纪律维持的依赖方向，Rust 里由 cargo 编译期锁死——这是转写最大的架构红利。

## 4. 类型系统映射表

| Kotlin | Rust |
|---|---|
| `data class` | `struct` + `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]` |
| `sealed interface DecisionPoint`（6 子类） | `enum DecisionPoint { TransferWindow {...}, TrainingFocus {...}, ... }` |
| `enum class` | `enum`（可带 `#[repr]` 或新类型） |
| `fun interface DecisionSource` | `trait DecisionSource { fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> }` |
| `object AutoDecisionSource` | `struct AutoSource; impl DecisionSource for AutoSource`（零状态） |
| `object FreeAgentTeam.instance` | **消除**：`TeamId::FREE_AGENT` 常量 + 全局 arena 登记（修 C1） |
| `Random` 接口 + `DeterministicRandom` | `trait DeterministicRng` + `Xoshiro256StarStar`（**同算法**，Kotlin/Rust 可互验证） |
| 可空 `T?` | `Option<T>` |
| `MutableList` / `MutableMap` | `Vec<T>` / `HashMap<K, V>`（推进期用 `&mut` 独占，无需 RefCell） |
| `Team.roster: List<Player>`（引用共享） | `Team.roster_ids: Vec<PlayerId>` + `World.players: Vec<Player>` arena |
| `require/check`（内部不变量） | `assert!`/`debug_assert!`（panic = 编程错误） |
| 外部输入校验 | `Result<T, SimError>`（bad save / 非法决策 → Err，不 panic） |
| `snapshot()/restore()` | `GameState` 全量 serde；**含实体/VRS/赛事（无妥协）** |

### 4.1 实体 ID 设计（修 B3，转写前置）

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamId(pub u32);

pub struct World {
    pub players: Vec<Player>,          // arena：id 即索引（删除用 tombstone 或 swap_remove+索引表）
    pub teams: Vec<Team>,
    pub free_agents: Vec<PlayerId>,
    pub next_player_id: u32,
    pub next_team_id: u32,
}
```
- Kotlin 的 `playerName`/`team.signature` 降级为展示字段
- 事件/档案/转会/决策点/锁**全部携带 ID**（2026-08-13 已落地）；存档序列化天然是 arena 快照（B1 一并解决）

## 5. 关键设计决策

### D1. 核心 = 纯同步单线程逻辑库
- `csc-core` 及以下**零异步、零 IO**（不依赖 tokio），`no_std` 不需要但保持纯 lib
- 每局模拟 = 独立 `std::thread`（无共享可变状态）→ 多局天然隔离
- server 用 tokio + `spawn_blocking` 桥接；单局内所有推进是同步的（与 Kotlin 同构）

### D2. 确定性策略（跨语言对照的分界线）
- 语言内：**位级可复现**（同种子同决策 → 完全相同的状态序列）——Rust 侧硬性要求
- **概率判定整数化（已落地 2026-08-13）**：`rng.roll_bp(p)` / `rng.next_bp()`（基点 1/10000，
  `next_u64() % 10_000 < round(p × 10_000)`）替代 `rng.next_double() < prob` 浮点分支
  ——1 ulp 的浮点差不再导致跨语言事件序列分叉。RNG 消耗同为 1 个 `next_u64`。
  **连续值抽样**（Box-Muller/加权取点/属性生成）保留 `next_double`（无分支判定，不参与对齐）
- 数值公式（rating/power/疲劳）用 `f64`：加减乘除位级一致；`pow/exp/sqrt` 依赖 libm，
  跨语言可能最后 1-2 ulp 差异 → **对照测试用统计断言**（同种子 N 局：分布/均值/事件序列模式）
- `csc-verify`：Rust 内部重放校验（种子+日志→重算→journal/archive 指纹一致）

### D3. 决策源：保持同步 trait + channel 桥接
```
模拟线程: 推进 → 遇到决策批次 → HumanSource.decide()
                                        │ (mpsc::sync_channel)
server 层: 收集 DecisionPoint → WS 推送前端 → 玩家响应 → send 回决策
```
- 世界级批次与比赛级图间干预**同一通道**（与 Kotlin 两级决策流同构）
- WASM 单机模式：决策源 = 本地 UI 回调（同步返回，无 channel）
- ✅ **已实现（2026-08-13，csc-server）**：世界级批次与比赛级图间干预走同一 channel
  （`ChannelDecisionSource`：批次先发布（WS 推送 + REST 拉取）再阻塞等决策，多批次/月
  的协议要点见 csc-server README 与根 README「Human 协议要点」）；
- WASM v1（csc-wasm）为 auto 推演形态；浏览器内交互模式需步骤化状态机
  （`EngineStep` 枚举，推进到一半挂起）——列为 WASM v2

### D4. 存档 = 全量 arena 快照（一步到位，不做 Kotlin 式妥协）
```rust
#[derive(Serialize, Deserialize)]
pub struct GameState {
    pub version: u32,                    // ★ 格式版本（2026-08-13 加入）
    pub world: World,                    // 实体 arena + VRS + 赛事结果（第 3-5 块）
    pub month: u32,
    pub last_contract_year: i32,
    pub locks: Vec<LockRecord>,          // ★ team_ids（稳定 ID；签名反解已移除）
    pub rng: [u64; 4],
    pub decisions: Vec<PlayerDecision>,
    pub journal: Vec<WorldEvent>,        // ★ 事件含稳定 ID
    pub archive: HashMap<PlayerId, Vec<SeasonRecord>>, // ★ 按 PlayerId 键控
}
```
- 格式：JSON（调试友好）起步；后续 `postcard` 可选（bincode 1.x 已归档不引入；postcard 为 no_std 友好的 serde 格式）
- **版本契约（2026-08-13）**：`deny_unknown_fields` 拒绝未知字段、`from_json_str`
  拒绝未知版本；字段增删/语义变更必须 bump `CURRENT_VERSION` 并在 `migrate` 逐级迁移
- `save → load → 继续推进` 与原路径**逐字节一致**（新增测试门禁）

### D5. WASM 两用策略
- `csc-core` 及以下直接编译 `wasm32-unknown-unknown`（无 IO 即无阻碍）
- `csc-wasm` 薄层：`create(seed)/step/save/load/decide/events` + 事件缓冲
- 同一核心：**服务端权威模式**（WS 连 server）与**本地单机模式**（WASM 全本地）
  两种玩法，一份代码——这是迁移 Rust 的核心红利

### D6. 错误处理分层
- 内部不变量：`assert!`（panic=bug，等价 Kotlin require）
- 外部输入：`Result<T, SimError>`（非法存档/非法决策/缺失 assets → 可恢复错误）
- **已落地（2026-08-13）**：`csc_util::SimError`（Asset/Save/Protocol）；
  assets 语义校验 `VrsDatabase::parse_json_checked`（空队名/负积分/空阵容/非正排名
  显式报错；**同名不同队合法**——真实数据含青训队共用 org 名）；
  `RoleBaseline::from_json_str` / `Engine::load_from_standings` 返回 `Result`，
  坏资产不再 panic

### D7. 前端：纯表现层 + 只读 DTO
- 状态模型镜像 `csc-query` 的 DTO（由 server 推送/拉取）
- 前端零模拟逻辑；决策面板 = `DecisionPoint` JSON 渲染 + 提交
- 回放 = `WorldJournal` 事件流渲染（seq 游标增量，Kotlin 既有协议）

## 6. REST / WebSocket API 草案

```
REST
POST /games                      { seed?, policy: "auto"|"human" } → { game_id }
GET  /games/{id}/state            → GameState（或摘要）
GET  /games/{id}/journal?seq=N    → { events, next_seq }           （增量）
GET  /games/{id}/archive          → 生涯档案
POST /games/{id}/save             → 快照 JSON（下载）
POST /games/{id}/load             { snapshot } → ok
GET  /games/{id}/decision/pending → 当前待决策点

WS /games/{id}
服务端 → 客户端： { type:"decisions", points:[...] }
                 { type:"step", result: StepSummary, journal_delta:[...] }
                 { type:"match", ... }        （比赛级事件）
客户端 → 服务端： { type:"decide", decisions:[...] }
```

## 7. 转写顺序（逐模块交付，每模块含：类型定义 → 核心逻辑 → 单测 → Kotlin 对照）

| 模块 | 内容 | 关键对照点 |
|---|---|---|
| **M0** | workspace 骨架 + `csc-domain` 全量值对象 + cargo test | domain 保持零依赖 |
| **M1** | `csc-util`：xoshiro256\*\* + ID + `csc-time` | 与 DeterministicRandom 同算法 |
| **M2** | `csc-entities`：arena/ID/属性领域（PowerCalculator/RoleProfile/Players/VrsMapper） | 对象图→ID 引用 |
| **M3** | `csc-simulation`：MatchSimulator/WinRate/Form/Growth/... + SeriesResult | 纯函数化；概率整数化 |
| **M4** | `csc-vrs`：VrsDatabase 窗口剪枝 + reseed | 修 B2 |
| **M5** | `csc-tournaments`：排程/赛制/Conductor/结算 | 比赛级决策点返回而非现场消费 |
| **M6** | `csc-systems`：transfer/population/injury/chemistry/economy | 与核心单向 |
| **M7** | `csc-core`：Engine/SeasonDirector/决策流/事件/档案/GameState | 完整存档（无妥协） |
| **M8** | `csc-query` + `csc-verify` + **跨语言对照测试** | Kotlin vs Rust 统计一致 |
| **M9** | `csc-server`（REST/WS + channel 决策源） | HumanSource 桥接 |
| **M10** | `csc-app` CLI + `csc-wasm` | 交互闭环（A1） |
| **M11** | TS 前端（Solid）：决策面板/回放/生涯视图 | 纯表现层 |

## 8. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 浮点跨语言位级差异 | 概率整数化；公式对照用统计断言（D2） |
| 对象图→ID 重构引入引用 bug | arena 不变式测试（ID 必须存在/引用无环）+ 存档往返测试 |
| 决策点生成时序分散（伤病 tick 在批次内） | channel 阻塞决策源（D3），API 与 Kotlin 同构 |
| 全量快照序列化开销 | JSON 起步；存档低频（月/年），postcard 备选——仅当存档体积实测超阈值再评估（P2-⑪），当前 JSON+gzip 够用 |
| 转写工作量（~11 模块） | 逐模块交付 + 每模块对照测试先行（Kotlin 侧跑通 → Rust 侧复刻 → 对照） |

## 9. 与 docs/09 路线图的关系

| docs/09 项 | 处理方式 |
|---|---|
| B3 稳定 ID | ✅ **已落地（2026-08-13）**：实体 arena + 事件/决策点/档案/锁全覆盖；Kotlin 原型保持 name/signature，对照测试按名字映射 |
| B1 完整存档 | ✅ **已落地**：全量 arena 快照 + roundtrip 测试；另有 version/deny_unknown_fields/migrate（存档格式演进契约） |
| B2 matchHistory 无界 | ✅ **已落地**：reseed 按时间窗口剪枝（见 `csc-vrs`） |
| C1 FreeAgentTeam 单例 | ✅ **已落地**：`TeamId::NONE` 哨兵 + 自由市场 arena |
| C2 回放校验器 | ✅ **已落地（csc-verify）**：FNV-1a 世界指纹（JSON 递归键排序）+ 决策日志回灌 + 篡改/分歧显式报错 |
| A1 交互入口 | ✅ **已落地**：csc-app CLI（stdin 决策源）+ csc-server WS（决策面板协议）；前端（M11）待 UX 设计稿 |
| D2 assets 校验 | ✅ **已落地**：`parse_json_checked` 语义校验 + `SimError` 显式报错（M0/M4） |
