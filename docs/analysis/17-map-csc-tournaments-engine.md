# 17-map-csc-tournaments-engine.md（底层探针·精准读取）

> 读取方式：完整读取骨架 + 对疑点函数（legacy_single_elim / run_bracket）整段精读 + 与 format/single_elim.rs、swiss_playoff.rs 逐行比对。
> 读取日期：2026-09-05。文件：csc-tournaments/src/engine.rs（2040 行，测试占 1090-2040 约 47%）。

## 1. 文件职责
- 模块文档：赛事子系统 sub-engine（Kotlin TournamentEngine.kt 转写）：排程→邀请→逐场 series 模拟→结算编排。本引擎只做编排，比赛决策全在 conductor，账目公式全在 settlement。
- 一句话：**赛事编排壳**（非规则/非结算）。只持有跨调用状态（scheduler/settlement/events/scheduled_records）。

## 2. 公开面（生产代码 60-1089，测试 1090-2040）
| 方法 | 行号 | 作用 | 备注 |
|---|---|---|---|
| new / default / restore_from | 60-98 | 构造/读档 | |
| scheduled_tournaments / participants_of / results / results_ref / event_count | 99-125 | 查询面 | event_count 高频防 clone |
| settlement / settlement_mut / yearly_rating_snapshot / top20_history / restore_settlement / scheduled_records_snapshot | 126-162 | 结算层访问器 | 封装边界 |
| skip_live_today | 165 | 标记今日 pending 为 skipped | 持久语义不改结算 |
| last_event_teams | 205 | 最近赛事队伍 | |
| mark_fixture_watched | 217 | 标记已看（幂等） | |
| run_series | 245 | 单场模拟便捷入口 | 走 match_simulator |
| run_tournament_detailed | 293 | 赛事详细模拟入口 | **无 doc 注释** |
| simulate_event / plan_event_with_ranking / simulate_event_with_ranking | 339-640 | 排程+模拟家族 | 部分无 doc |
| settle_pending_events / settle_pending_event | 456-579 | 结算 pending | |
| yearly_stats / year_end_awards | 641-667 | 年度结算薄转发 | |
| **run_bracket** | 668 | **分赛制跑完整赛事** | 见 §3 |
| ranking_snapshot | 850 | 月首资格快照（唯一资格来源） | |
| scheduled_importance | 884 | **纯别名**（转发 tier_importance） | doc 明示唯一实现见 tier_importance |
| rec_day / backfill_fixture_results / backfill_fixture_score | 890-989 | 赛程回填辅助 | |
| **legacy_single_elim** | 990 | **历史单败路径** | 见 §4 |

## 3. run_bracket 精准判定（668-849）
- **职责**：按 scheduled.format 分派 → 产出 BracketResult（champion+series+matches）。入口先校验赛制队伍数。
- **Major 专用分支（699-763）**：Swiss（强制 Group stage，写"小组赛"里程碑日志）→ SingleElimPlayoff（强制 Playoff stage，写"淘汰赛"里程碑日志）。两段各用自定义闭包注入 run_lod_series_with_importance。
- **普通分支（806-848）**：先首轮随机预打乱（Kotlin shuffled 语义，RNG 消费在闭包创建前），再按 SingleElim→legacy_single_elim / SwissPlayoff→SwissPlayoffBracket / DoubleElimGroups→DoubleElimGroupsBracket 分派。
- **判定**：⚠️ 修正 REDUNDANCY 发现 3——Major 内联**不是**与 SwissPlayoffBracket 的"语义重复"：差异（里程碑日志、stage 强制覆盖、RNG 消费序、决赛 BO5）都是功能契约。真正缺口是 SwissPlayoffBracket 接口无"阶段间回调"，Major 只能内联。可选改进：加 on_stage_transition 回调参数（增强，非去重）。

## 4. legacy_single_elim 精准判定（990-1089）——对 REDUNDANCY 发现 2 的修正
与 SingleElimPlayoff::run（format/single_elim.rs:24-82）逐行比对：

| 维度 | legacy_single_elim | SingleElimPlayoff::run | 是否真重复 |
|---|---|---|---|
| 首轮配对 | 物化 fixtures（T3 契约）| BracketSeeding::bracket_order | 否（不同源）|
| 轮空策略 | 末位轮空 | 最强种子 field[0] 轮空 | **否（行为不同！）** |
| stage 标注 | 恒 Playoff | QF/SF/Final 推导 | 否 |
| 赛制 | 恒 tier_best_of | best_of + final_best_of(BO5) | 否 |
| bye | fixtures is_bye 过滤 | 运行时 is_bye 晋级 | 否 |
| 胜者判定 | winner_sig==a/b.signature | 同左 | ✅ 唯一相同点 |

**结论**：❌ 发现 2"语义重复建议合并"是**误判**——两者轮空策略与赛制不同，合并会破坏 T3 预览一致性与单败守恒。真实重叠仅"while 循环+配对+胜者晋级"骨架 ~15 行。安全改进 = 提取 `advance_pair(a,b,best_of,stage,runner)` 辅助（双方共用），保留各自策略差异。

## 5. 代码规范问题
- **4 个大块头 pub 方法无 doc 注释**（run_tournament_detailed/simulate_event/plan_event_with_ranking/simulate_event_with_ranking）——违反"pub 方法必有 doc"规范，靠读实现才能懂（正是注意力涣散诱因）。
- 测试占文件 47%（950 行）与实现同文件——找代码要翻测试。

## 6. 判定汇总
| 报告结论 | 实测 |
|---|---|
| REDUNDANCY 发现 2：legacy_single_elim 与 SingleElimPlayoff 语义重复 | ❌ **误判**（轮空/赛制/配对来源均不同，合并危险）|
| REDUNDANCY 发现 3：Major 内联与 SwissPlayoffBracket 组装重复 | ❌ **误判**（里程碑日志/stage 强制/RNG 序是功能契约）|
| 真实可做 | 提取 `advance_pair` 共享骨架 + 补 pub 方法 doc + 测试分离 |
