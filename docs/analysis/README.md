# CSC 架构分析文档索引（docs/analysis）

> **2026-09-11 16:30 实施交付**：[整体计划](../DEV-文字生涯-整体纠偏计划.md)、[验收矩阵](../DEV-文字生涯-验收矩阵.md)、[任务台账](../DEV-文字生涯-TASK台账.md)、[验收与运行](../DEV-文字生涯-验收报告.md)、[叙事修复](../DEV-叙事正确性-2026-09-11.md)、[前端页面计划](../DEV-文字生涯-前端页面计划.md)。已新增 `web/` 和真实序章API，GameState v11；世界故事推进仍未完成。下列审查入口是实施前快照，不应机械重复修已闭环项。

> **2026-09-11 当前执行入口**：[25 · DeepSeek 偏移复审](25-DeepSeek偏移复审-2026-09-11.md) → [26 · 严格 TASK](26-文字生涯纠偏TASK-2026-09-11.md) → [TASK 驱动 prompt](../PROMPT-DeepSeek-TASK驱动纠偏.md)。当前代码为 v10 草稿，恢复语义尚未验收；633 tests passed / 3 ignored，fmt 与 Clippy 未通过。旧 M1 完成标记不作为验收依据。下列 09-10 入口和版本描述保留为历史记录。

> **2026-09-10 入口**：文字生涯重构请先读 [22 · 当前工作树审查](22-生涯叙事架构审查-2026-09-10.md)、[23 · 后端与玩法实施方案](23-文字生涯实施方案-2026-09-10.md)、[24 · 前端视觉与交互规范](24-前端视觉与交互规范-2026-09-10.md)，再使用 [DeepSeek harness prompt](../PROMPT-DeepSeek-文字生涯重构.md)。下方 2026-08 的前端结构/版本图为历史快照；当前前端源码已移除，存档为 v9，已确认的文档漂移详见 22。

> 本目录是 **cs-career-simulation**（CS2 职业选手生涯模拟器）的**快速调取式架构分析文档集**。
> 每个文件对应一个子系统，以 **API 参考（Struct / Enum / Trait / Function / Interface）** 形式组织，
> 供开发、审查、AI 编码助手快速检索「哪个类/函数/接口做什么、怎么调」。
>
> 生成时间：2026-08-23 · 依据：当前工作树代码（git HEAD `a00ec15`）· 覆盖后端 17 crates + 前端 + 脚本

---

## 一、快速入口（按需求跳转）

| 我想做什么 | 去这里 |
|---|---|
| **当前 AI 修改是否偏移、证据与最新门禁** | [25-DeepSeek偏移复审-2026-09-11.md](25-DeepSeek偏移复审-2026-09-11.md) |
| **先整体规划、按依赖逐项实施的 TASK** | [26-文字生涯纠偏TASK-2026-09-11.md](26-文字生涯纠偏TASK-2026-09-11.md) |
| **当前应复制到 harness 的纠偏 prompt** | [PROMPT-DeepSeek-TASK驱动纠偏.md](../PROMPT-DeepSeek-TASK驱动纠偏.md) |
| **当前工作树的生涯叙事代码审查与验证（2026-09-10）** | [22-生涯叙事架构审查-2026-09-10.md](22-生涯叙事架构审查-2026-09-10.md) |
| **可暂停执行、剧情弧、API 与重构顺序** | [23-文字生涯实施方案-2026-09-10.md](23-文字生涯实施方案-2026-09-10.md) |
| **Sondaven / HLTV 参考、视觉 token、六页与交互验收** | [24-前端视觉与交互规范-2026-09-10.md](24-前端视觉与交互规范-2026-09-10.md) |
| **交给 DeepSeek 完成代码与本地运行** | [PROMPT-DeepSeek-文字生涯重构.md](../PROMPT-DeepSeek-文字生涯重构.md) |
| 了解整体分层与依赖方向 | [01-总览与架构分层](01-总览与架构分层.md) |
| 后端：RNG / ID / 错误 / 时钟 / 值对象 | [02-基础层-csc-util-time-domain.md](02-基础层-csc-util-time-domain.md) |
| 后端：实体 arena / 属性 / 角色 / 队伍 | [03-实体层-csc-entities.md](03-实体层-csc-entities.md) |
| 后端：纯规则层（胜率/成长/训练/比赛/经济/财务） | [04-规则层-csc-simulation.md](04-规则层-csc-simulation.md) |
| 后端：事件流 / 决策流 / 生涯档案 | [05-事件与决策-csc-events-decision-career.md](05-事件与决策-csc-events-decision-career.md) |
| 后端：VRS 排名系统 | [06-排名系统-csc-vrs.md](06-排名系统-csc-vrs.md) |
| 后端：五大系统引擎（转会/人口/伤病/化学/经济/人生） | [07-系统引擎-csc-systems.md](07-系统引擎-csc-systems.md) |
| 后端：赛事系统（排程/赛制/结算/TOP20） | [08-赛事系统-csc-tournaments.md](08-赛事系统-csc-tournaments.md) |
| 后端：总引擎 / GameState / 查询层 / 客户端视图 | [09-核心引擎-csc-core.md](09-核心引擎-csc-core.md) |
| 后端：REST/WS 服务端 + CLI + WASM | [10-服务端与运行时-csc-server-app-wasm.md](10-服务端与运行时-csc-server-app-wasm.md) |
| 前端：API 客户端 / 类型契约 / WebSocket / 状态管理 | [11-前端-api与store.md](11-前端-api与store.md) |
| 前端：页面与组件结构 | [12-前端-页面与组件.md](12-前端-页面与组件.md) |
| 脚本 / 数据资产 / 工具链 | [13-脚本与数据资产.md](13-脚本与数据资产.md) |
| 架构审查结论（亮点 / 风险 / 建议） | [14-架构审查结论.md](14-架构审查结论.md) |
| **架构整理报告（冗余诊断 + 重构路线图，2026-09）** | **[16-架构整理报告-2026-09.md](16-架构整理报告-2026-09.md)** |
| **后端代码审查（2026-09-06：代码盘点 + 2 P0 / 3 P1 / 6 P2）** | **[19-后端代码审查-2026-09-06.md](19-后端代码审查-2026-09-06.md)** |
| **Sprint 2 巨文件拆分可执行方案（可派单）** | **[DEV-refactor-sprint2-plan.md](../DEV-refactor-sprint2-plan.md)** |

---

## 二、系统全景

```
┌────────────────────────────────────────────────────────────────────┐
│  前端 TypeScript + Solid（纯表现层）                                 │
│  src/api（client.ts / ws.ts / types.ts）→ REST + WS               │
│  src/store/game.tsx（全局状态机） → src/pages（15 页）             │
└───────────────┬────────────────────────────────────────────────────┘
                │ REST（axum）/ WS
┌───────────────▼────────────────────────────────────────────────────┐
│  csc-server（每局一线程 + 租约 + 存档 gzip + WS 心跳）              │
│  csc-app（CLI stdin 决策源） · csc-wasm（浏览器单机）                │
└───────────────┬────────────────────────────────────────────────────┘
                │ Engine（csc-core：装配 / 推进 / 存档 / 查询）
┌───────────────▼────────────────────────────────────────────────────┐
│  第 6 层  csc-core   总引擎 + GameState 聚合根 + WorldQuery         │
│  第 5 层  csc-tournaments  赛事（排程/赛制/结算/TOP20）             │
│  第 4 层  csc-systems  转会/人口/伤病/化学/经济/人生 五+引擎         │
│  第 3 层  csc-decision 决策点 + DecisionSource + 决策日志           │
│  第 2 层  csc-simulation 纯函数规则  ·  csc-vrs 排名  ·  csc-career │
│  第 1 层  csc-entities 实体 arena  ·  csc-events 事件流            │
│  第 0 层  csc-domain 值对象  ·  csc-util（RNG/ID/错误）· csc-time   │
└────────────────────────────────────────────────────────────────────┘
```

**铁律**：确定性内核（同种子 + 同决策序列 = 同世界）；引擎是编排壳、公式在规则层；csc-core 只接线。
分层由 cargo 依赖图编译期强制（无环）。

---

## 三、API 速查表（最常用调用，按子系统）

### 后端

| 需求 | 调用 | 文档 |
|---|---|---|
| 掷随机数 | `Xoshiro256StarStar::seed(s).next_u64()` / `roll_bp(p)` / `next_i32_bound(n)` | 02 |
| 概率判定 | `rng.roll_bp(p)`（整数化，布尔判定唯一入口） | 02 |
| 查选手/队伍 | `world.player(id)` / `world.team(id)` / `world.roster(team_id)` | 03 |
| 算胜率 | `WinRateCalculator::win_rate(a, b, tier, rng, d1, d2, s1, s2)` | 04 |
| 算 Rating | `RatingCalculator::rating_of_counts(k, d, a, rounds)` | 04 |
| 记事件 | `journal.record(WorldEvent::MatchPlayed{..})` | 05 |
| 生成决策 | `DecisionSource::decide(&mut self, points)`（实现 Auto/Human/Stdin） | 05 |
| 查排名 | `engine.query().rankings()` / `engine.vrs().ranking_entries()` | 06/09 |
| 排定训练 | `engine.plan_training(player_id, focus)` | 09 |
| 推进一月 | `engine.advance_month(&mut decision_source)` | 09 |
| 存档 | `engine.snapshot()` / `GameState::from_json_str(s)` / `migrate(s)` | 09 |
| 轻量视图 | `engine.client_state()` | 09 |

### 服务端 REST

| 需求 | 端点 | 文档 |
|---|---|---|
| 建局 | `POST /games {seed, policy, player_name, role}` | 10 |
| 高频刷新 | `GET /games/{id}/view` | 10 |
| 推进 | `POST /games/{id}/advance {months}` / `advance-season` | 10 |
| 决策 | `GET /decisions/pending` + `POST /decisions {decisions, batch_id}` | 10 |
| 事件流 | `GET /journal?since=&channel=&limit=` | 10 |
| 存档 | `GET /save/gzip` / `POST /load/gzip` | 10 |
| 训练 | `GET /training/options` + `POST /training {focus}` | 10 |

> 🔗 **前端对接唯一入口**：完整 47 端点 + WS 协议 + DTO + 存档契约见
> [`docs/FRONTEND-API.md`](../FRONTEND-API.md)（后端协议层权威，随 router() 同步维护）。

### 前端

| 需求 | 调用 | 文档 |
|---|---|---|
| 请求后端 | `api.view(gameId)` / `api.advance(id, months)` / `api.journal(id, since, channel)` | 11 |
| 订阅推送 | `new GameSocket().connect(gameId, handlers)` | 11 |
| 全局状态 | `useGame()` → `store` + 方法（advance/submitDecisions/...） | 11 |
| 事件判别 | `eventKind(event)` / `decisionKind(point)` | 11 |
| 默认决策 | `defaultDecision(point)`（与后端 AutoDecisionSource 对齐） | 11 |

---

## 四、各文档内容一览

| 文档 | 核心内容 |
|---|---|
| 01 | 分层图、crate 职责表、数据流（REST/WS/存档）、目录结构 |
| 02 | `Xoshiro256StarStar` 全部方法、`PlayerId/TeamId`、`SimError`、`SimClock/TimeWindow`、domain 值对象 |
| 03 | `World` arena 原子操作、`PlayerCharacter/Team` 字段、属性生成器、`PowerCalculator` |
| 04 | `WinRateCalculator`、`RatingCalculator`、`GrowthModel`、`TrainingModel`、`MatchSimulator`、`LiveMatchEngine`、`FinanceModel` |
| 05 | `WorldEvent` 14 变体、`WorldJournal`、`DecisionPoint` 7 变体、`DecisionSource` trait、`CareerArchive` |
| 06 | `VrsDatabase`、`VrsEngine`、`VrsEntry`、Seed+ELO 两层模型、modifiers |
| 07 | `TransferEngine`、`PopulationEngine`、`InjuryEngine`、`ChemistryEngine`、`EconomyEngine`、`LifeEventsEngine` |
| 08 | `TournamentEngine`、`TournamentScheduler`、赛制（Swiss/双败/单败）、`SeriesSettlement`、`Top20Evaluator` |
| 09 | `Engine` 门面、`GameState` v9 存档契约、`SeasonDirector` 阶段管线、`WorldQuery`、`ClientState` |
| 10 | 全部 REST 端点签名、WS 协议、`AppState`/租约/容量模型、CLI、WASM |
| 11 | `api` 对象全部方法、`types.ts` 契约（1332 行）、`GameSocket`、`GameContextValue` |
| 12 | 15 个页面职责、组件树、i18n、lib 工具 |
| 13 | scripts/*.mjs 30+ 脚本、assets 资产 schema、CI |
| 14 | 架构审查结论：设计亮点、遗留风险、优化建议（含既有 CODE-REVIEW 系列结论整合） |
| 15 | 第二轮多智能体交叉审查：62 项修正清单（20 P0 硬错误 + 17 P1 遗漏 + 8 疑点 + 7 时效性）、矛盾裁决、可信度 5.5/10 |

---

## 五、既有评审文档（docs/ 根目录，按需参考）

- `docs/CODE-REVIEW-audit.md` — 玩法全貌 / 完成度 / 可达性探测（历史结论；TOP20 外卡等条目已在工作树闭环，见 16 号报告）
- `docs/CODE-REVIEW-2026-08-17.md` — 全仓代码审查
- `docs/CODE-REVIEW-BLOAT-2026-08-18.md` — 代码臃肿度审查
- `docs/CODE-REVIEW-COMPARATIVE-2026.md` / `docs/REVIEW-COMPARATIVE-FINAL.md` — 对比式审查（成熟项目最佳实践）
- `docs/FINAL-REVIEW-ENGINEERING.md` — 总终审（585 测试 / 指纹门禁 / 遗留项）
- `docs/SAVE-FORMAT.md` — 存档格式版本契约（v1–v9）
- `docs/FRONTEND-API.md` — 前端对接文档（47 REST + WS / DTO / 存档 v9 / LIVE / 决策流）
- `docs/DECISION-POLICY.md` — 决策分级与自动策略
- `docs/CAPACITY-MODEL.md` — 多局容量模型
- `docs/P2-PLAN.md` — P2 遗留项实施方案
