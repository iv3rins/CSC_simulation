# csc-systems —— M6 模块交付（五引擎合一 crate）

Kotlin 五个同包 sub-engine 转写：`chemistry/` `economy/` `injury/` `population/` `transfer/`
（按用户决策**系统包合一**合并为单 crate，模块边界保持）。
对照蓝本：`ARCHITECTURE-MAPPING.md` §M6。

## 交付清单（19 测试）

| 模块 | Kotlin 源 | 职责 | 测试 |
|---|---|---|---|
| `chemistry` | ChemistryEngine | 队友失误反应 → 关系/凝聚力/印记/冲突事件；月度关系恢复 | 4 |
| `economy` | EconomyEngine | 奖金分成 + 代言（评估点/签约/月付/年度结转） | 4 |
| `injury` | InjuryEngine | 月度 tick（恢复/倒计时/掷伤/缺勤累计）+ 伤病决策应用 | 4 |
| `population` | PopulationEngine | 退役（≥35）+ 青训新秀补位（Tier4/18~20）+ VRS 迁移 | 4 |
| `transfer` | TransferEngine | 转会窗（候选生成/决策执行/培养补偿费/合同签约） | 3 |

## 关键转写决策

### 1. World 为唯一状态容器
Kotlin 各引擎持 `EntityEngine`（名字 map + FreeAgentTeam 单例）→ Rust 引擎方法
接收 `&mut World`（ID arena + `free_agents: Vec<PlayerId>` + `team: Option<TeamId>`
自由身判定）。Kotlin `Team.with*` 不可变快照 → World 原子操作
（`assign_player_to_team`/`release_player` 双端不变量）。

### 2. 无状态引擎 + 参数化依赖
Rust 引擎结构（`pub struct ChemistryEngine;` 等）只暴露静态方法，依赖
（world/vrs/clock/journal/rng/decision batch）全部作为方法参数——
消除 `&mut` 借用冲突，M8 `csc-core` 可任意组合编排。

### 3. 退役 = 标记而非删除（ID 稳定性）
`World::remove_npc` 从物理删除改为**置 `retired` 标记 + 清引用**（arena 不收缩）：
- 「id = Vec 索引」存档不变量永不破坏（Vec::remove/swap_remove 会移动后续 ID，
  使其它 roster_ids 中的旧 ID 指向错误选手——实测抓出）；
- 退役者保留为档案记录；查询层（all_players/all_npcs/player_by_name）过滤；
- `PlayerCharacter` 新增 `retired: bool` 字段。

### 4. Kotlin 语义保真（防回归）
- 伤病决策 `PLAY_THROUGH`/`REST`：REST = 休养（`tick_resting` 双倍恢复）；
- **受伤当月只标记不扣减**（`ticked` 保护月——测试先写错被修正）；
- 退役者 partition（自由人/队伍内）分开处理——Kotlin 同；
- 转会候选：`eligible_teams` 门槛（T1≥85 等）+ `can_afford`（薪资+转会费）；
  执行时**重新解析目标队**（同窗互斥：签名已变 → 跳过告警）；
- 代言年限 `2 + nextInt(2)` 用**单参数 nextInt 原语**（nextLong 族）；
- 转会费 `power×10_000`；冠军奖金 60%/40% 分成 + 玩家 15%/8%。

## 语义等价验证表

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| 支持型反应改善关系 + 印记 | CONFRONT 降关系、SUPPORT 升关系 | `support_reaction_improves_relation_and_marks` |
| 冲突事件（关系 < 20） | 凝聚力 -3 + Conflict 日志 | `conflict_event_when_relation_drops` |
| 月度恢复收敛 50 | ±2/月 | `monthly_recovery_converges_to_default` |
| 奖金分成累计 | 冠军 15% | `prize_share_accumulates` |
| 代言门槛（rep ≥ 70） | 拒/收 | `sponsor_check_generates_offer_only_when_eligible` |
| 伤病 tick 保护月 | 当月不扣减 | `rest_flag_and_tick_double_recovery` |
| 痊愈清 injury + resting | + 事件日志 | `recovery_clears_injury_and_resting` |
| 退役补位 5 人不变式 | + VRS 签名迁移 | `veteran_retires_and_rookie_replenishes` |
| 自由人退役无补位 | | `free_agent_veteran_removed_without_intake` |
| 转会全流程 | 互换/预算/合同/自由市场 | `free_agent_gets_offers_and_transfers` |
| STAY 不转会 | | `stay_decision_skips_transfer` |

## 编译与测试

```bash
cd backend
cargo test          # 284 用例全绿（12 个 crate）
cargo clippy --workspace   # 0 警告
```
