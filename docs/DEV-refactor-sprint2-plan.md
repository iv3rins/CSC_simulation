# Sprint 2 巨文件拆分方案（可直接派单执行）

> 给其它模型/队员的可执行任务书。每个任务都可独立 PR、独立验证、独立回滚。
> 按 AGENTS.md §10.5 任务模板写：**第一步永远是读"文档定位"，读完先输出实现计划再写代码。**
>
> **前置状态**（已验证）：
> - 596 测试全绿 / clippy 零警告 / fmt 通过（当时前端 typecheck + vitest 35/35；前端 2026-09 已移除）
> - Sprint 1 已完成去重（见 docs/analysis/16）
> - 当前工作树干净，本方案从 `HEAD` 直接开工
>
> **铁律**（所有任务都必须遵守）：
> 1. **零行为变化**——只是物理移动代码，公开 API 路径与签名一字不改；
> 2. **每改一个文件跑一次验证**（§4.4 门禁），挂了就回滚；
> 3. **测试随代码一起搬**——`#[cfg(test)] mod tests` 跟它测的函数走；
> 4. **不新增任何功能、不改任何逻辑、不改任何注释语义**——只挪位置 + 修 import + 修 pub 可见性。

---

## 任务总览

| # | 任务 | 目标文件 | 当前行数 | 拆分后 | 预估工作量 |
|---|---|---|---|---|---|
| **T1** | csc-server/routes/mod.rs 按领域拆 6 文件 | routes/mod.rs | 1761 | 7 个文件，每个 ≤400 行 | 2-3h |
| **T2** | csc-server/game/mod.rs 拆 4 文件 | game/mod.rs | 2224 | 5 个文件，每个 ≤700 行 | 2-3h |
| **T3** | csc-core/client.rs 拆 5 文件 | client.rs | 1847 | 6 个文件，每个 ≤500 行 | 2-3h |

**推荐执行顺序**：T1 → T2 → T3（T1 最简单，T3 测试最多）。三个任务**无依赖**，可并行；但同一文件只能一个人改。

---

## T1：拆分 csc-server/routes/mod.rs

### T1.1 文档定位

- `docs/analysis/10-服务端与运行时-csc-server-app-wasm.md` — 服务端 API 全景
- `docs/analysis/16-架构整理报告-2026-09.md` §二 P1-3 — 拆分理由

### T1.2 现状

单文件 1761 行，57 个 fn，其中 43 个 axum handler。所有 handler 共享：
- `err(status, msg)` 统一错误响应（line 115）
- `game_of(state, id)` 租约提取（line 128）
- `compress_save` / `run_save_op`（存档辅助，line 846/878）
- `entry_load_via`（load 辅助，line 1065）

**handler 分组**（按 axum 路由表 line 62-113）：

| 组 | 路由 | handler | 行数估算 |
|---|---|---|---|
| 基础 | /health | health | ~20 |
| 生命周期 | POST /games, DELETE /games/:id, /save, /save/gzip, /load, /load/gzip | create_game, shutdown_game, save_game, save_game_gzip, load_game, load_game_gzip | ~300 |
| 查询 | /state, /view, /calendar, /events/:name, /fixtures/watched, /news, /replay, /summary, /journal, /archive, /ending, /top20*, /world, /insights, /calibration | get_state, get_view, get_calendar, get_event_aggregate, mark_fixture_watched, get_news, get_replay, get_summary, get_journal, get_archive, get_ending, get_top20, get_top20_history, get_insights, get_world_stories, get_top20_reference, get_calibration | ~700 |
| 推进 | /advance, /advance-season | advance, advance_season | ~150 |
| 决策 | /decisions/*, /training/*, /season-goal, /finance | get_pending, submit_decisions, get_training_options, set_season_goal, plan_training, post_finance | ~250 |
| 转会 | /transfers/* | get_transfer_market, get_transfer_offers, sign_transfer | ~80 |
| LIVE | /live/*, /ws | start_live, get_live, advance_live, decide_live, get_live_review, skip_live, mark_live_watched, get_live_today, skip_live_today, ws_handler, ws_loop | ~250 |

### T1.3 目标结构

```
crates/csc-server/src/routes/
├── mod.rs           ← 只保留 router() 汇总 + 共享 use re-export + err/game_of 公共辅助
├── lifecycle.rs     ← 生命周期 + 存档（含 compress_save / run_save_op / entry_load_via）
├── query.rs         ← 全部 GET 查询 handler
├── advance.rs       ← advance + advance_season
├── decision.rs      ← decisions/training/season-goal/finance
├── transfer.rs      ← transfers/market/offers/sign
└── live.rs          ← live/* + ws_handler + ws_loop
```

### T1.4 实施步骤

**Step 1：先抽公共辅助**（30 min）

在 `routes/mod.rs` 中保留：
- `pub fn router()`（router 表）
- `pub(crate) fn err(...)`
- `pub(crate) async fn game_of(...)`
- 所有共享 `use` 语句

修改 `err` 和 `game_of` 的可见性为 `pub(crate)`，让子模块可用。

**Step 2：按顺序迁移**（每个迁移完跑 `cargo check -p csc-server`）：

1. **transfer.rs**（最小）：把 1374-1429 行的 3 个 handler 整体剪切到新文件；
   - `pub(super) async fn get_transfer_market / get_transfer_offers / sign_transfer`
   - 在 mod.rs 加 `mod transfer; use transfer::*;`（保持 router() 调用点不变）
2. **live.rs**：把 1483-1760 行的 10 个 handler 迁出；
3. **decision.rs**：把 1227-1372 行的 6 个 handler 迁出；
4. **advance.rs**：把 1093-1226 行的 2 个 handler 迁出；
5. **lifecycle.rs**：把 174-240 + 846-1091 行的 6 个 handler + 3 个辅助迁出；
6. **query.rs**：剩余所有 GET handler 迁出。

**Step 3：调整 router()**

`router()` 函数体内的 `.route(...)` 调用**全部保留在 mod.rs**，handler 函数名不变、路由路径不变。

**Step 4：修 import**

每个新文件顶部：
```rust
use super::{err, game_of};  // 共享辅助
use crate::state::AppState;
use axum::{...};  // 按需
```

### T1.5 验收

```bash
cargo test -p csc-server --locked       # 全绿（应当还是 ~50 个测试）
cargo clippy -p csc-server --all-targets -- -D warnings
cargo fmt --all -- --check
wc -l crates/csc-server/src/routes/*.rs  # 每个文件 ≤ 500 行
grep -c "^async fn\|^pub(super) async fn" crates/csc-server/src/routes/mod.rs  # 应该 ≤ 5
```

### T1.6 边界

- **不动** `crates/csc-server/src/game/*`（T2 负责）；
- **不动** `crates/csc-server/src/state.rs` / `live.rs`（独立模块）；
- **不改** 任何 handler 的函数签名/请求路径/响应格式；
- **不删除** 任何现有测试。

---

## T2：拆分 csc-server/game/mod.rs

### T2.1 文档定位

- `docs/analysis/10-服务端与运行时-csc-server-app-wasm.md` — GameManager/GameEntry 并发契约
- `docs/analysis/16-架构整理报告-2026-09.md` §二 P1-3
- **必读**：`crates/csc-server/src/game/mod.rs` 开头 20 行的模块文档（讲清 actor 模型 + 租约协议）

### T2.2 现状

2224 行，4 个角色混杂：

| 角色 | 内容 | 行范围 |
|---|---|---|
| 值类型 | Policy, PersistedGame, PendingBatch, StepSummary, WsEvent, WsIn, AdvanceStep | 73-268 |
| 决策源 | ChannelDecisionSource + Command enum + run_advance_steps + finish_advance | 186-379 |
| GameEntry actor | GameEntry struct + impl（24 方法）+ GameHandle + SaveGuard + RestoreInProgress | 431-850 |
| GameManager | GameManager struct + impl（20 方法，694 行 impl 块） | 852-1566 |
| 存档辅助 | transfer_market_from_pending, canonical_state_bytes, build_persisted_envelope, write_persisted_package | 49-104, 1569-1595 |
| 测试 | #[cfg(test)] mod tests | 1597-2224 |

### T2.3 目标结构

```
crates/csc-server/src/game/
├── mod.rs           ← 只保留模块文档 + pub use 汇总
├── types.rs         ← Policy/PersistedGame/PendingBatch/StepSummary/WsEvent/WsIn/AdvanceStep
├── decision.rs      ← ChannelDecisionSource + Command + run_advance_steps + finish_advance
├── entry.rs         ← GameEntry + GameHandle + SaveGuard + RestoreInProgress
├── manager.rs       ← GameManager
└── persist.rs       ← transfer_market_from_pending + canonical_state_bytes + build_persisted_envelope + write_persisted_package + GameLoader
```

测试按职能跟走：
- `entry.rs` 内的测试 → 留在 entry.rs 尾部
- `manager.rs` 内的测试 → 留在 manager.rs 尾部
- `persist.rs` 内的测试 → 留在 persist.rs 尾部
- 跨模块集成测试 → 留在 `mod.rs` 尾部

### T2.4 实施步骤

**Step 1：先抽 types.rs**（30 min）

把以下值类型整体剪切：
- `pub enum Policy`
- `pub struct PersistedGame`
- `pub struct PendingBatch`
- `pub struct StepSummary`
- `pub enum WsEvent`
- `pub enum WsIn`
- `pub enum AdvanceStep`（非 pub 但跨模块用）

加 `pub use types::*;` 到 mod.rs。**注意**：所有 derive 属性、serde 属性、doc comment 必须原样保留。

**Step 2：抽 persist.rs**（30 min）

把以下函数整体剪切：
- `pub struct GameLoader` + impl
- `fn canonical_state_bytes`
- `fn transfer_market_from_pending`
- `fn build_persisted_envelope`
- `fn write_persisted_package`

公开性：外部只需要 `GameLoader`，其他保持 `pub(crate)` 或 `pub(super)`。

**Step 3：抽 decision.rs**（45 min）

把以下整体剪切：
- `pub struct ChannelDecisionSource` + impl DecisionSource
- `enum Command`
- `fn run_advance_steps`
- `fn finish_advance`

注意：`run_advance_steps` 调 `WsEvent::Step` 等，需要 `use super::types::*;`。

**Step 4：抽 entry.rs**（45 min）

把 `GameEntry` + `GameHandle` + `SaveGuard` + `RestoreInProgress` 及其 impl 整体剪切。

注意：`GameEntry` 的方法被 routes/* 大量调用，**所有 pub 方法签名一字不改**。

**Step 5：抽 manager.rs**（45 min）

把 `GameManager` struct + impl 整体剪切。

注意：`GameManager::spawn_entry` 内部用 `Engine`/`ChannelDecisionSource`/`Command` 等，需要 `use super::{decision::*, entry::*, types::*};`。

**Step 6：mod.rs 收尾**

只保留：
- 模块文档（开头 20 行 `//!` 注释）
- `pub mod types; pub mod decision; pub mod entry; pub mod manager; pub mod persist;`
- `pub use types::*; pub use entry::*; pub use manager::*; pub use persist::GameLoader;`（decision 不 re-export，外部不直接用）
- 共享常量 `WS_CHANNEL_CAP` / `DECISION_TIMEOUT`（放在 mod.rs 或 types.rs，但 pub use 出去）

### T2.5 验收

```bash
cargo test -p csc-server --locked
cargo clippy -p csc-server --all-targets -- -D warnings
cargo fmt --all -- --check
wc -l crates/csc-server/src/game/*.rs   # 每个 ≤ 800 行
# 启动服务端烟测：
cargo run -p csc-server -- ../assets 8080 &
sleep 2
curl http://127.0.0.1:8080/health
curl -X POST http://127.0.0.1:8080/games -H "Content-Type: application/json" -d '{"seed":42,"policy":"auto"}'
```

### T2.6 边界

- **不改** `GameEntry` / `GameManager` 的任何方法签名；
- **不改** 任何字段名/字段顺序；
- **不动** `routes/*`（如果 T1 已合入，以新结构为准；否则按旧路径 mod.rs 调整 use）；
- **不删除** 任何测试；
- **actor 并发契约不变**——模拟线程独占 Engine、共享只有 Arc<Mutex<GameState>>。

---

## T3：拆分 csc-core/client.rs

### T3.1 文档定位

- `docs/analysis/09-核心引擎-csc-core.md` — ClientState 轻量视图契约
- `docs/analysis/16-架构整理报告-2026-09.md` §二 P1-2

### T3.2 现状

1847 行，6 个职责：

| 职责 | 内容 | 行范围 | 公开函数 |
|---|---|---|---|
| 轻量视图 | CareerMilestone/TeamRef/ClientSeason/ClientWorld/ClientTournamentResult/ClientBracketSlot/MajorPresentation/ClientState | 44-419 | client_state_from（pub(crate)） |
| 年度赛历 | season_calendar_plan / calendar_results_for_year | 420-502 | 2 个 pub fn |
| 主角赛历 | PlayerCalendarEvent/ClientFixture/ClientFixtureResult/ClientFixtureScore + project_fixtures/scheduled_status_label/lightweight_tournament/player_calendar_events | 503-780 | 2 个 pub fn |
| LIVE 闸门 | LiveGateFixture/LivePlayerSlot/player_live_fixtures | 781-894 | 1 个 pub fn |
| 赛事聚合 | EventTeam/EventChampion/EventAggregate/event_aggregate | 895-1031 | 1 个 pub fn |
| 测试 | 27 个 #[test] | 1032-1847 | — |

### T3.3 目标结构

```
crates/csc-core/src/client/
├── mod.rs              ← pub use 汇总 + CLIENT_VIEW_VERSION
├── state.rs            ← ClientState 等 8 个结构 + client_state_from + series_has_player
├── calendar.rs         ← season_calendar_plan / calendar_results_for_year / player_calendar_events + 关联结构
├── live_gate.rs        ← LiveGateFixture / LivePlayerSlot / player_live_fixtures
├── event_aggregate.rs  ← EventTeam / EventChampion / EventAggregate / event_aggregate
└── tests.rs            ← 集成测试（跨子模块的；或者按职能分散到各子模块尾部）
```

### T3.4 实施步骤

**Step 1：创建 client/ 目录，先移 state.rs**（45 min）

把 44-419 行剪切到 `client/state.rs`：
- 所有 8 个结构（CareerMilestone/TeamRef/ClientSeason/ClientWorld/ClientTournamentResult/ClientBracketSlot/MajorPresentation/ClientState）
- `pub const CLIENT_VIEW_VERSION`
- `fn series_has_player`
- `pub(crate) fn client_state_from`

注意：`client_state_from` 235 行巨函数，**不重构内部逻辑**——只是物理移动。

**Step 2：移 calendar.rs**（30 min）

把 420-780 行剪切：
- `season_calendar_plan` / `calendar_results_for_year`
- `PlayerCalendarEvent` / `ClientFixture` / `ClientFixtureResult` / `ClientFixtureScore`
- `project_fixtures` / `scheduled_status_label` / `lightweight_tournament` / `player_calendar_events`

**Step 3：移 live_gate.rs**（15 min）

把 781-894 行剪切：`LiveGateFixture` / `LivePlayerSlot` / `player_live_fixtures`。

**Step 4：移 event_aggregate.rs**（15 min）

把 895-1031 行剪切：`EventTeam` / `EventChampion` / `EventAggregate` / `event_aggregate`。

**Step 5：测试分配**（30 min）

`client.rs` 内 27 个测试（1032-1847 行）按被测对象分配：

| 测试 | 应跟随 |
|---|---|
| `client_state_*` / `test_calendar_state` / `season_calendar_plan_matches_engine_density` | state.rs / calendar.rs |
| `player_calendar_events_*` / `fixtures_survive_*` / `next_milestone_*` | calendar.rs |
| `event_aggregate_*` / `aggregate_state` | event_aggregate.rs |

放在各子模块尾部 `#[cfg(test)] mod tests`，或者抽到 `client/tests.rs` 作为集成测试。

**Step 6：mod.rs 收尾**

```rust
pub mod state;
pub mod calendar;
pub mod live_gate;
pub mod event_aggregate;

pub use state::{CLIENT_VIEW_VERSION, CareerMilestone, ClientBracketSlot, ClientSeason, ClientState, ClientTournamentResult, ClientWorld, MajorPresentation, TeamRef, client_state_from};
pub use calendar::{ClientFixture, ClientFixtureResult, ClientFixtureScore, PlayerCalendarEvent, calendar_results_for_year, player_calendar_events, scheduled_status_label, season_calendar_plan};
pub use live_gate::{LiveGateFixture, LivePlayerSlot, player_live_fixtures};
pub use event_aggregate::{EventAggregate, EventChampion, EventTeam, event_aggregate};
```

**注意**：`csc-core/src/lib.rs` 里 `pub use client::{...}` 的 re-export **保持不变**——外部消费方（csc-server/game、csc-server/routes）完全无感知。

### T3.5 验收

```bash
cargo test -p csc-core --locked          # 全绿（应当还是 ~150 个测试）
cargo clippy -p csc-core --all-targets -- -D warnings
cargo fmt --all -- --check
wc -l crates/csc-core/src/client/*.rs    # 每个 ≤ 600 行
# 整工作树验证（client 被 csc-server 消费）：
cargo test --workspace --locked           # 596 全绿
```

### T3.6 边界

- **不改** `client_state_from` 的参数列表/返回类型/内部逻辑；
- **不改** 所有 pub fn 的签名；
- **不改** `csc-core/src/lib.rs` 的 re-export 列表（只在内部调整路径）；
- **不删除** 任何测试；
- **ClientState 18 个字段一个不动**。

---

## 通用验收（所有任务完成后）

```bash
# 后端
cd backend
cargo test --workspace --locked           # 596 全绿
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build -p csc-wasm --target wasm32-unknown-unknown

# （历史记录：前端当时存在，本 Sprint 未动；2026-09 已移除）
npm test

# 服务端烟测（T2 后必做）
cd ../backend
cargo run -p csc-server -- ../assets 8080 &
SERVER_PID=$!
sleep 3
curl -s http://127.0.0.1:8080/health | jq .
GAME_ID=$(curl -s -X POST http://127.0.0.1:8080/games -H "Content-Type: application/json" -d '{"seed":42,"policy":"auto","player_name":"T"}' | jq -r .game_id)
curl -s -X POST "http://127.0.0.1:8080/games/$GAME_ID/advance" -H "Content-Type: application/json" -d '{"months":1}' | jq .
curl -s "http://127.0.0.1:8080/games/$GAME_ID/view" | jq '.date'
kill $SERVER_PID
```

---

## 回滚策略

每个任务独立 PR：
- T1 失败：`git revert <T1-commit>`（不动 T2/T3）
- T2 失败：同上
- T3 失败：同上

如果中途某个迁移步挂掉：
```bash
git diff --stat                                # 看改了哪些文件
git checkout -- <path/to/file>                 # 单文件回滚
cargo check -p csc-server                      # 确认恢复编译
```

---

## 常见坑（本仓库实测）

1. **import 路径**：Rust `mod foo;` + `use foo::bar` vs `use crate::foo::bar` 在不同层级不同——子模块里引用兄弟模块用 `use super::brother::X`，引用 crate 根用 `use crate::foo::X`；
2. **可见性**：`fn` → `pub(crate) fn` → `pub(super) fn` → `pub fn` 的区别，迁移时优先用最小可见性（`pub(super)`），编译报错再放大；
3. **测试跟随**：`#[cfg(test)] mod tests` 里的 `use super::*;` 在移动后会指向新模块——检查每个测试用例的 use 是否仍有效；
4. **serde 属性**：`#[serde(rename_all = "...")]` / `#[serde(tag = "...")]` 等必须随结构一起搬，不能丢；
5. **doc comment**：`///` 和 `//!` 注释必须跟代码一起搬，不能丢失——这是 docs/analysis 的事实源；
6. **`pub use` re-export 顺序**：`csc-core/src/lib.rs` 里的 `pub use client::{...}` 列表**一字不改**，保持外部消费方无感知。

---

## 预估收益

| 指标 | 拆分前 | 拆分后 |
|---|---|---|
| routes/mod.rs | 1761 行 | 7 文件 × ~250 行 |
| game/mod.rs | 2224 行 | 5 文件 × ~450 行 |
| client.rs | 1847 行 | 6 文件 × ~310 行 |
| 单文件最大行数 | 2224 | ~700 |
| 定位一个 handler 时间 | 全文件搜 | 按领域直接进 |
| code review diff | 整文件 | 仅目标模块 |

**所有改动合计**：~5832 行物理搬移，**零逻辑变化**。

---

## 派单建议

按 AGENTS.md §1 的通道规则，推荐：
- **写码队员**：deepseek-v4-flash-vip × 3（T1/T2/T3 并行）
- **审查**：deepseek-v4-pro-vip × 1（完成后整体 review）
- **队长**：当前模型监控 + 合并

**任务下发时**，把对应任务编号（T1/T2/T3）的整段 §T?.1-§T?.6 复制给队员，并在开头加上：
> 第一步读 docs/analysis/10（服务端）或 09（核心引擎）对应章节 + docs/analysis/16 §二 P1-2/P1-3；
> 读完先输出你的实现计划（含每个文件的迁移清单 + 测试分配），再动手写码。
> 每迁移一个文件跑一次 `cargo check -p csc-server` 或 `cargo check -p csc-core`，挂了就回滚。
> 全部完成后跑「通用验收」清单。
