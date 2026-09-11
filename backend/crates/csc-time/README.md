# csc-time —— M1 模块交付（时间系统）

Kotlin `time/`（SimClock + TimeWindow）的 Rust 转写。
对照说明见 `../csc-util/README.md`（M1 对照说明含时间部分）。

## 要点

- **零日期库依赖**：`civil.rs` 用 Howard Hinnant 算法实现公历 ↔ 序数日互转
  （`days_from_civil`/`civil_from_days`），与 Java `LocalDate` 语义一致；
- **月末截断**：`advance_months` 用 `div_euclid/rem_euclid`（Java floorDiv/floorMod 语义），
  1-31 进 2 月 → 2-28/29；闰日 2-29 +12 月 → 2-28；
- **epoch 秒** = 序数日 × 86400（UTC 天首）；
- **TimeWindow**：`end >= start` 契约（Kotlin `require` 等价）、退化区间取中点、
  `window_mod` 线性映射 + clamp。

## 测试

19 用例：civil 全量往返（1970-2100 逐日）、epoch 已知值、月末截断矩阵、
label 格式、rolling_window 闭区间、window_mod 边界/退化、serde 往返。

```bash
cd backend && cargo test -p csc-time
```
