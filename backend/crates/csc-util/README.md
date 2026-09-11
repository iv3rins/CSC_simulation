# csc-util + csc-time —— M1 模块交付

Kotlin `util/`（DeterministicRandom）+ `time/`（SimClock/TimeWindow）的 Rust 转写。
对照蓝本：`ARCHITECTURE-MAPPING.md` §3-§7（M1）。

## 交付物

| 交付项 | 位置 | 状态 |
|---|---|---|
| xoshiro256\*\* RNG（位级对照 Kotlin） | `crates/csc-util/src/rng.rs` | ✅ |
| 稳定 ID + 分配器（修 B3/C1） | `crates/csc-util/src/id.rs` | ✅ |
| 零依赖日期算术（civil 算法） | `crates/csc-time/src/civil.rs` | ✅ |
| SimClock（月末截断语义） | `crates/csc-time/src/clock.rs` | ✅ |
| TimeWindow（时间衰减） | `crates/csc-time/src/window.rs` | ✅ |
| **跨语言 golden 对照**（第一道锁） | `crates/csc-util/src/rng.rs` 测试 + `tools/gen_rng_golden.kt` | ✅ |
| Kotlin 对照说明 | 本文件 | ✅ |

## 关键转写决策

### 1. RNG：位级一致的三道防线
- **同算法**：xoshiro256**（`rotl(s[1]*5,7)*9` + 状态推进），Kotlin `DeterministicRandom` 直译；
- **同种子派生**：splitmix64 展开 4 字（同常量 `0x9E3779B97F4A7C15` 等）；
- **golden 测试**：Kotlin 侧 `tools/gen_rng_golden.kt` 生成权威序列（**位模式**输出，
  避免跨语言十进制打印差异），固化进 Rust 测试——种子 42/7/0 的 nextLong/nextDouble/
  nextFloat/nextInt 序列 + 快照恢复，任何位级偏差立即红灯。

### 2. Kotlin 语义陷阱（逐位复刻）
| 陷阱 | Kotlin | Rust 处理 |
|---|---|---|
| 整数溢出 | Long/Int 补码 wrap | 全部 `wrapping_*` |
| `nextInt(until)` 拒绝采样 | `v - v % until + (until-1) < 0`，`v%until` 为 trunc 余数 | `wrapping_sub/wrapping_add` + Rust `%`（同 trunc 语义） |
| 2 的幂分支 | `nextLong() and (until-1)` | 同（用 `next_u64`，注意 Kotlin 实现取 nextLong 而非 nextInt） |
| `rotl(k=0)` | `ushr 64` 取模为 `ushr 0` | `u64::rotate_left`（k 取模 64，语义一致） |
| `nextDouble` | `(nextLong ushr 11) * 2^-53` | `(next_u64 >> 11) as f64 * (1/2^53)`（IEEE754 乘法位级一致） |

### 3. 日期算术零依赖
- 不引入 chrono：Howard Hinnant `days_from_civil`/`civil_from_days`（~60 行），
  与 Java `LocalDate` 语义一致（proleptic Gregorian、负年支持）；
- `advance_months` 用 `div_euclid/rem_euclid` 实现 Java `floorDiv/floorMod` 语义
  （月末截断：1-31 进 2 月 → 2-28/29；2024-02-29 +12 月 → 2025-02-28）；
- epoch 秒 = 序数日 × 86400（UTC 天首，与 `atStartOfDay(UTC).toEpochSecond()` 一致）。

### 4. ID 设计（对应 docs/09 B3/C1）
- `PlayerId(u32)`/`TeamId(u32)` 新类型 + `IdAllocator` 单调分配；
- **FreeAgentTeam 单例消除**：自由市场队伍用 `TeamId` 常量由世界持有者登记（C1）；
- `0` 保留为未分配哨兵；存档恢复 `IdAllocator.restore(next)` 防 ID 冲突。

## 语义等价验证（对照 Kotlin 行为）

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| seed(42) nextLong×10 | 位模式 `15780b2e...` 等 | `golden_next_long_42` |
| nextDouble/nextFloat 位模式 | toRawBits 序列 | `golden_next_double_42` / `golden_next_float_42` |
| nextInt(100/97/64/1) | 十进制序列 | `golden_next_int_bound_42`（97 非 2 幂拒绝采样路径） |
| seed(7)/seed(0) 序列 | 位模式 | `golden_other_seeds` |
| 快照→恢复→续推 | 状态 4 字 + 恢复序列一致 | `golden_snapshot_restore` |
| LocalDate 月末截断 | 1-31+1月→2-28、闰年 2-29、2-29+12月→2-28 | `month_end_truncation_matches_local_date` |
| epoch 秒 | 1970=0、2000=946684800、2026-06-08=1780876800 | `epoch_seconds_known_values` |
| rollingWindow | 6 个月前今天（含截断）→今天闭区间 | `rolling_window_is_closed_interval` |
| windowMod | start→0、end→1、中点→0.5、外部 clamp、退化→0.5 | `window_mod_*` |

## 编译与测试

```bash
cd backend
cargo test          # 64 用例全绿（domain 28 + time 19 + util 17）
cargo clippy --workspace   # 0 警告

# 重新生成 golden（Kotlin 侧，仅在 DeterministicRandom 变更时需要）：
cd design/kotlin_prototype/csc_prototype
kotlinc src/me/iverins/csc/util/DeterministicRandom.kt \
        ../../../../backend/tools/gen_rng_golden.kt -include-runtime -d /tmp/golden.jar
java -jar /tmp/golden.jar
```
