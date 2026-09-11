# CSC 产品化路线图（PM 口径 → 可验收工程项）

> 输入：技术架构报告 + 产品经理视角优化建议。
> 原则：先证明「第 1/5/10/20 年仍然有趣且可理解」，再谈扩大世界规模与压缩格式。
> 验收：每个迭代 = 固定种子回归 + 存档兼容 + 漏斗数据看板 + 一次真实玩家试玩。

## 0. 产品北极星

**「玩家能理解自己的职业为什么走到这里，并愿意再开一局。」**

支撑指标：
- 激活：完成建档并推进到首场正式比赛 ≥80%。
- 首局体验：建档 → 第一次有意义决策 ≤3 分钟。
- 决策质量：非默认选项选择率 ≥35%。
- 生涯深度：第 2/5/10 赛季到达率建立基线并逐版本提升。
- 重玩价值：完结后 7 天内重新开档率 ≥15%。
- 性能：月推进/存档/读档 P95 按档龄（1/5/10/20 年）分桶监控。
- 稳定性：存档失败率、推进失败率 <0.1%。

## 1. 指标 → 埋点事件（最低集）

| 事件 | 必须字段 | 分析用途 |
|---|---|---|
| onboarding_complete | role, seed_used | 激活漏斗 |
| first_match_reached | sim_date, team_tier | 首局体验 |
| decision_shown | batch_id, kinds, tier, auto_policy, auto_style | 决策曝光 |
| decision_chosen | point_id, option_id, source=player|auto, auto_policy, auto_style | 选择质量 |
| match_finished | event, result, rating, kd | 比赛反馈 |
| transfer_offer_seen / accepted / rejected | team_rank, salary_delta | 转会决策价值 |
| save_downloaded / save_loaded | bytes, compressed, duration_ms | 存档可信度 |
| season_summary_viewed | season_index | 反馈闭环 |
| career_ended | years, cause | 生涯完结漏斗 |
| new_game_after_end | days_since_end | 重玩价值 |

关键规则：**必须记录「选项被看到但未选择」**，区分无吸引力、文案不清、自动提交过快。

## 2. 迭代路线图

### Sprint 1（2 周）：建立长生涯质量基线 + 可靠性加固
- [x] 128 队 20 年 release soak 基线：16.6s、P95 82.7ms、快照 176MB；见 `docs/GOLDEN-CAREER-BASELINE.md`。
- [x] 10 个固定种子黄金生涯集 v1（可扩展到 20–50）；产出 `scripts/data/golden_careers.json`。
- [x] gzip 保存/读档与解析移入 `spawn_blocking`。
- [x] 单局保存互斥、解压后体积上限、压缩比/版本元数据、保存错误语义。
- [x] 产品埋点最小集（本地缓冲 + 可替换的上报适配器）。
- [x] journal 游标按 `gameId + save fingerprint` 持久化，防读档/回滚串档。

验收：
- 20 年档保存期间，其他游戏的 `/view` 与推进 P95 不显著恶化。
- 固定种子跨版本重算指纹一致；1/5/10/20 年档均可读回。
- 埋点事件包含 player/manual/auto/timeout 来源且能区分「未选择」。

### Sprint 2（2 周）：运营约束与多局治理
- [x] 存档 CRC32 校验和（响应头 + 文件名校验）。
- [x] 保存/读档 300s 超时、快照取用 120s 超时与明确错误提示。
- [x] GameManager 生命周期 v2：LRU 淘汰、磁盘持久化、按需恢复、恢复去重、请求租约。
- [x] 活跃游戏数上限（默认 4）与空闲局 gzip 落盘；容量压测 10×20 年并发全绿。
- [x] 容量模型 v1：6 局/活跃 4 的 LRU 单测 + `scripts/capacity_stress.mjs` 10 局 20 年实测，
      结果见 `docs/CAPACITY-MODEL.md` 与 `scripts/data/capacity_stress_10x20y.json`。
- [x] 断连/取消不泄漏保存互斥（`SaveGuard` RAII）。

验收：
- 超过上限时按 LRU 卸载而非删除；被卸载局可无损恢复。
- 并发长档保存不互相阻塞；实测 10 个 20 年档并发保存 P95 58.3s、读档 P95 2.7s、
  `/view` P95 5ms，0 错误。
- 保存/读档超时与明确错误提示已上线。

### Sprint 3–4（4 周）：核心循环产品化
- [x] 赛季目标闭环：设置 → 训练调制 → 跨年清空 → 赛季档案写入 `goal_met / goal_outcome` → 赛季总结弹层可解释结算。
- [x] 决策分级 v1：P0 必确认 / P2 可自动 / minor-only 默认；见 `docs/DECISION-POLICY.md`。
- [x] 「自动处理同类决策」按类型记忆（localStorage 白名单，与分级策略正交）。
- [x] 自动策略规则：均衡 / 保守 / 激进三种代选风格（`lib/decide.ts::styleDecision`；
      假赛选项任何风格都不自动接受）。
- [x] 结果解释层 v1：转会洞察 + 最近比赛解读 + 队内地位解释均由后端规则推导（`/games/{id}/insights`）。
- [x] 关键选项显示影响方向、风险等级、生效周期；决策点头部显示 P0/P1/P2。

验收：
- 非默认选项选择率 ≥35%（埋点上线后统计，当前已采集 `option_id` 与 `source`）。
- 每赛季主动高价值决策 20–40 个（黄金基线 43.7 决策/年，结构仍需调参）。
- 赛后/转会原因详情查看率 ≥40%（`insights` 已上线，待试玩数据）。
- 自动代选与玩家主动选择在数据上可区分。

### Sprint 5–6（4 周）：长生涯内容治理
- [x] 职业阶段事件池 v2：阶段分池 + 同类型 3 个月冷却（`life_event_last_kind/month` 持久化）。
- [x] soak 自动检测重复事件率：`life_consecutive_repeats_mean = 0.6`（10 seeds×20 年，v6 基线）。
- [ ] 生涯结局总结增强：关键转折点、代价/收益回顾（下一迭代）。
- [x] 前端信息架构按任务归组：生涯中心 / 竞技 / 俱乐部 / 世界 / 档案。

验收：
- 第 5 与第 10 赛季事件池重合率下降至目标线（基线先行）。
- 完结后 7 天内重新开档率 ≥15%。
- 首页回答「我现在该做什么」与「最近发生了什么」。

### 后续（按数据决策）
- 数据驱动赛历（免改代码更新赛季）。
- CSS 单一 token 体系：**已完成主文件暗色层清理**（仅保留文件末尾 HLTV 浅色统一层）；
  视觉回归基线（截图对照/像素级 diff）待下一迭代。
- zstd / checkpoint + journal 增量存档实验。

## 3. 双轨治理

- **可靠性轨**：20 年 soak、异步压缩、内存上限、存档安全、版本兼容、并发恢复去重。
- **产品深度轨**：赛季目标、决策代价、结果解释、阶段化事件、中后期差异。

只有玩家在第 1、5、10、20 年面对不同且可理解的职业问题时，扩大世界规模、
增加赛事和压缩格式优化才具有产品价值。

## 4. 本轮完成记录（Sprint 1–6 主体收官）

- 三条门禁全绿：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `npm run build`。
- 黄金生涯集 v6 重跑 10 seeds×20 年（16.6s、P95 82.7ms、快照 176MB、同类型事件连续重复 0.6）。
- 10×20 年并发容量压测通过（保存 P95 58.3s / 读档 P95 2.7s / view P95 5ms / CRC 10/10 / 0 错误）。
- 决策弹窗补齐影响方向、风险等级、生效周期；自动代选风格三选一并持久化。
