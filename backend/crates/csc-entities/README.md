# csc-entities —— M2 模块交付

Kotlin `entities/`（19 文件 / 1546 行）的 Rust 转写。对照蓝本：`ARCHITECTURE-MAPPING.md` §4.1。

## 交付物

| 交付项 | 位置 | 状态 |
|---|---|---|
| 值类型：Role/属性组/PlayerCharacter/Team/CareerInfo/印记/伤病/化学/财务 | `src/{role,attributes,character,career,mark,injury,chemistry,team}.rs` | ✅ |
| 属性领域：Attr/RoleProfile/RoleProfiles/PowerCalculator | `src/{role_profile,power}.rs` | ✅ |
| 生成器：RandomPlayerGenerator（差异化机制） | `src/generator.rs` | ✅ |
| 真实位置基准：RoleBaseline（serde 解析） | `src/baseline.rs` | ✅ |
| **World arena**（ID 即索引 + 归属原子操作） | `src/world.rs` | ✅ |
| 跨语言 golden 对照（生成链位级验证） | 测试 + `tools/gen_entities_golden.kt` | ✅ |
| Kotlin 对照说明 | 本文件 | ✅ |
| **重大发现：Kotlin 双参数 RNG 原语差异**（见下） | `csc-util::next_i32_in` | ✅ |

## 核心重设计（对象图 → ID 引用）

| Kotlin | Rust | 收益 |
|---|---|---|
| 继承（PlayerCharacter ← Player/NPC） | **组合**：单一 struct + `career: Option` | 模拟路径统一、存档即结构体 |
| `Team.roster` 持对象引用 | `Team.roster_ids: Vec<PlayerId>` + World arena | 引用→索引，无别名 |
| `Team.with*` 不可变快照 | `Team` 可变 + World 原子操作 | 双端不变量测试锁死 |
| `FreeAgentTeam.instance` 全局单例 | **`Option<TeamId>`（None = 自由身）** | C1 消除，多世界隔离 |
| `CareerInfo.team` 引用 | 移除——统一到实体 `team` 字段 | 单一事实来源 |
| `EntityEngine`（name/sig → 实体 map） | `World`（arena + 线性查询） | 引擎编排留 M8 |

## ⚠️ 重大发现：Kotlin 单/双参数 `nextInt` 是**两种不同原语**

跨语言 golden 对照测试抓到：`nextInt(from, until)`（双参数，stdlib 默认实现）**不是**
`from + nextInt(until - from)`。反编译 `kotlin-stdlib` 确认（`commonMain/kotlin/random/Random.kt`）：

| 分支 | 单参数 `nextInt(until)`（DeterministicRandom 重写） | 双参数 `nextInt(from, until)`（stdlib 默认） |
|---|---|---|
| 2 的幂 | `nextLong() and (n-1)`（低 n-1 位） | `nextBits(fastLog2(n))`（取高 bitCount 位） |
| 拒绝采样 | `v = (nextLong() ushr 1).toInt()`（高 63 位） | `bits = nextInt().ushr(1)`（**低 32 位** ushr 1） |

两者取出的 31 位值在 nextLong 高 32 位为奇数时不同 → 拒绝行为分叉（实证：前 3 次相同，
第 4 次起不同）。**调用点必须与 Kotlin 一一对应**（`namePool.random`/`entries.random` 用单参数；
`nextInt(18,31)`/`nextInt(15,41)` 等用双参数）。Rust 侧已实现 `next_i32_bound`（单参数）+ `next_i32_in`（双参数）双原语，
各有 golden 锁死。

## 其他转写决策

- `RoleProfiles.prefs: Map<Attr,Double>` → **`[f64; 19]` 数组**（Attr 索引直映，零散列开销）；
- `statValue` 的 `p.pow(2.0)` → **`p * p`**（平方数学等价，消除 libm pow 跨语言位级差异）；
- `RoleBaseline` 手写 JSON 解析 → **serde 结构解析**（字段定死防漂移）+ 零 IO（`from_json_str`）；
- `PlayerFinance` 手写 Default → derive；
- World 的 id 即 Vec 索引（`next_*` 随存档序列化，恢复后不冲突）。

## 语义等价验证（golden 对照 Kotlin）

| 用例 | Kotlin 权威 | Rust 测试 |
|---|---|---|
| statValue(80..98, 0.85, 0.5, 0.3) | 91 | `golden_stat_value_and_prefs` |
| playerPower 固定属性组 ×3 角色 | toRawBits 位模式 | `golden_player_power_bits` |
| generateAttributes(seed42, TIER2, RIFLER, 20) | 全 19 属性 + potential=91 | `golden_generate_attributes` |
| potentialFor(20,60) ×3（双参数路径） | 76,77,95 | `golden_potential_for_sequence` |
| 双参数 nextInt(15,41)/(0,64)/(5,26) | 位级序列 | `golden_next_int_in`（csc-util） |
| World 双端不变量 | —（新设计） | `assign_and_release_keep_invariant` 等 |

## 编译与测试

```bash
cd backend
cargo test          # 117 用例全绿（domain 28 + entities 52 + time 19 + util 18）
cargo clippy --workspace   # 0 警告
```

## 遗留（后续模块消化）

- `VrsMapper`（standings → 实体）：依赖 `csc-vrs` 的 `VrsEntry` → **M4 后补**（在 entities 内实现）
- `EntityEngine` 引擎编排（syncRankings 等）：**M8 csc-core**
