# CSC 后端架构分析报告（Archify）

> 分析对象：`backend/` Rust workspace（16 crates，~48.7K 行）
> 分析时间：2026-09 · 基于代码实读（Serena 语义工具）+ `cargo test --workspace` 实跑
> 测试结果：**596 passed / 0 failed / 3 ignored**（ignored 为 soak/fingerprint 门禁，需 release 模式显式触发）

---

## 1. 系统定位与总体架构

**CS 生涯模拟游戏后端**：确定性模拟内核（纯 Rust 逻辑库，可编译 native 与 wasm32）+ axum 服务端（REST + WebSocket）+ 薄 WASM 绑定。核心约束是**确定性可复现**：同种子 + 同决策序列 → 位级一致的世界状态（语言内硬保证）。

### 1.1 三端拓扑

```
TS 前端 (Solid) ──REST/WS(JSON)──► csc-server (axum, 每局一模拟线程)
        │                                │
        └──(可选) WASM 单机模式 ◄── csc-wasm（薄绑定，auto 推演）
                                         │
                          csc-core 及以下：纯同步逻辑库（无 tokio / 无 IO）
                          domain → util/time → entities/events →
                          simulation/vrs/career → systems/tournaments → core
```

依赖方向由 **Cargo 编译期强制无环**（这是从 Kotlin 转写最大的架构红利：Kotlin 靠审查纪律维持的依赖方向，Rust 里由编译器锁死）。

### 1.2 关键设计决策（D1–D7）

| # | 决策 | 状态 |
|---|---|---|
| D1 | 核心 = 纯同步单线程逻辑库；每局模拟 = 独立 `std::thread`；server 用 tokio + `spawn_blocking` 桥接 | ✅ 落地 |
| D2 | 确定性策略：概率判定整数化（`roll_bp` 基点 1/10000）替代浮点分支；RNG 与 Kotlin `DeterministicRandom` 位级一致（8 组黄金值对照） | ✅ 落地 |
| D3 | 决策源同步 trait + channel 桥接（`ChannelDecisionSource`：批次先发布 WS/REST，再阻塞等决策） | ✅ 落地 |
| D4 | 存档 = 全量 arena 快照；版本化契约（`deny_unknown_fields` + `CURRENT_VERSION` + `migrate` 挂载点） | ✅ 落地（v9） |
| D5 | WASM 两用：服务端权威模式 + 本地单机模式，同一核心 | ✅ v1 auto 推演；交互模式列为 v2 |
| D6 | 错误分层：内部不变量 `assert!`（panic=bug）；外部输入 `Result<T, SimError>` | ✅ 落地 |
| D7 | 前端纯表现层 + 只读 DTO；决策面板 = `DecisionPoint` JSON 渲染 | ✅ 落地 |

---

## 2. Crate 分层盘点（16 crates）

| 层 | crate | 行数 | 职责 |
|---|---|---|---|
| L0 | `csc-domain` | ~700 | 跨系统共享值对象（零依赖；全部 `Copy/Clone/PartialEq/Serde`） |
| L0 | `csc-util` | ~965 | 确定性 RNG（`Xoshiro256StarStar`，与 Kotlin 位级一致）+ 稳定 ID + `SimError` |
| L1 | `csc-time` | ~500 | 模拟时钟 `SimClock` + 时间窗口；零日期库依赖，自实现 civil calendar |
| L1 | `csc-text` | ~300 | 文案/叙事文本包（编译期内嵌默认 + assets/text/zh-CN.json 运行期覆盖） |
| L1 | `csc-entities` | ~1600 | 实体 arena（选手/队伍/自由市场）+ 属性领域 + 生成器 + 基线 |
| L1 | `csc-events` | ~1618 | `WorldJournal` 世界事件流：纯值 + 可序列化 + 增量拉取（seq 游标） |
| L1 | `csc-decision` | ~724 | `DecisionPoint` / `DecisionSource` / `DecisionLog`（可复现回放基础） |
| L2 | `csc-simulation` | ~6763 | **纯函数规则层**：胜率/成长/训练/比赛/LIVE 回合模型；无 IO，不反向依赖引擎 |
| L2 | `csc-vrs` | ~1200 | VRS 排名：静态种子分 + 动态 ELO + 窗口剪枝（修 B2：matchHistory 无界） |
| L2 | `csc-career` | ~800 | 生涯档案：`CareerArchive` / `SeasonRecord` / 结局评估 |
| L3 | `csc-systems` | ~4560 | 五个世界子系统合一：chemistry / economy / injury / population / transfer |
| L3 | `csc-tournaments` | ~9045 | 赛事系统：排程/赛制引擎（Swiss/单双败）/日历/结算/TOP20 |
| L4 | `csc-core` | ~7816 | **编排层**：`Engine` 装配全部子系统 + `GameState` 完整存档聚合根 + 查询层 |
| L5 | `csc-server` | ~5081 | axum REST/WS + `GameManager`（并发/容量治理/落盘/LIVE 会话） |
| L5 | `csc-app` | ~645 | 交互式 CLI（`StdinDecisionSource`） |
| L5 | `csc-wasm` | ~316 | 浏览器绑定：薄 API + 事件缓冲（当前仅 auto 推演，无人类交互） |

> 最大的三个 crate（tournaments 9k / core 7.8k / simulation 6.8k）占总代码约一半，与 `docs/analysis/18-文件拆分作战清单.md` 的在途拆分方向一致。

### 2.1 核心数据流

**推进链路**：
```
HTTP advance → GameManager → Command → 模拟线程
  → csc-core Engine（编排：月度 8 步管线）
    → csc-tournaments 编排 + 结算（Swiss/淘汰赛/日历/锁）
    → csc-systems 月 tick（伤病/化学/经济/人口/转会）
    → csc-simulation 纯函数结算（胜率/成长/状态）
    → csc-vrs 更新排名
  → 需要人类输入 → csc-decision 产出 DecisionPoint 并挂起（channel 阻塞）
  → 完成 → 更新 snapshot/view 锁 → 广播 WsEvent::Step
```

**存档链路**：
```
存：Engine::snapshot() → GameState → canonical JSON → CRC32 → gzip → tmp → rename（原子写）
读：文件 → CRC 校验（失败隔离为 .corrupt）→ GameState::from_json_str（版本校验）
    → migrate_state()（版本迁移挂载点）→ Engine::restore_result（构造-交换，失败不留半恢复）
    → loop_::restore_locks()（重建赛事锁）
```

---

## 3. 服务端运行时模型（csc-server）

### 3.1 GameManager —— 每局一个模拟线程

- `game_id: u64` 索引；每局 = 独立 `std::thread` + `mpsc::channel<Command>`（D1 多局天然隔离）
- 共享状态经 `Arc<Mutex<>>` 暴露给路由：`snapshot`（完整 GameState）、`view`（ClientState 轻量视图）、`pending`（待决策批次）、`events_tx`（broadcast WS 事件源，容量 256）

### 3.2 容量治理（LRU 淘汰/恢复）

- `max_active`（默认 4）超出时把最久未访问的**空闲局** gzip 落盘卸载；`leased > 0`（请求租约）/`advancing`/`saving`/`restoring` 中的局不动
- 恢复：`get_or_restore` 读磁盘包 → CRC 校验 → 迁移 → 构造-交换恢复；并发恢复同局由 `restoring` 标记去重（有专门竞态回归测试 `concurrent_restore_of_same_game_never_404s`）
- 磁盘包格式：`gzip(JSON(PersistedGame{ format:"csc-persisted-game", format_version:2, policy, state_crc32, state }))`；v1→v2 自动升级；CRC 失败隔离为 `.corrupt`
- 持久化目录：`CSC_RUNTIME_DIR` 环境变量 → 缺省系统临时目录 `csc-simulation/games`；兼容读回退 `backend/runtime/games`

### 3.3 LIVE 会话（LiveSessionStore）

- 进程级 `HashMap<(game_id, match_id), LiveSession>`；会话只存当前 `LiveRoundState` + 已播回合事件，未来结果留在 `LiveMatchEngine` 内直到客户端 advance
- **start 幂等可重进**（P0-2 修复后）：同 `(game_id, match_id)` 重复 start = 覆盖重建
- 生命周期已接入 GameManager：LRU 淘汰 / `shutdown` 时回收该 game_id 全部会话（修复"会话只增不删"的进程级泄漏）
- `skip` 有 `MAX_ROUNDS=48` 硬上界，永不悬挂

### 3.4 WebSocket 协议

- 应用层心跳（JS 无法发协议级 ping）：服务端每 30s 发 `{type:"ping",ts}`，10s 无 pong 判死关闭；前端对称实现
- 慢消费者：`broadcast` 通道容量 256，溢出丢事件并发 `{type:"lagged",skipped:N}`，REST `journal?since=` 兜底对账
- 决策超时：`DECISION_TIMEOUT = 300s`，超时自动继续（确定性：同种子同决策日志下超时行为可复现，经 `WorldEvent::LiveUpdate` 入 journal）

---

## 4. 存档格式（GameState v9）

唯一聚合根 `csc-core/src/state.rs::GameState`，全量可序列化，字段：

```
version / month / last_contract_year / locks / sim_year|month|day /
rng_state[4] / decisions[] / journal[] / archive{} / world / vrs /
events[] / yearly_rating / top20_history[] / scheduled_records[] / sim_version
```

**版本契约**：
- `deny_unknown_fields`：未知字段拒绝解析（防格式漂移静默默认化）
- 未知版本（> `CURRENT_VERSION=9`）拒绝加载
- 旧档（v1–v8）读入自动迁移到 v9：v5 荣誉积分 ×0.1 量纲归一；其余新增字段 `serde(default)` 补默认
- 迁移入口唯一：`migrate_state()`，服务端 `/load`、`/load/gzip`、磁盘恢复、WASM `csc_restore` 全部走同一入口
- 指纹门禁：`FNV-1a64(sim_version ‖ canonical_json(state))`，`csc-core/tests/fingerprint.rs` 随 `cargo test --workspace` 默认执行（禁止 `#[ignore]`）

演进记录权威文档：`docs/SAVE-FORMAT.md`。

---

## 5. 测试与质量门禁

| 项 | 结果 |
|---|---|
| `cargo test --workspace` | **596 passed / 0 failed / 3 ignored**（soak 240 月 / bench / fingerprint 说明） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过（2026-09-06 审查记录） |
| `cargo fmt --all -- --check` | 已修复（P0-1 已闭环） |
| 确定性黄金值 | `csc-util/src/rng.rs` 8 组与 Kotlin 位级对照 |
| 并发竞态回归 | `concurrent_restore_of_same_game_never_404s` 等专项测试 |
| 存档往返 | save→load→继续推进逐字节一致门禁 |

---

## 6. 已知问题清单

### 6.1 已修复闭环（2026-09-06 审查批次）

| # | 问题 | 修复方式 |
|---|---|---|
| P0-1 | `cargo fmt` 违规已在 HEAD，CI 红 | `cargo fmt --all` 单独 commit |
| P0-2 | `LiveSessionStore` 无任何回收路径（内存泄漏 + 同场无法重进 LIVE） | 加 `end` + shutdown/LRU 淘汰清理 + `start` 幂等覆盖重建 |
| P1-3 | WASM 读档跳过 `migrate_state()`（v1–v4 旧档荣誉分量纲错一个数量级） | 补迁移；顺带收敛"解析即迁移"唯一入口 |
| P1-4 | 读档 panic 留下半恢复引擎继续对外服务 | 构造-交换 + `Result` 化（`restore_result`），不再 `catch_unwind` |
| P1-5 | 新增存档字段（`scheduled_records`/`sim_version`）未 bump `CURRENT_VERSION` | bump 到 9 + 同步 `docs/SAVE-FORMAT.md` |
| P2-6 | `from_json_str` 文档注释声称做迁移但实际不做（误导调用方） | 改正注释 + 入口收敛 |
| P2-7 | `routes/mod.rs` 文档滞后（列 18 实际 47）+ 代码围栏未闭合 | 全量同步端点清单 |
| P2-8 | LIVE 关键回合两套判定口径不一致（实时用回合前比分，复盘用回合后） | 抽共用判定函数，统一以"回合前比分"为唯一口径 |
| P2-9 | debug 构建单月最多 3 次全量快照（NaN 断言开销） | 已闭环 |
| P2-10 | 15 个文件 >700 行 | 在途拆分（见 §6.3） |
| P2-11 | `LockRecord.start_epoch_day` 注释错误（复制粘贴） | 已修正 |

### 6.2 结构性/设计层面的已知限制（非 bug，属当前边界）

| # | 限制 | 说明 |
|---|---|---|
| K1 | **WASM 仅 auto 推演** | 浏览器内交互模式需步骤化状态机（`EngineStep` 枚举，推进到一半挂起）——列为 WASM v2 |
| K2 | **浮点跨语言位级差异** | `pow/exp/sqrt` 依赖 libm，跨语言可能 1-2 ulp 差异；已通过概率整数化消除分支分叉，对照测试用统计断言（非位级）——设计内妥协 |
| K3 | **无跨版本决策重放协议** | 决策点 ID 格式历经两次变更（M1 名字→PlayerId；P2-4 队名→TeamId），旧档历史决策日志与新格式重放失配——影响面仅限同一存档内历史日志展示 |
| K4 | **`/state` 端点不可高频轮询** | 20 年生涯完整快照 ≈68MB；高频轮询必须走 `/view` + `/journal?since=` + WS（前端契约已明示） |
| K5 | **请求体上限 128MB** | 长生涯读档需要；axum 默认 2MB 会把 `/load` 打成 413——已显式放行，但意味着单请求内存峰值高 |
| K6 | **存档 JSON 起步** | 体积大但调试友好；`postcard`（no_std 友好 serde 格式）为备选——仅当实测超阈值再评估，当前 JSON+gzip ≈1/10 够用 |

### 6.3 在途工作（不阻塞，跟着既有计划走）

| # | 事项 | 依据 |
|---|---|---|
| W1 | 巨文件拆分：`csc-tournaments/src/engine.rs` 2040 行、`csc-core/src/engine.rs` 1814 行、`csc-simulation/src/live_match.rs` 1486 行、`csc-tournaments/src/conductor.rs` 1298 行 | `docs/analysis/18-文件拆分作战清单.md` |
| W2 | M11 TS 前端 UX 设计稿 | `backend/ARCHITECTURE-MAPPING.md` §7 |
| W3 | WASM v2 交互模式（`EngineStep` 状态机） | D5 |

---

## 7. 工程亮点（审查确认真实有效）

1. **确定性内核有真门禁**：RNG 8 组黄金值位级对照；`roll_bp` 整数基点判定从根上消除跨语言浮点分叉；`next_i32_bound` 复刻上游行为同时修正负数缺陷并配白盒回归
2. **存档纪律四件套齐全**：`deny_unknown_fields` + 未知版本显式拒绝 + 独立迁移挂载点 + 往返测试（P1-5 是执行漏 bump，不是机制缺失，已闭环）
3. **持久化工程化**：canonical JSON + CRC32 + 篡改隔离 `.corrupt` + tmp+rename 原子写 + v1→v2 自动升级——每条都有对应测试
4. **并发协议扎实**：`GameHandle` 租约防使用中被淘汰、`restoring` 标记去重并发恢复、只淘汰空闲局、收缩不在请求关键路径；有专门竞态回归测试
5. **分层无环编译器强制**：不靠人工守约；规则层保持纯函数，引擎只做编排
6. **CI 覆盖面超出常规**：test/clippy/fmt + WASM 编译 + 资产 schema 校验 + 20 年 soak + REST/WS 冒烟
