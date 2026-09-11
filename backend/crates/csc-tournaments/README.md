# csc-tournaments —— M5 模块交付（两轮完成）

Kotlin `tournaments/`（15 文件 / 1894 行）的转写（两轮：自包含部分 + 核心引擎）。
对照蓝本：`ARCHITECTURE-MAPPING.md` §3-§7（M5）。

## 交付总览（25 测试）

| 交付项 | 位置 | 状态 |
|---|---|---|
| 赛制引擎：TeamEntry/MatchRunner/BracketSeeding/单败/瑞士轮/双败/赛制 A 管线 | `src/format/`（6 文件） | ✅ |
| 直邀拒绝模型（sigmoid 中心分布） | `src/invite_model.rs` | ✅ |
| ScheduledTournament / TournamentResult（**ID 化**） | `src/scheduled.rs` | ✅ |
| 赛季日历（并行生态位：T1/T2/T3 + Major） | `src/calendar.rs` | ✅ |
| 排程器（VRS 邀请 + 队伍池切片 + 空库回退；**参数化依赖**） | `src/scheduler.rs` | ✅ |
| **比赛指挥**：图间决策循环（干预/失误） | `src/conductor.rs` | ✅ |
| **结算层**：生涯回写/疲劳/日志/VRS/奖金/荣誉/年度 TOP20 | `src/settlement.rs` | ✅ |
| **赛事引擎**：编排壳（分派/模拟/收官/年度） | `src/engine.rs` | ✅ |
| **前置共享包**：csc-events / csc-decision / csc-career | `crates/` ×3 | ✅ |
| **M6 依赖**：csc-systems（chemistry/economy） | `crates/csc-systems` | ✅ |
| Kotlin 对照说明 | 本文件 | ✅ |

## 关键转写决策

### 1. 赛制引擎零实体依赖（核心设计）
Kotlin 赛制引擎操作 `Team` 实体（标识+数据一体）→ Rust 分离：
- `TeamEntry { id, signature }`（仅标识，BYE 用 `TeamId::NONE` 哨兵）；
- `MatchRunner = dyn FnMut(TeamId, TeamId, i32) -> SeriesResult`（结算闭包由引擎注入）；
- 赛制引擎只做对阵推进，**可脱离模拟环境独立测试**（测试注入固定胜者 runner）。

### 2. NONE 哨兵冲突（真实 bug）
`TeamId::NONE = 0` 与 World arena（id 从 0 起）冲突——`is_bye` 把合法队伍当轮空。
修复：**NONE = u32::MAX**（0 是合法索引）。测试矩阵抓出（单败/双败/瑞士轮全挂）。

### 3. 排程器依赖注入
Kotlin 持 EntityEngine（签名 → 实体）→ Rust 注入 `TeamBySig`/`TeamFreeCheck` 闭包
（`TeamBySig<'a>` 生命周期别名），排程器不直接依赖 World 内部。

### 4. 日历构建器零引用捕获
Kotlin `ScheduledEvent.build: () -> Tournament` 闭包捕获 clock 引用 → Rust
`EventBuilder = Box<dyn Fn(i32, &str) -> Tournament>`（只捕获 year 值，date 由调用处注入）。

## 前置共享包（M5 依赖链）

| crate | 内容 | 测试 |
|---|---|---|
| csc-events | WorldEvent（enum 10 变体 + with_seq stamp）+ WorldJournal（seq 游标） | 6 |
| csc-decision | DecisionPoint（enum 6 变体）/DecisionSource trait/AutoDecisionSource/DecisionLog/DecisionRecorder/TransferOffer | 8 |
| csc-career | CareerArchive/SeasonRecord/SeasonTotals + YearlyRatingTracker（解耦：award 返回结果，荣誉写入归结算层） | 8 |

## 语义等价验证

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| 蛇形分组 8 队 2 组 | 1,4,5,8 / 2,3,6,7 | `snake_seed_balances_strength` |
| 标准种子对阵 8 队 | 1,8,4,5,2,7,3,6 | `bracket_order_standard_8` |
| 单败 4 队 3 场 / 5 队补 BYE 4 场 | 场次矩阵 | `single_elim_*` |
| 双败小组 5 场 / 16 队 27 场 | 场次矩阵 | `double_elim_*` |
| 弹性降级（6 队 → 单败 5 场） | 语义 | `elastic_downgrade_on_non_multiple` |
| 瑞士轮 3 轮 8 队 19 场 | 场次矩阵 | `swiss_*` / `swiss_playoff_8_teams` |
| 直邀拒绝率边界 | 中心 5%、封顶 10% | `decline_rate_bounded_and_monotonic` |
| 日历 T1/T2/T3 密度 | 2026 全年 25 场 T1 桶 + 60 场 T2 + 96 场 T3 | `calendar.rs` 测试 |
| 池切片轮转 | offset 错位 | `slice_pool_rotates` |

## 编译与测试

```bash
cd backend
cargo test          # 260 用例全绿（10 个 crate）
cargo clippy --workspace   # 0 警告
```

## 第二轮核心设计（conductor/settlement/engine）

### 自由函数 + 参数化依赖
Kotlin 三个类互相注入引用 → Rust 全部方法参数化（world/vrs/clock/journal/
decision/decision_log/settlement），结构只持有**跨调用状态**（settlement 的
`YearlyRatingTracker`、engine 的排程产物/赛事列表）。借用检查器在编译期
锁死依赖方向——Kotlin 靠纪律维持的架构在 Rust 中成为类型系统保证。

### MatchRunner 闭包独占 rng
`run_bracket` 的结算闭包捕获 `&mut rng`（`move`），因此**全部赛制引擎的 rng
参数删除**（第一轮误留 `_rng`——Kotlin 中赛制引擎同样不消费随机，随机只在
MatchRunner 内消费）：首轮随机配对（`shuffled(random)`）预打乱在闭包创建**前**，
随机消费顺序与 Kotlin 一致（可复现性不破坏）。

### 结算管线（settlement 单点）
- `settle_series`：生涯回写（K/D/胜场/士气）→ 疲劳（`fatigue_cost × 场地`）→
  MatchPlayed 日志 → `vrs.apply_match_result(timestamp=当下, window=None)`；
- `award_event_honours`：冠军 TeamHonour + `apply_champion_bonus`；MVP =
  逐图 Rating 场均最高（≥2 图防一轮游）；`apply_mvp_bonus` + HonourAwarded；
- `award_prize_money`：60%/40% 队伍预算 + 玩家 15%/8% 分成；
  **亚军守卫**：最后一场 series 必须含冠军，否则不发放（宁可少发不可错发）；
- `year_end_awards`：`YearlyRatingTracker.award`（场均而非累计）→ TOP20 荣誉 +
  HonourAwarded 日志。

### 图间决策循环（conductor）
- 每图前：玩家队伍 → `MatchIntervention` 点（第 0 项 = 印记派生基线，
  暂停 +3 加成）→ 决策应用为 `MatchDirectives` + 风格印记累积；
- 赛后：NPC 单图击杀 ≤ 1 且显著低于实力份额 → `TeammateBlunder` →
  `ChemistryEngine::apply_blunder_reaction`（关系/士气/印记/冲突事件）；
- 无玩家对局 → `simulate_quick_series`（零决策点，VRS 照常结算）。

## 语义等价验证

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| 4 队单败 3 场 | 场次矩阵 | `legacy_single_elim_four_teams` |
| 空参赛名单 → None | | `empty_participants_returns_none` |
| 干预点默认选项 | 印记派生基线 + 5 选项 | `intervention_point_default_derived_from_marks` |
| 直邀拒绝率边界 | 中心 5%、封顶 10% | `invite_model` |
| 日历 T1/T2/T3 密度 | 2026 全年 25 场 T1 桶 + 60 场 T2 + 96 场 T3 | `calendar` |
| 池切片轮转 | offset 错位 | `slice_pool_rotates` |
| 瑞士轮/双败/单败场次 | 场次矩阵 | `format/` 各模块 |

## 编译与测试

```bash
cd backend
cargo test          # 284 用例全绿（12 个 crate）
cargo clippy --workspace   # 0 警告
```

## 遗留（M7/M8）

- csc-core：Engine/SeasonDirector（月度阶段管线）/GameState（含 rng/decisions/
  journal/archive 的完整存档）；csc-query / csc-verify；
- 日历驱动赛事流（SeasonLoop：并行日历 + 队伍级互斥锁定）。
