# PROMPT · 模块化架构重写（拆分→实跑→修 BUG 循环）

> **用法**：把本文件全文作为任务书，粘贴给执行智能体（新 Codex 线程的第一条消息 / AgentTeams 写码队员 / 任何 AI 编码助手）。执行者不需要任何前置对话历史——本文件自包含。
> 版本：2026-09-05 · 仓库：D:\repo-by-iverins\cs-career-simulation

---

## 0. 你的任务（一句话）

把本仓库 **生产代码超过 500 行的文件** 全部拆分为「职责单一、每文件 ≤500 行生产代码」的模块；以 **拆分 → 实跑 → 出 BUG → 修复 → 再拆** 的循环推进，每一步都用门禁验证，直到所有目标文件达标且全部门禁绿灯。

## 1. 背景与动机（先读懂为什么这么做）

- **500 行是红线**：没有任何人能连续阅读 500 行代码后保持高度注意力。超过即屎山，除非迫不得已。
- **模块分离**：每个文件只有一个职责，一个类/模块只做一件事；职责混杂是 BUG 温床。
- **人类优化架构的方式是重写式循环**，不是一次性大重构：
  `拆分 → 实际跑一段 → 出 BUG → 再重构 → 再跑一段 → 出 BUG → 修复`。
  接受这个循环，不要试图"一次改完美"；每次只动一个文件，跑起来，坏了就修，绿了再下一个。
- **为什么要小步**：AI 上下文窗口存在注意力涣散；文件越小、循环越短，错误越少、越好修。
- 本仓库已按层分好 crate（cargo 依赖图无环），**分层大方向不动**；你要做的是**层内文件拆分与职责收拢**。

## 2. 开工前必读文档（按顺序，Docs-First 铁律）

| 顺序 | 文档 | 作用 |
|---|---|---|
| 1 | `docs/analysis/17-底层探针与代码地图.md` | 读取协议 + 代码地图索引 + 人类式维护五原则 |
| 2 | `docs/analysis/18-文件拆分作战清单.md` | **拆分目标清单**（文件/生产行数/拆法/优先级） |
| 3 | `docs/DEV-refactor-sprint2-plan.md` | T1/T2/T3 现成可执行方案（routes/game/client） |
| 4 | `docs/analysis/17-map-*.md`（已有 3 份） | 已读文件的地图：settlement×2、tournaments-engine |
| 5 | `docs/analysis/16-架构整理报告-2026-09.md` | 冗余诊断背景（注意：其中发现 2/3 已被 17-map 修正，以 map 为准） |
| 6 | `docs/analysis/` 对应层文档 | 你正在拆的模块的 API 契约（01-14 号） |
| 7 | `AGENTS.md` §0.2 §4.3 §10 | 文档定位表 / 任务要素 / 单智能体纪律 |

**读法纪律**（对抗注意力涣散）：
- 引用方法**先看注释**（doc 签名+作用），注释不够懂才下沉读实现；
- 不 grep 预测式下结论；要判定"是否重复"必须完整读两边代码；
- 每读完一个文件，若有新认知，先补进 `17-map-*.md` 再动手。

## 3. 拆分目标清单（按优先级，一次一个）

行数口径：**生产代码行数** = 文件总行数 − 内嵌 `#[cfg(test)]` 测试区（测试随模块搬或留原文件，不算入红线）。

| # | 文件 | 生产行 | 拆法 | 方案来源 |
|---|---|---|---|---|
| 1 | `backend/crates/csc-server/src/routes/mod.rs` | 1760 | lifecycle / query / advance / decision / transfer / live 六个子模块 | DEV-refactor-sprint2 T1 |
| 2 | `backend/crates/csc-server/src/game/mod.rs` | 1596 | types / decision / entry / manager / persist | DEV-refactor-sprint2 T2 |
| 3 | `backend/crates/csc-core/src/engine.rs` | 1091 | 总引擎门面 / 推进 / 赛季 拆分 | 自拟（读后定） |
| 4 | `backend/crates/csc-tournaments/src/engine.rs` | 1081 | 编排 / run_bracket / 回填辅助 / legacy_single_elim | 17-map-csc-tournaments-engine |
| 5 | `backend/crates/csc-core/src/client.rs` | 1031 | state / calendar / live_gate / event_aggregate | DEV-refactor-sprint2 T3 |
| 6 | `backend/crates/csc-tournaments/src/conductor.rs` | 908 | 决策循环 / LOD 系列 / 玩家系列 | 自拟 |
| 7 | `backend/crates/csc-simulation/src/live_match.rs` | 846 | LIVE 引擎 / 事件 / 结算 | 自拟 |
| 8 | `backend/crates/csc-simulation/src/replay.rs` | 728 | 回放状态 / 推进 / 序列化 | 自拟 |
| 9 | `backend/crates/csc-tournaments/src/settlement.rs` | 653 | 系列结算 / 荣誉 / 奖金 / 年度 | 17-map-csc-tournaments-settlement |
| 10 | `backend/crates/csc-tournaments/src/calendar.rs` | 583 | 日历表 / 档期 | 自拟 |
| 11 | `backend/crates/csc-core/src/settlement.rs` | 563 | 年度编排 / 赛季目标 / 记忆 | 17-map-csc-core-settlement |
| 12 | `backend/crates/csc-systems/src/life_events.rs` | 559 | 收集 / 各事件处理 | 自拟 |
| 13 | `backend/crates/csc-events/src/event.rs` | 513 | WorldEvent 类型拆分 | 自拟 |
| 14 | `backend/crates/csc-vrs/src/database.rs` | 506 | VRS 库 / 积分 | 自拟 |
| 15 | `backend/crates/csc-simulation/src/match_simulator.rs` | 505 | 精确 / 快速路径 | 自拟 |
| 16 | `backend/crates/csc-tournaments/src/scheduled.rs` | 502 | 排程记录 / 物化 | 自拟 |
| 17 | `frontend/src/store/game.tsx` | 1459 | 按职责拆多 store | DEV-refactor-sprint2 S3.4 |
| 18 | `frontend/src/api/types.ts` | 1382 | 按领域拆类型文件 | 自拟 |
| 19 | `frontend/src/pages/MatchPage.tsx` | 512 | 拆子组件 | 自拟 |

**已完成（不用拆）**：top20.rs / economy.rs / nan.rs / scheduler.rs / training.rs / yearly_rating.rs / population.rs / generator.rs / loop_.rs / narrator.rs（生产代码已 <500）。

## 4. 工作方法（每轮循环 = 处理一个文件）

```
Step 0  开工基线：git status 确认工作区状态；跑一次全量门禁（§5），
        记录基线。若基线红 → 先修基线，不拆。
Step 1  选一个文件（按 §3 优先级，从 1 开始）
Step 2  读：先查 docs/analysis/ 对应层文档 + 17-map（若有）→ 再读源码
        → 输出"我的拆分计划"（新文件结构 + 每个函数去哪 + 公共依赖怎么共享）
Step 3  拆：优先"纯物理搬移"（零行为变化）；若某模块结构太烂搬不动，
        允许小范围重写该模块（保持对外契约，见 §6）——这是循环的本来含义
Step 4  验证：跑该 crate 的 check + 测试（§5 局部），红了就修
Step 5  全绿 → 跑全量门禁（§5 全量）
Step 6  记录：更新 docs/analysis/18 该行状态为 ✅ + 写一段"拆分摘要"
        （旧结构→新结构 / 行为变化：无 or 有+原因 / 踩坑）
Step 7  进入下一个文件（回到 Step 1）
```

**循环纪律（最重要）**：
- 一次只动一个文件；禁止"顺手"改范围外代码；
- 出 BUG 先**定位 → 剥离分析 → 修复**，不盲改；同一处修两次不过 → 停下报告（你烧 token 重试不如停下来想）；
- 每拆一个文件至少跑一次 `cargo check -p <crate>`（前端 `tsc --noEmit`），不要攒到最后一起验证；
- 测试随被拆代码走，一条不删；新增拆分需要时补 1 个"结构冒烟"测试（可选）；
- 注释、serde 属性、doc 必须原样跟代码走——它们是 docs 的事实源。

## 5. 验证门禁（每个文件拆完必须全过；全量在每轮 Step 5）

```bash
# 后端（workspace 根 = backend/）
cd backend
cargo test --workspace --locked        # 全绿（当前基线约 596+）
cargo clippy --workspace --all-targets -- -D warnings   # 零警告
cargo fmt --all -- --check             # 格式通过
cargo build -p csc-wasm --target wasm32-unknown-unknown  # wasm 可编译

# 前端（frontend/）
cd ../frontend
npm run typecheck                      # 0 错误
npm test                               # vitest 全过
npm run build                          # 构建成功
```

**判定**：任何一条红 = 本轮未完成，修到绿才能进入下一个文件。禁止用"应该没问题"跳过验证。

## 6. 不可破坏的契约（边界，越界前必须停下来问）

1. **存档契约**：`GameState::CURRENT_VERSION`、字段、`deny_unknown_fields`、迁移链——见 `docs/SAVE-FORMAT.md`，**一个字不许改**；
2. **可复现性**：RNG 消费顺序（同种子+同决策=同世界；指纹测试会抓你）——**拆分不得改变任何 RNG 调用顺序**；
3. **对外 API**：REST 端点/路径/请求响应 JSON、WS 协议、`csc-wasm` 导出——前端全量消费，**签名与语义不变**；
4. **crate 对外 re-export**：`lib.rs` 的 `pub use` 列表不变（只在 crate 内部调整路径）；
5. **分层依赖**：不许让低层 crate 反向依赖高层（cargo 无环是编译期强制的，别试图绕过）；
6. **行为锁定测试**：`csc-core/tests/fingerprint.rs`、`golden` 类测试必须继续绿——它们是"改坏了会告诉你"的哨兵。

**如果某个文件确实需要 breaking change（改 API/格式/行为）才能拆好 → 停下来，在交付物里写明理由与影响面，等批准。不要自作主张。**

## 7. 文档同步义务（Docs-First）

- 拆完一个文件：更新 `docs/analysis/18-文件拆分作战清单.md`（状态列 ✅ + 拆法简述）；
- 若拆分暴露了"文档与代码不符"（如 08 号文档记录的签名与源码不符）→ 在交付物中列出，同步修正对应 docs/analysis 文档；
- 新增的模块若改变了"哪个文件提供哪个 API"的组织 → 更新 docs/analysis/ 对应层文档的定位；
- 你的"拆分摘要"写入 `docs/analysis/17-map-*.md` 或独立小节，不欠账。

## 8. 交付物（本轮全部完成后）

1. 所有目标文件生产代码 ≤500 行（§3 清单全部 ✅）；
2. `docs/analysis/18` 状态全绿 + 每个文件的拆分摘要（旧→新 / 行为变化 / 踩坑）；
3. 全程门禁结果记录（每文件一次，粘贴关键输出尾部）；
4. 遗留说明：哪些文件你建议保持 >500（如有，说明"迫不得已"的理由）；
5. 发现的文档滞后清单（如 settle_series 签名在 08 号文档 vs 源码不符）。

## 9. 你可以引用的仓库内先例

- `DEV-refactor-sprint2-plan.md`：T1/T2/T3 已经把 routes/game/client 的拆法写到"可派单"粒度（文件结构+步骤+验收+坑），直接照做；
- `format/tests_common.rs`、`csc-simulation/src/kills_alloc.rs`：Sprint 1 已完成的"重复→单一事实源"样板（提取共享模块 + 注释记录动机）；
- `17-map-csc-tournaments-engine.md` §4：**重要教训**——`legacy_single_elim` 与 `SingleElimPlayoff` 看似重复实则轮空/赛制/配对来源有意不同，**判定重复前必须完整读两边**，别被 grep 骗。

## 10. 开始前，先输出你的执行计划

读完全文后，先不要写代码。输出：
1. 你对任务的理解（3-5 行）；
2. 你对 §3 清单的疑问（如有）；
3. 你打算先处理哪 3 个文件、顺序与理由；
4. 你预计最大的风险点。

等确认（或自检无异议）后，从 §4 Step 0 开始。
