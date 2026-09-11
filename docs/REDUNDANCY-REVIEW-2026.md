# 冗余与重复性审查报告 (2026-08-18)

> 审查范围：`backend/crates/csc-*`（重点：`csc-tournaments` 的赛事编排 / 结算 / 排程链路，
> `csc-simulation` 的比赛模拟，`format/` 赛制引擎群，`csc-domain` 的共享字典）。
> 方法：静态阅读 + grep 定位 + 逐锚点核对源码；**只做分析与收敛建议，不包含任何删除 / 重构动作**。
> 立场声明：本报告只对「职责相同、口径需保持一致」的代码给出「对齐到单一事实源」的建议，
> 任何「统一 / 移除」均须由后续独立的重构任务书承载，本报告不直接实施。
>
> 附注：本报告范围内的全部源文件均经 UTF-8 校验读取，文本完好；
> 终端 / 工具展示中的乱码仅为显示层伪影，与源码内容无关（非缺陷）。

---

## 1. 结论摘要

在 `backend/crates/csc-*` 范围内，本轮复核确认的重复 / 冗余**全部属于「同口径多实现」类**，
没有任何一处是需要立即删除的死代码或损坏文件。按收敛价值排序：

1. **「tier → 赛事重要度」映射三处同口径实现**（`engine.rs` / `scheduled.rs` / `conductor.rs`），
   是风险最高、收敛收益最直接的一处。
2. **`legacy_single_elim` 与 `SingleElimPlayoff` 的单败推进语义重复**，且前者未复用
   `BracketSeeding` / `format` 引擎（历史路径）。
3. **Major 内联瑞士轮 + 单败管线**与 `SwissPlayoffBracket` 组装语义重复（里程碑日志除外）。
4. **`format/` 四个赛制模块历史上各自的伪造 runner 已收敛到 `tests_common`**（既有正向先例）。
5. **结算层 / 模拟层**的精确与粗略双路径（`settle_series` vs `settle_quick_series`、
   `simulate_series` vs `simulate_quick_series`）属于**有意的 LOD 分层**，建议只做「共享口径」收敛，
   不可合并为单一路径。
6. 其余（排程门面、种子序辅助、年度榜表）为**独立校准的表面**，建议保持现状，仅登记口径差异。

**非发现（已核实，不构成冗余问题）**：`#[allow(clippy::too_many_arguments)]` 仅出现在
`run_tournament_detailed` / `settle_pending_event` / `simulate_event_with_ranking` / `run_bracket` /
`run_lod_series*` / `run_player_series` 等**编排与参数透传**函数上——参数数量来自跨层依赖注入
（world / vrs / clock / journal / decision / decision_log / settlement / rng 等），是 Rust 自由函数
转写 Kotlin 注入式构造后的签名形态，**不是重复代码的产物**，不计入冗余。

---

## 2. 已核实的分层与职责边界（判断冗余的前提）

下列边界全部经源码核对，是判断「哪些相似是重复、哪些相似是有意分层」的基准：

- **`csc-tournaments/src/engine.rs` 模块文档（L1-15）**明示：排程门面委托 `TournamentScheduler`；
  逐场 series 模拟经 `MatchRunner` 注入 `conductor::run_lod_series`（对局级 LOD + 玩家队伍的图间决策循环）；
  比赛级决策（场内干预 / 队友失误）完全在 conductor，引擎零感知；账目（结算公式）完全在 settlement，引擎零公式。
  模块文档逐条属实，**「match 级 LOD + 玩家队伍图间决策循环 + RNG 消费顺序 + first_round fixtures 物化」
  均为有意的功能设计**，不是冗余——相关代码块（`conductor.rs` 的决策循环、`engine.rs` 的预打乱
  与首轮物化落位）必须按「功能契约」保留。
- **`conductor.rs` 模块文档（L1-16）**：从赛事引擎拆出的比赛级决策循环；自由函数参数化
  消除借用冲突；结算层状态收在 `SeriesSettlement`。
- **`settlement.rs`**：`settle_series`（精确路径：生涯回写 → 疲劳 → 日志 → VRS）与
  `settle_quick_series`（粗略路径：士气 → 年度累计 → 疲劳 → 日志 → VRS）共享的尾部
  （疲劳、日志、VRS 积分）与差异头部（生涯回写 vs 士气反馈）是**结算 LOD 的有意分层**，
  粗路径额外把 NPC 对局计入年度累计是有意的 2026 修订，两者不可合并。
- **`format/bracket.rs` 模块文档（L10-12）+ `format/mod.rs`（L9-15）**：赛制引擎只做「对阵推进」，
  经 `MatchRunner` 回调解耦结算；测试 runner 已在 `tests_common` 收敛（唯一事实源），
  是仓库内已经完成的收敛样板。

---

## 3. 发现清单（按收敛价值排序）

### 发现 1（高）：`tier → EventImportance` 映射存在三处同口径实现

| 位置 | 内容 |
|---|---|
| `engine.rs` `scheduled_importance`（L882-893） | `Major→Championship`、`SuperElite/Elite→Major`、`T1→Important`、`T2/Qualify→Background` |
| `scheduled.rs` `build_event`（L79-82 附近） | 同上的 tier 匹配分支，写入 `scheduled.importance` |
| `conductor.rs` `run_lod_series`（L76-81） | 为「无 importance 透传」的调用点推导同一映射 |

- **口径完全一致**（三处均为同一 match 表达式语义），属「同一事实多处书写」。
- **风险**：若未来调整（例如新增 tier、调整 T2 的体验重要度），三处需要同步修改；
  漏改一处会造成「预览记录 importance ≠ 运行时 importance ≠ 决策 LIVE 开关」的错位。
- **收敛建议**：在 `csc-domain` 的 tier / event 共享字典侧新增一个 `tier_importance(tier)`
  纯函数（与 `tier_best_of`、`tier_lan`、`tier_prize_pool` 并列），三处调用点改为调用它，
  **对齐到单一事实源**；`engine.rs` 中保留的 `scheduled_importance` 只做薄转发（或直接改调用点），
  不改变任何现有调用语义。
- 说明：`engine.rs` 测试（L1108-1116 一带）用 `scheduled.importance = EventImportance::Important`
  断言 T1 口径——属于对上述事实源的**行为锁定测试**，收敛后应继续通过（口径不变）。

### 发现 2（高）：`legacy_single_elim`（engine.rs L997-1087）与 `SingleElimPlayoff::run` 单败推进语义重复

- `legacy_single_elim` 完整手写单败淘汰推进：循环按 field 减半、奇数轮空、逐轮用
  `tier_best_of(tier)`（`csc_domain::tier_profile`，单一事实源）+ `SeriesStage::Playoff` 结算；
- `format/single_elim.rs` `SingleElimPlayoff`（L24-82）：同一推进语义，另有按 field 尺寸
  （8=QF / 4=SF / 2=Final）推导 `SeriesStage`、决赛 BO5 特判与 `bracket_order` 种子序；
- 两者**逐轮语义高度重叠**：都是「轮空晋级 / 两两配对 / 胜者签名判定 / 冠军收敛」，
  差异在 stage 标注、决赛 best_of 与种子序来源（`bracket_order` vs 调用方随机预打乱序）。
- **历史成因**：`legacy_single_elim` 是 SingleElim 默认赛制路径（全程按 tier 赛制、无决赛特判），
  并携带 `first_round_fixtures` 物化落位（2026-08-20 起保证「赛前预览 = 实际赛程」、
  赛后回填 100% 命中），Kotlin 侧旧语义随机序依赖。
- **收敛建议（不动功能契约）**：
  1. 把「field 减半推进 / 轮空 / 胜者判定 / 冠军收敛」的公共骨架收敛到 `format/` 的单一赛制引擎
     （例如让 `SingleElimPlayoff` 支持「外部首轮配对（fixtures）」与「stage 标注策略」两个可选入参），
     使 `legacy_single_elim` 成为薄适配层，只保留 tier 赛制默认与 fixtures 注入；
  2. 或最小改动：先把 `legacy_single_elim` 中的胜者判定 / 轮空分支抽取为 `format/bracket.rs`
     的共享辅助（与 `BracketSeeding` 并列），再逐步让 `SingleElimPlayoff` 复用。
  3. `first_round_fixtures` 的物化配对、RNG 消费顺序（随机打乱发生在闭包创建前）必须保持原序，
     属可复现性契约的一部分（详见 §5.2），收敛时不得改变其时序。
- **候选收敛锚点**：`format/` 的赛制引擎与 `engine.rs` 历史路径之间至今没有共享的
  「配对 → 推进 → 产出」可复用单元；建议以 `BracketSeeding::bracket_order` /
  `TeamEntry::is_bye` 为既有共享起点。

### 发现 3（中）：Major 内联双阶段管线与 `SwissPlayoffBracket` 组装语义重复

- `engine.rs` `run_bracket`（L703-782）为 `Major` + `SwissPlayoff` 内联执行：
  先 `SwissStage::new(...).run`（stage 强制 `Group`），再 `SingleElimPlayoff::new(...).run`
  （stage 强制 `Playoff`），合并 series，另在组前 / 组后写两段 `TournamentStage` 里程碑日志；
- `format/swiss_playoff.rs` `SwissPlayoffBracket::run`（L40-60）做同样的两步组装
  （Swiss → playoff → 合并 series → 转 `BracketResult`），**无里程碑日志**；
- 逐字重复度不高（milestone 日志与 stage 强制闭包是内联版独有），但**组装结构（组合顺序与产出）重复**，
  未来若调整 SwissPlayoff 的组装细节（例如晋级数传递、series 合并次序），需要同步两处。
- **收敛建议**：保留内联版的**赛事叙事职责**（里程碑日志、Major 专属 stage 标注），
  将「Swiss 结果 → 单败 → 合并 series → BracketResult」的纯组装下沉到 `format` 侧单一函数；
  内联版只保留「注入 runner 闭包（含强制 stage）」+「写日志」两层职责，其余与
  `SwissPlayoffBracket` 对齐到同一组装事实源。RNG 消费顺序、journal 顺序不变。

### 发现 4（低-中，既有正向先例）：`format/` 四引擎测试 runner 的重复已收敛

- 历史状态：`double_elim.rs` / `swiss.rs` / `swiss_playoff.rs` / `single_elim.rs` 曾各自内嵌
  一份几乎逐字符相同的 `runner()` 伪造回调（risk_diagnose Jaccard=1.00）；
- 现状：已提取到 `format/tests_common.rs`（`runner_a_wins`，模块文档明示「唯一事实源」），
  四个模块的测试统一 `use crate::format::tests_common::runner_a_wins`（源码已逐文件核对）。
- 结论：**该处冗余已收敛**，无需新增动作；作为「重复 → 单一事实源」的既有成功案例列入本报告，
  供后续收敛动作参考其模式（提取共享模块 + 模块文档记录动机与 Jaccard 证据）。

### 发现 5（低，口径共享而非合并）：结算层 / 模拟层的精确与粗略双路径

- `settlement.rs`：`settle_series`（L101-120）vs `settle_quick_series`（L125-170）；
- `csc-simulation/match_simulator.rs`：`simulate_series`（逐图精确 + `win_rate` 全参数）vs
  `simulate_quick_series`（粗略内联 + 简化胜负模型）。
- 两者差异是**有意的对局级 LOD**：精确路径服务玩家在场 / 可回放 / 完整生涯回写；
  粗略路径服务 NPC 批量比赛（无 CareerInfo、无逐图决策、事件流降噪）。`settle_quick_series`
  头部（士气反馈 + 年度累计全员）与尾部（疲劳 / 日志 / VRS）共享 `record_yearly_and_career` /
  `apply_fatigue` / `record_match_played` / `vrs.apply_match_result` 等既有辅助——内部已经按
  「共享尾部 + LOD 差异头」组织。
- **收敛建议（不合并路径）**：只把两个 LOD 之间的**公共口径**收拢到共享辅助：
  ① 胜者签名判定（`winner_sig == team_a_sig / team_b_sig` 三态）在 `settle_series`、
  `settle_quick_series`、`apply_career_stats`、`record_match_played` 中重复出现——可抽
  `fn winner_side(series) -> Option<(TeamId, TeamId)>` 之类单一辅助；② BO1 与 BO3/5 的
  「比分回填推导」（`engine.rs` `backfill_fixture_score` 的 BO1 取末图回合分 / 系列取胜场数）
  与前端 HLTV 展示口径一致即可，无需新代码。
- `format/` 与 `engine.rs` 内的「胜者 = 签名匹配方」判定（`double_elim.rs` `play`、
  `single_elim.rs` run 内、`swiss.rs` 记录更新、`engine.rs` `legacy_single_elim`）同属一族，
  可统一收敛到 `TeamEntry` 侧的单一判定辅助（如 `winner_of(a, b, &series)`）。

### 发现 6（信息级，非缺陷）：排程 / 种子序 / 年度榜各表是独立校准表面

下列表面**不是重复实现**，但口径彼此独立，改动一方不会自动传导，登记以便日后对照：

- **排程门面两段式**（`scheduler.rs` `schedule_t1/t2/t3` + `schedule_for` 分发 L229-271 +
  `resolve_t3_participants` L286+）与 `calendar.rs` 的 `top_event`（L404-453，importance 映射
  存在 `Major≠Championship` 的差异：日历给的是「赛事档期重要度」，不是「玩家体验重要度」）
  各自校准，建议在两侧注释中互指，避免被误读成同口径映射。
- **`scheduled.rs` 首轮 fixtures 物化**（`single_elim_bracket_fixtures` 等，L296-419+）与
  **运行时种子序**（`BracketSeeding` / `bracket_order_ids` 等）是「预览面」与「运行面」两个
  **故意的**物化 / 推导来源；因 T3 预览一致性契约，两者语义必须一致，但建议以
  `bracket_order` / `bracket_order_ids` 为单一推导源，fixtures 只做「快照物化」不做「第二套推导」。
- **年度榜等级表**：`yearly_rating.rs` `tier_weight`（L25-33）、`top20.rs` `mvp_score`/`evp_score`
  （L128-146）、`win_rate.rs` `tier_factor`（L32-41）、`calendar.rs` `tier_ordinal`（L573 一带）
  各自是**独立校准的经验权重**（不同量纲：Rating 权重 / MVP 得分 / 胜率因子 / 排序序数），
  **不应合并**；只需在各自文档注明「与其它 tier 权重表无关」防止误读。
- 共享字典 `csc-domain` 的 tier 族枚举（`TourneyTier` / `TeamTier` / `Tier` / `EventImportance`）
  本身是**唯一定义一次**的共享词汇，不计入任何重复。

---

## 4. 已核实不存在 / 明确排除的类别

- **源码损坏 / 乱码**：全部目标源文件为合法 UTF-8（以 `ReadAllLines` UTF-8 读取逐行核对），
  报告撰写期间观察到的乱码仅来自终端输出管道的显示伪影，**不是源码问题**（结论：非发现，已关闭）。
- **`#[allow(clippy::too_many_arguments)]`**：仅见于编排 / 参数透传函数（`run_tournament_detailed`
  L292、`settle_pending_event` L487、`simulate_event_with_ranking` L578、`run_bracket` L667、
  `conductor.rs` 的 `run_lod_series` L44 / `run_lod_series_with_importance` L86 / `run_player_series`
  L144 等）。成因是 Rust 自由函数转写 Kotlin 构造注入的签名形态（跨层依赖全部参数化，
  模块文档已明示为转写差异）——不是重复 / 冗余的产物，不计入冗余。
- **结算 / 模拟双路径**（发现 5 所述）：是有意 LOD，不是需要删除的重复。
- **排程两段式**（plan → settle）：是「先登记、后按日历结算」的状态机设计，不是重复。

---

## 5. 附：约束与注意事项（收敛时不可逾越的契约）

### 5.1 功能契约（来自模块文档，均已核对属实）

- `engine.rs` 模块文档：逐场经 `MatchRunner` 注入 `run_lod_series`（对局级 LOD + 玩家队伍
  图间决策循环）；比赛级决策完全在 conductor，引擎零感知；账目公式完全在 settlement，引擎零公式。
- `conductor.rs` 模块文档：**图间决策循环**（`run_player_series`）——首图前赛前方案、决胜局
  关键调整窗口、LIVE 阶段节点整场最多两次；`interactive = importance.is_live()`；玩家不在场走
  `run_quick_series` 粗略路径；场内干预 / 队友失误决策只产生于玩家在场。
- 上述职责划分是**架构性意图**：任何收敛不得把决策逻辑拉回 engine，也不得把结算公式外移。

### 5.2 可复现性契约（RNG / 顺序）

- `run_bracket`（engine.rs L784-789）：首轮随机配对预打乱（Kotlin `shuffled(random)` 语义）
  的 **RNG 消费发生在结算闭包创建之前**；Major 分支与 `match_runner` 闭包内部也按固定序消费。
  收敛 / 对齐代码结构时**不得改变 RNG 消费顺序**（同种子 + 同决策序列 → 同世界的可复现性契约）。
- `first_round_fixtures` 物化（scheduled.rs 排程期）与运行时落位（`legacy_single_elim` 首轮）
  的先后、回填（`backfill_fixture_results`）的匹配顺序都属存档一致性面，收敛时保持原时序。

### 5.3 行为锁定测试

- `engine.rs` 测试区（L1096 起，如 `empty_participants_returns_none`、T1 importance 断言等）
  与 `format/*` 各模块测试（现统一走 `tests_common::runner_a_wins`）是收敛后的**回归护栏**：
  建议任何对齐改动先跑 `cargo test -p csc-tournaments -p csc-simulation` 全绿再提交。

---

## 6. 后续动作建议（全部为分析与立项输入，非本次执行）

1. 立项（小）：`tier_importance(tier)` 单一事实源化 —— 见发现 1。
2. 立项（中）：`legacy_single_elim` 与 `SingleElimPlayoff` 的公共骨架收敛 —— 见发现 2。
3. 立项（中）：Major 内联组装与 `SwissPlayoffBracket` 的组装事实源对齐 —— 见发现 3。
4. 立项（低）：胜者判定三态辅助抽取 —— 见发现 5。
5. 非动作项：`tests_common` 收敛（发现 4）已达成，仅作样板记录。

---

*报告生成：2026-08-18。仅分析与收敛建议；无删除 / 重构指令。*
