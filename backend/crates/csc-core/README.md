# csc-core —— M7 模块交付（总引擎 + 完整存档 + 推进编排 + 查询层）

Kotlin `core/`（Engine/GameState/season 4 件套/query）转写。
对照蓝本：`ARCHITECTURE-MAPPING.md` §M7。

## 交付清单（13 测试）

| 模块 | Kotlin 源 | 职责 |
|---|---|---|
| `state` | GameState | **完整存档聚合根**（推进状态+时钟+RNG+决策日志+事件流+档案+**实体 arena+VRS+赛事结果+年度累计**） |
| `engine` | Engine | 总引擎接线层（装配/推进/存档/门面/standings 装载） |
| `season` | SeasonDirector | 月度阶段管线（7 步） |
| `loop_` | SeasonLoop | 队伍级互斥锁 + 月度赛事编排 |
| `settlement` | YearlySettlement | 跨年结算 7 步（颁奖/归档/成长/人口/薪资/代言/合同） |
| `batch` | WorldDecisionBatch | 世界级决策批次（伤病/训练/转会窗/代言 → 决策 → 记录 → 应用） |
| `query` | WorldQuery | 只读查询层（WorldSummary/PlayerProfile/增量游标） |

## 关键转写决策

### 1. 完整存档一次到位（Rust 值模型优势）
Kotlin GameState 因对象引用限制只是"第 2 块聚合根"（docs/09 阶段 2 的实体/VRS/
赛事/年度累计待后续并入）；Rust 中 `World`（ID arena）、`VrsDatabase`、
`TournamentResult`、`YearlyRatingTracker` 均为纯值 → **`GameState` 直接收敛全部
模拟状态**，`snapshot`/`restore` 一次还原。测试验证：推进 2 个月 → JSON 序列化 →
全新引擎恢复 → 两边继续推进的 RNG 轨迹/VRS 状态完全一致（可复现性闭环）。

### 2. 阶段管线（season.rs）
```
advance_month()
1. 时钟进新月 + 清上月锁       2. 【跨年】YearlySettlement（7 步业务契约）
3. SeasonLoop 月度赛事+锁       4. vrs.reseed（6 月滚动窗口）
5. month += 1                   6. chemistry.monthly_recovery
7. WorldDecisionBatch（伤病tick+决策批次）
```

### 3. 锁的双端一致性（真实 bug）
Rust `clear_locks` 初版只清 `Team.current_tournament_id` 没清 `locks` map
（Kotlin 是 `locks.clear()`）→ 上月锁签名残留，快照恢复时重复覆盖
（调试抓到：恢复后队伍锁名变成上月赛事）。修复后 map 与字段双清。

### 4. restore 顺序契约
**先还原实体 arena，再反解锁签名**（锁反解依赖新 world）——初版先反解旧 world
导致签名找不到 panic。

### 5. 其他 Kotlin 语义保真
- 跨年结算 7 步顺序不可变（先颁上年荣誉再成长、先退役再薪资、先归档再清空统计）；
- 代言结转**只复位评估年份**（合同年限结转是 EconomyEngine 单点职责——
  双扣 bug 的历史修复约定）；
- 月度薪资 = 玩家薪资 + NPC×100k（可透支为负，预算约束在转会端）；
- 归档后 `injury_days_this_year` 复位；合同年限 -1（0 不再减）；
- 训练选项 id = `TrainingFocus.name()`（Kotlin `valueOf` 反解）；
- T1 瑞士轮 ≥8 队门槛、T2 池 13~32 名——测试世界最小 40 队；生产资产 128 队（Kotlin 用真实 standings）。

## 语义等价验证

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| 快照 → JSON → 恢复 → 继续推进一致 | RNG/VRS/实体/事件/决策日志 | `snapshot_restore_full_roundtrip` |
| 锁值记录 roundtrip | 签名反解 + 重设占用标记 | `lock_roundtrip_via_records` |
| 签名反解失败显式抛错 | 拒绝静默丢锁 | `restore_missing_signature_panics` |
| 清锁 = 字段 + map 双清 | `locks.clear()` | `clear_locks_releases_teams_and_map` |
| 决策批次记录训练点 | 决策日志 + DecisionMade 镜像 | `batch_runs_and_records_decisions` |
| 合同 -1 / 代言复位 | 跨年结算 | `contracts_and_sponsor_rollover` |
| 薪资支出 = 薪资 + NPC×100k | 预算扣减 | `payroll_deducts_salary_and_npc_cost` |
| 查询层概览/档案 | 只读纯值 | `summary_and_profile` |

## 编译与测试

```bash
cd backend
cargo test          # 297 用例全绿（13 个 crate）
cargo clippy --workspace   # 0 警告
```

## 遗留（M8+）

- `csc-verify`（回放校验器：种子+决策日志 → 世界一致断言）；
- 跨语言对照测试（Kotlin golden × Rust 统计断言）；
- M9 server / M10 app+wasm / M11 前端。
