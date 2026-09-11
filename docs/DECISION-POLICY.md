# 决策分级与自动策略（Sprint 3–4 实现稿）

> **状态标注（2026-09-10，依据 22 号审查 §4 裁决）**：本文 §2–§6 描述的分级 UI、
> 代选风格（`lib/decide.ts`）、localStorage 白名单与埋点（`store.game.tsx`）**均属
> 已删除的历史前端**实现。当前后端只有 `Auto` / `Human` 两种策略（`csc-server` `Policy`），
> 本文的 TS 侧算法（`styleDecision` / `csc.auto_handled_kinds`）不构成后端契约。
> 新的故事生涯模式默认 `human` 且关键选择不自动代选（见 23 号方案 §4.2）。
> 本节内容保留为历史与待迁移参考，**不是当前已上线能力**。

## 1. 问题

早期 `store.game.tsx` 在自动模拟开启时对**所有决策批次**统一 500ms 自动提交默认项。
工程上保证不卡死，产品上会把决策系统降级为通知，且数据无法区分「玩家主动选择」
与「系统代选」。

## 2. 已上线能力

- 关键职业决策按分级暂停自动推进（`off` / `minor-only` / `all`）。
- 「本类后续自动处理」按决策类型记忆（localStorage `csc.auto_handled_kinds`），
  与分级策略正交：玩家显式放行的类型不再暂停。
- 自动代选风格：`balanced`（默认，与后端 `AutoDecisionSource` 完全一致）/
  `conservative`（留队、休养、婉拒、维护关系）/ `aggressive`（转更强队、带伤、
  接代言）。**假赛选项（MATCH_FIXER_CONTACT 的 ACCEPT）任何风格都不会自动选择。**
- 埋点区分 `source = player | auto`（timeout 由服务端决策日志扩展时补录），
  `decision_shown/decision_chosen` 同时携带 `auto_policy` 与 `auto_style`。
- 决策弹窗对每个关键选项展示：影响方向（impact）、风险等级（高/中/低）、
  生效周期（本图/本月/合同期/至恢复等），决策点头部展示 P0/P1/P2。

## 3. 决策分级

| 等级 | 决策类型 | 默认行为 | 说明 |
|---|---|---|---|
| P0 关键 | TRANSFER_WINDOW | 暂停并确认 | 合同/转会改变生涯归属 |
| P0 关键 | LIFE_EVENT（MATCH_FIXER_CONTACT） | 暂停并确认 | 声誉/生涯风险 |
| P0 关键 | INJURY_DECISION（SEVERE） | 暂停并确认 | 长周期状态风险 |
| P1 重要 | MATCH_INTERVENTION | 暂停或按策略 | 关键比赛临场取舍 |
| P1 重要 | SPONSORSHIP_OFFER | 暂停或按策略 | 收入 vs 状态/约束 |
| P1 重要 | LIFE_EVENT（私下/正式战队接触、队友宫斗） | 暂停或按策略 | 关系与未来选项 |
| P2 日常 | TEAMMATE_BLUNDER | 按策略自动 | 关系微调 |
| P2 日常 | INTERVIEW / TEAMMATE_INVITE 等低影响生活事件 | 按策略自动 | 叙事噪音 |
| P2 日常 | TRAINING_FOCUS | 按策略自动 | 月度养成 |

## 4. 自动策略配置（ControlPage 设置页）

- `off`：所有决策暂停等待玩家。
- `minor-only`：仅 P2 自动；P0/P1 暂停（除非该类型已被玩家加入自动白名单）。
- `all`：全部自动（挂机/长程测试），按下方风格选择选项。

推荐默认：`minor-only` + `balanced`。

## 5. 代选风格语义（`lib/decide.ts::styleDecision`）

| 决策 | balanced | conservative | aggressive |
|---|---|---|---|
| 转会窗 | 只签明显更强队，否则留队（=后端 auto） | 留队/续约 | 入队实力提升最大的候选 |
| 场内干预 | 当前风格基线 | CONSERVATIVE | AGGRESSIVE |
| 伤病 | 默认（历史行为：带伤） | REST 休养 | PLAY_THROUGH 带伤 |
| 代言 | ACCEPT | DECLINE | ACCEPT |
| 队友失误 | IGNORE | SUPPORT 鼓励 | CONFRONT 指责 |
| 假赛联系 | REFUSE | REFUSE | REFUSE（安全红线） |
| 私下接触 | 默认 | DECLINE | MEET |
| 队友宫斗 | MEDIATE | MEDIATE | BACK_ACTOR |
| 采访/邀请等 | 默认 | 维护关系选项 | 曝光/收益选项 |

找不到目标选项时一律回退 `defaultDecision`，保证提交协议永不卡死。

## 6. 验收口径

- 自动模拟 12 个月：P0 批次全部被暂停，P2 批次按策略自动，推进不永久卡死。
- 分级策略、代选风格、类型白名单均持久化在 localStorage；存档/读档不丢失。
- 埋点可回答：每个 P0 决策「看到后多久提交、选了哪个、是否系统代选、什么风格」。
- 非默认选项选择率 ≥35% 的基线在埋点上线后开始统计。
