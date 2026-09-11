# csc-vrs —— M4 模块交付

Kotlin `vrs/`（5 文件 / 980 行）的 Rust 转写。对照蓝本：`ARCHITECTURE-MAPPING.md` §3-§7（M4）。

## 交付物

| 交付项 | 位置 | 状态 |
|---|---|---|
| VrsEntry（JSON 字段 `teamName` 对齐数据源） | `src/entry.rs` | ✅ |
| VrsDatabase（加载/结算/阵容变更/排名刷新 + **窗口剪枝**） | `src/database.rs` | ✅ |
| VrsScoring（官方 ELO：Glicko 退化） | `src/scoring.rs` | ✅ |
| VrsModifiers（静态 Seed 三层函数链） | `src/modifiers.rs` | ✅ |
| VrsEngine 门面（资格/分级/迁移校验） | `src/engine.rs` | ✅ |
| **M2 遗留补全**：VrsMapper（standings → 实体） | `csc-entities/src/mapper.rs` | ✅ |
| **M3 遗留补全**：TransferRules::eligible_teams | `csc-simulation/src/transfer_rules.rs` | ✅ |
| **Kotlin roundToInt 语义修正**（floor(x+0.5)） | `csc-util::round_to_int` + M3 修正 | ✅ |
| 跨语言 golden（VrsScoring 位模式） | `scoring.rs` 测试 | ✅ |
| Kotlin 对照说明 | 本文件 | ✅ |

## 关键转写决策

### 1. matchHistory 窗口剪枝（docs/09 B2 消化）
Kotlin `matchHistory` 无界增长（20 年 ~15K 条）→ Rust `reseed` 时
`retain(timestamp >= window.start)`——窗口外记录对衰减恒为 0，剪掉后内存有界。
测试：`reseed_computes_seed_points_and_prunes_history` 锁死。

### 2. 零 IO 契约
Kotlin `load(File)` → Rust `from_json_files(&[(file_name, content)])`——
文件读取由调用方（server/CLI）负责；`parse_json` 用 serde 结构解析
（缺 ranking/teamName 丢弃、points/roster 缺省——与 Kotlin 正则解析语义对齐）。

### 3. roundToInt 语义（M3 修正的正式化）
Kotlin `roundToInt()` = `floor(x + 0.5)`（负半值向 +∞）；Rust `round()` 是
half-away-from-zero（`round(-2.5) = -3` vs Kotlin `-2`）——loseDelta 恒负，
1 分之差会破坏与 Kotlin 的积分一致性。`csc-util::round_to_int` 统一接入
（GrowthModel 成长 delta + MatchSimulator 比分/击杀 + VrsScoring 积分）。

### 4. HashMap 双可变借用
`apply_match` 需同时修改胜者/败者状态 → 从 map `remove` 取出修改后插回
（Kotlin 引用语义的 Rust 等价；败者缺失时恢复胜者）。

### 5. ASCII 排序澄清（测试预期修正）
Kotlin `sorted()` 是字典序：`'Z'(90) < 'a'(97)` → `"ZywOo"` 排在 `"apEX"` 前。
Rust `sort()` 同语义——初版测试预期写反，golden 与 Kotlin 实测后修正。

## 语义等价验证（golden 对照 Kotlin）

| 用例 | Kotlin 权威 | Rust 测试 |
|---|---|---|
| Q / G 常量 | 位模式 | `golden_scoring_formulas` |
| expected(2000,1000) / (1500,1500) | 位模式 | 同上 |
| winDelta / loseDelta ×3 场景 | 位模式（含负值） | 同上 |
| applyMatch 积分增减 + roundToInt | 逐场结算 | `apply_match_updates_both_sides` |
| 阵容变更 3 人清零 + 签名迁移 | 语义 | `roster_change_clears_points_on_3_departures` |
| 资格门面（T1/T2/T3/T4/Major） | 按 points 重排 | `qualification_gates` |
| VrsMapper 档位/槽位/基准优先 | 语义 | `mapper.rs` 测试（M2 遗留） |
| eligible_teams 门槛过滤 | 语义 | `transfer_rules.rs` 测试（M3 遗留） |

## 编译与测试

```bash
cd backend
cargo test          # 218 用例全绿（domain 28 + entities 67 + simulation 57 + vrs 25 + time 19 + util 22）
cargo clippy --workspace   # 0 警告
```

## 遗留（后续模块消化）

- 引擎编排（TournamentEngine 消费 vrs 门面）→ **M5 csc-tournaments**
- 真实 standings JSON 加载（server 层读文件 → from_json_files）→ **M9**
