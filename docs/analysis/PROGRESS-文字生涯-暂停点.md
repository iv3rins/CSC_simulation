# 文字生涯重构 · 进度记录（暂停于 2026-09-10 夜）

> **2026-09-11 16:28 已实施增量**：请从 [当前TASK台账](../DEV-文字生涯-TASK台账.md) 和 [本轮验收报告](../DEV-文字生涯-验收报告.md) 接续。当前GameState v11、web已存在，649项测试通过。完整世界/赛事暂停仍未完成，下方旧“恢复重放”计划继续禁止使用。

> **2026-09-11 复审覆盖说明**：下文是历史暂停记录。M1 的整体完成判断被 [25 号复审](25-DeepSeek偏移复审-2026-09-11.md) 撤销；M2 的无限等待/恢复重放路线需要重新设计。当前资产实际为 13 场景，最新全量测试 633 passed / 3 ignored，但 fmt/Clippy 未通过。请从 [26 号 TASK](26-文字生涯纠偏TASK-2026-09-11.md) 的 T00 开始，不直接接续下文“次日待做”。

> 依据 `docs/PROMPT-DeepSeek-文字生涯重构.md`（M0→M5）实施。本文记录暂停点，
> 便于次日接续。**当前工作树未提交，勿 reset/clean。**

## 已完成

### M0 基线复核 + 文档漂移修正 ✅
- 实测门禁（`backend/`，离线）：`cargo test --workspace --locked --offline`
  = **604 passed / 0 failed / 3 ignored**；clippy / fmt / wasm 全部 exit 0（与 22 号一致）。
- 已修文档漂移：
  - `docs/analysis/09`：v8 → v9（`CURRENT_VERSION`、版本演进表）。
  - `docs/analysis/05`：`ProjectionContext { protagonist_id, protagonist_team_id }`、
    `JournalProjection { tier, channel, visible }`。
  - `docs/analysis/07`：`InjuryEngine::monthly_tick` 实际签名（5 参）。
  - `docs/analysis/README.md`：09 行 v8 → v9。
  - `docs/FRONTEND-API.md`：`/advance` auto → `{summaries:[...]}`；
    `/decisions/pending` → `{pending:...}` 包装。
  - `docs/DECISION-POLICY.md`：顶部加「历史前端已删除」状态标注。
  - `docs/SAVE-FORMAT.md` §5.2：修正 VRS 兜底（已恢复，非「不再生成」）。
  - `AGENTS.md`：wasm 目标 `wasm32-unknown-unknown`；测试数 604+。

### M1 可靠决策提交 ✅（已通过 csc-server 全部测试）
- 新增 `csc-decision/src/validation.rs`：类型化全集校验器
  `validate_submission` / `candidate_option_ids` / `ValidationError`（含 8 个单测）。
- `csc-core/src/batch.rs`：`WorldDecisionBatch::run` **先校验，再 record + apply**（R2）。
- `csc-server/src/game/types.rs`：新增 `DecisionSubmission` 信封（batch_id/request_id/reply）、
  `submission_fingerprint`、`ReceiptCache`（有界幂等缓存）。
- `csc-server/src/game/entry.rs`：`submit_decisions(request_id, decisions, batch_id)`——
  预检 + 投递信封 + 等权威回执；同 ID 同载荷幂等，异载荷拒绝。
- `csc-server/src/game/decision.rs`：`ChannelDecisionSource` 接收信封，在**状态拥有者**
  处再校验批次身份 + 全集，通过才消费并回执（R1）；新增 2 个 R1 回归测试。
- `csc-server/src/live.rs`：`decide` **先校验后 `take`**（R3），非法选择保留 pending；
  新增 `invalid_live_decision_preserves_pending` 回归测试。
- `csc-server/tests/api.rs`：新增 `request_id_idempotency_same_payload_ok_diff_payload_rejected`。
- 结果：`cargo test -p csc-server` 31+28 全绿。

## 进行中（M2 持久暂停）——**当前工作树可编译（含 tests），但尚未跑全量回归**

### 已落地
- `csc-entities/src/narrative.rs`（新）：`NarrativeProgress` / `Promise` / `PromiseStatus` /
  `ActiveSceneRun`（纯值，含 5 个单测通过）。
- `csc-core/src/state.rs`：`GameState` **bump v10**，新增
  `narrative: NarrativeProgress` + `execution: ExecutionState`（均 serde default，
  旧档补最小进度 / `Idle`）；新增 `ExecutionState` / `StepKind`；
  新增 v9→v10 迁移测试。
- `csc-core/src/narrative/`（新，3 文件）：
  - `content.rs`：内容 schema（`NarrativeContent`/`SceneDef`/`ChoiceDef`/`Trigger`/`Effect`）
    + 加载校验（重复 ID/悬空 next_scene/无出口）；
  - `director.rs`：`Director::select` 纯函数选场景（priority + 稳定 ID 排序）；
  - `apply.rs`：受限 effect handler（承诺/flag/关系/凝聚力/士气/声誉/终态/转会意图）；
  - `mod.rs`：`NarrativeState` 聚合 + 多资产合并。
- `csc-core/src/engine.rs`：Engine 持有 `narrative`/`execution`；新增
  `narrative()/narrative_mut()/execution()/execution_mut()/set_narrative_content`、
  `advance_unit`、`narrative_facts`、`begin_scene`、`apply_story_choice`；
  `snapshot`/`restore_result` 纳入新字段并**保留装配配置（text/rating_profile）**。
- `assets/narrative/zh-CN/career.json`（新）：3 弧 × 14 场景内容（中文正文/选项/效果）。
- `csc-server`：`Policy` 加 `is_human`；`GameEntry` 加 `story`；
  `PersistedGame` 加 `story`（serde default）；`ChannelDecisionSource` 加
  `decision_timeout: Option<Duration>`（**故事模式 = None = 持续等待**）+ `snapshot` + `story`；
  `stamp_awaiting` 把悬停点写入共享快照（待决策时可保存）。

### 次日待做（M2 收尾）
1. `cargo build --workspace --tests` 最后一次确认（上次仅剩 decision.rs 测试辅助的
   `yearly_rating` 依赖问题，已用 `Default::default()` 修复，待复跑）。
2. `create_with_story` 接线到 `/games` 创建路由（`CreateGameRequest` 加 `story` 字段，
   故事模式默认 `policy=human`）。
3. 新增 `GET /games/{id}/career` + `POST /games/{id}/career/continue` +
   `POST /games/{id}/career/choices` + `operations/{id}` + `history` 路由/DTO。
4. 故事驱动：`continue` 推进一步 → 检查 `NarrativeDirector` 停点 → 入 `active_scene`；
   场步循环（`AwaitingDecision(0, [], replay_current=false)`）。
5. 比赛/月度决策批次暂停时把 `execution` 落盘 + 恢复重放（回灌 armed decisions）。
6. 跑全量回归 + 补 M2 针对性测试。
7. 之后 M3 内容/API 收尾 → M4 `web/` 前端 → M5 联调/截图/验收报告。

## 已知风险 / 注意
- `GameState` 已到 v10：`docs/SAVE-FORMAT.md` **尚需补 v10 行**（下次务必同步）。
- `docs/analysis/10` / `FRONTEND-API` 的 `/career` 契约尚未写（待 M3/M4）。
- 前端 `web/` 尚未创建。
- 未跑 `csc-app`/wasm 与全量 workspace 测试的**最后一次**验证（M1 时通过）。

## 关键命令
```sh
cd backend
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
cargo build -p csc-wasm --target wasm32-unknown-unknown --locked --offline
```
