# 项目审查报告（2026-08-18）

> 审查工具：Serena（符号级核查）+ 门禁实跑（cargo / tsc / vitest）
> 审查范围：后端 workspace 16 crates（146 个 .rs / 42,167 行）+ 前端（72 个 ts/tsx/css / 15,420 行）
> 审查基线：工作树（含当日 tier_importance 收敛与弹窗 LIVE 改造，均未提交）
> 方法：每项结论均有符号级或行级证据（标注文件与符号路径）；见文末「审查流程」。

---

## 0. 总判定

**✅ 整体健康：架构分层纪律好、确定性契约落地扎实、门禁全绿；无阻塞缺陷。**
发现 1 项低危设计债 + 3 项观察项（均不影响正确性与确定性）。

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace`（后端 16 crates） | ✅ 全绿（0 失败；含 1 个 34s 存档恢复长测） |
| `cargo clippy --workspace --all-targets` | ✅ 零告警 |
| `npx tsc --noEmit`（前端） | ✅ 零错误 |
| `npx vitest run`（前端单测） | ✅ 4 文件 35 测试全过 |

---

## 1. 架构与分层核查（通过）

### 1.1 后端 crate 分层 ✅
- workspace 16 crates 与 `ARCHITECTURE-MAPPING.md` 规划一致（`backend/Cargo.toml` 实证）；
  注释中规划的 `csc-query` / `csc-verify` 尚未创建（见 §4 观察项 O-3）。
- 依赖方向健康：`csc-domain`（共享词表：`TourneyTier` / `EventImportance` / `TierProfile`）
  位于最底层，`csc-simulation`（纯模拟）、`csc-tournaments`（赛事编排 + conductor 决策循环）
  向上依赖，`csc-server`（axum + WS 桥接）仅做协议层。
- WASM 共享内核就位：`csc-wasm/src/lib.rs` 暴露 `csc_create / csc_advance / csc_snapshot_json /
  csc_restore / csc_journal_json` 五个薄绑定，核心无 IO 可直接编译 wasm32（boxcars/Jumpy 同款模式）。

### 1.2 服务端并发模型 ✅
- **一局一线程 + 命令通道**（`game/mod.rs` `GameManager::spawn_entry`）：模拟线程独占
  `Engine`，外部经 `Command` mpsc 通信；`GameEntry` 用 `advancing/saving/persisting/leased`
  四个原子量管并发资格。这是把「内核确定性」与「服务端异步」正确解耦的结构。
- **决策三件套**（`GameEntry::submit_decisions`）：advancing 前置校验 → batch_id 校验 →
  `try_send` 防阻塞（Full→429 / Disconnected→通道断开）。注释明确「pending 清理权在模拟线程」，
  与 `ChannelDecisionSource::decide` 末尾的清理对称——排除了「提交方清 pending 与新批次写入
  竞态」这一类协议死锁。
- **决策超时路径确定性**（`decide` 超时分支）：按 Auto 权威默认决策继续 + 追加 journal 提示
  事件（不消费 RNG、不改世界状态）→ 同种子同决策日志下超时行为可复现，指纹门禁不漂移。
- **panic 隔离**（`run_advance_steps`）：`catch_unwind` 包裹单步推进，panic 不杀局，
  报错文案明示「可读档恢复」。
- **心跳对称**（`routes/mod.rs::ws_loop`）：服务端 30s 主动 ping + 10s 无 pong 判死，
  与前端 `ws.ts` 双向对称；`pong_deadline` 在 ping 下一轮 select 才可能就绪，无「刚发即判死」竞态。
- **LRU 淘汰安全**（`GameManager::evict_lru_if_needed`）：只淘汰非 advancing/saving/persisting/
  leased/restoring 的最久未访问局；restore 期间的局受 `restoring` 集合保护，避免「磁盘包已读入
  内存但尚未重写」窗口内被误删。容量治理策略（预淘汰 + 全忙跳过）有明确注释边界。

### 1.3 LIVE 观战会话 ✅
- **服务端权威单回合推进**（`LiveSessionStore::advance`）：客户端不预测任何回合；
  `pending` 决策未消费时 advance 直接 CONFLICT（409），防止跳过关键回合。
- **decide 只排队不模拟**（`decide`）：决策进入 `queued_decision`，由下一次 advance 恰好
  消费一次——决策序列在输出中可见、可复现。
- **skip 有硬上界**（`skip` 的 `MAX_ROUNDS`）：若引擎边界缺陷导致永不 MapEnd，循环触界后
  强制收束（无 RNG 消费，仅改 phase），保证「跳过至赛果」永不悬挂。
- **HTTP 语义正确**（`routes/mod.rs` advance_live/decide_live）：「不存在」→ 404，
  其余会话错误 → 409；start 重复 → 409（前端凭 409 走重连恢复 `api.live`）。

---

## 2. 确定性 / 存档 / 版本化契约核查（通过）

| 契约 | 证据 | 结论 |
|---|---|---|
| RNG 隔离 | `csc-util/rng.rs` `Xoshiro256StarStar`（splitmix64 种子扩展 + 显式 `snapshot/from_state`）；simulation 全域仅见 `Xoshiro256StarStar` 注入，无 `rand::thread_rng` 类全局源 | ✅ |
| 拒绝采样位级同构 | `next_i32_bound`：u32 wrapping 运算与 Kotlin `(nextLong() ushr 1).toInt()` 位模式一致，注释记录 2026-08-13 负 v 修正；并警告与双参 `next_i32_in` 原语不可混用 | ✅ |
| 存档版本拒绝 | `GameState::CURRENT_VERSION = 8`（state.rs L120）；`from_json_str` 拒绝 v>CURRENT + `deny_unknown_fields`（state.rs L28/L55）拒绝未知字段——格式漂移静默默认化的两条路都被堵死 | ✅ |
| 迁移链 | `migrate_state`：v<5 荣誉量纲 ×0.1 → 归一 CURRENT；`server/game/mod.rs::restore_from_disk` 包格式 v1→v2 自动升级（CRC 校验失败隔离为 `.corrupt`，v1 重写为 v2） | ✅ |
| CRC 完整性 | 持久化包 v2 校验 `crc32fast::hash(canonical_state_bytes(state))`；canonical JSON 保证哈希输入稳定 | ✅ |
| journal 游标 | `WorldJournal` 单调 `next_seq` 盖章，`cursor()`、`since(seq)` 与 WS `next_seq = cursor()+1` 三方同口径；server `run_advance_steps` 广播与 REST `/journal?since=` 契约一致 | ✅ |
| 模拟版本标签 | `WORLD_SIM_VERSION = 1` 常量挂接指纹门禁；注释明确「任何改变 RNG 消费序的改动必须 bump」 | ✅ |
| NaN 防御 | `advance_month` 在 `debug_assertions` 下走 `crate::nan::assert_no_nan(&snapshot)` | ✅ |

---

## 3. 发现清单（全部非阻塞）

### 发现 1（低）：`EventImportance` 的引擎侧语义与日历侧语义同名不同义
- **位置**：`csc-domain/event_importance.rs`（玩家体验分层，Major→`EventImportance::Major`）
  vs `csc-tournaments/src/calendar.rs` L574 的 tier→日历重要性 match（Major≠Championship）。
- **现象**：两处「importance」枚举同名但口径不同（玩家 LIVE 体验 vs 日历展示位），
  有注释声明是有意为之，但枚举名未区分，新读者易混。
- **建议**：不动行为；在未来触碰这两个面时补充一行交叉引用注释
  （「日历 importance ≠ 玩家体验 EventImportance，见 calendar.rs」），或长期改名为
  `CalendarSlotImportance`（需独立重构任务书承载）。

### 观察项 O-1（信息）：`scheduled_importance` 已是纯别名
- `engine.rs::scheduled_importance` 现在只是 `tier_importance` 的薄转发（收敛后的合理形态）。
  若后续无独立价值，可在下一轮重构中删别名、调用点直连 `csc_domain::tier_profile::tier_importance`，
  再减一层间接。当前保留无害（doc 已声明「仅为别名」）。

### 观察项 O-2（信息）：`submit_decisions` 的 pending 清理注释依赖两处纪律
- 「pending 清理权在模拟线程」由 `GameEntry::submit_decisions` 注释 +
  `ChannelDecisionSource::decide` 实现共同保证，属于「靠注释维持的不变量」。
  建议未来加一条单元测试固化（人为构造提交后 pending 状态断言仍存在），把纪律变门禁。

### 观察项 O-3（信息）：workspace 注释中的 `csc-query` / `csc-verify` 未落地
- `backend/Cargo.toml` 注释仍列出这两个规划 crate；`ARCHITECTURE-MAPPING.md` 提到的
  存档指纹工具未在仓库内找到对应实现（golden 生成脚本在 `backend/tools/*.kt`）。
  规划与现实有差，建议在 Cargo.toml 注释标注「已合并/延后」状态，防误导。

---

## 4. 前端复核（与本次弹窗 LIVE 改造相关）

- `tsc --noEmit` 零错误 + vitest 35 测试全过（见 §0 门禁）。
- 协议消费面（`api/client.ts`）与后端路由一一对应（start/live/advance/decide/skip/review/watch/today），
  409 双重含义（会话已存在→重连恢复；等待决策→恢复视图）在前端均有分支处理。
- Cyrillic `wеароn` 标识符缺陷已在本日早前修复（全库 grep 零残留）。
- 弹窗 LIVE 的关闭语义（最小化保留会话 / 退出结束会话 / 刷新不自动弹）与
  store `liveModalOpen` 状态机一致，无孤儿会话路径。

---

## 5. 审查流程（可复用）

本报告采用「符号面优先、门禁收尾」的 Serena 审查法，全程 12 步：

1. **激活项目**：`activate_project` + `get_current_config` 确认语言服务器就绪（rust LSP）。
2. **定范围**：`list_dir` 枚举 crate/目录结构；`find_file` 补齐子模块文件清单；
   统计行数（146 文件 / 42,167 行后端；72 文件 / 15,420 行前端）确定抽样深度。
3. **抓契约锚点**：`search_for_pattern` 定位全局契约符号——
   `CURRENT_VERSION`（存档版本）、`deny_unknown_fields`（格式漂移拒绝）、
   `Xoshiro256StarStar`（RNG 隔离）、`deny/409/CONFLICT`（协议语义）。
4. **符号面概览**：对每个目标文件 `get_symbols_overview`（先看骨架，不读全文），
   对关键结构 `find_symbol(depth=1)` 列方法清单（如 `impl GameManager` 22 个方法一次拿全）。
5. **按需读体**：只对安全关键符号 `include_body=True`——本报告读了
   `restore_from_disk / evict_lru_if_needed / submit_decisions / decide(Channel) /
   run_advance_steps / ws_loop / next_i32_bound / from_json_str / migrate_state /
   LiveSessionStore.{start,advance,decide,skip}` 共 14 个符号体。
6. **并发/安全路径追踪**：沿「命令入口（HTTP/WS）→ GameEntry 原子资格 → 模拟线程
   Command 循环 → DecisionSource 阻塞点」追踪一轮，核对每个 `lock().expect()` 的
   锁序与每个原子量的 set/reset 所有者（advancing 归模拟线程、saving 归保存路径）。
7. **确定性契约专项**：RNG 种子链 → 快照/恢复（`snapshot/from_state`）→ 决策日志入存档 →
   超时路径不消费 RNG；四点连成线验证「同种子同决策 = 同结果」。
8. **前端契约对齐**：`search_for_pattern` 抓 client.ts 的 live 端点族，与
   `routes/mod.rs` 路由表逐一对照（方法/路径/状态码）；核对前端对 404/409 的分支语义。
9. **LSP 诊断**：`get_diagnostics_for_file(min_severity=2)` 对改动密集文件
   （game/mod.rs、engine.rs）过一遍编译器级诊断（本报告均空）。
10. **门禁实跑（独立证据，不信任静读）**：
    `cargo test --workspace` → `cargo clippy --workspace --all-targets` →
    `npx tsc --noEmit` → `npx vitest run`。四条全绿才允许「通过」结论。
11. **发现分级**：每条发现必须带 符号路径 + 原文证据 + 影响 + 建议；
    按「阻塞 / 低危 / 观察项」三级归类，禁止无证据断言。
12. **报告落盘**：写入 `docs/PROJECT-REVIEW-YYYY-MM-DD.md`（本文件），
    结论先行、证据可溯源、建议可执行（并注明「行为改动需独立重构任务书」边界）。

> 经验注记：本审查中 Serena 符号体/控制台对含反引号+中文的行显示偶发错位
> （曾把 tier_importance doc 误判为磁盘损坏）——后经逐码点与整句包含验证证实磁盘无损。
> 结论必须以「符号体原始字节 + 编译器/测试输出」为准，显示层异常不做发现依据。

---

## 6. 结论

- 后端架构（一局一线程、决策三件套、LRU 容量治理、存档版本化、确定性 RNG）
  与 `docs/RESEARCH-ARCHITECTURE.md` 调研的业界模式（OpenTTD 命令流、bevy_ggrs 快照、
  axum chat 广播）逐项同构，实现质量高于「原型」定位。
- 门禁四条全绿；无阻塞缺陷。
- 建议下一步：发现 1（importance 语义区分）与三个观察项排入下一轮重构任务书，
  不阻塞当前迭代；观察项 O-2 的「pending 清理权」测试化是其中性价比最高的一项。
