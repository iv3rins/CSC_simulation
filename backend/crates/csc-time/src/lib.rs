//! # csc-time —— 模拟时钟 + 时间窗口
//!
//! Kotlin `me.iverins.csc.time/`（SimClock + TimeWindow）的 Rust 转写（M1）。
//!
//! 设计决策：
//! - **零日期库依赖**：不引入 chrono/time——用 Howard Hinnant 的
//!   `days_from_civil`/`civil_from_days` 算法实现公历 ↔ 序数日互转
//!   （~60 行，语义与 Java `LocalDate` 完全一致，且可序列化为纯整数）；
//! - 时钟内部状态 = `(year, month, day)` 整数三字段（与 GameState 快照形态一致）；
//! - epoch 秒 = 序数日 × 86400（UTC 天首，与 Kotlin `atStartOfDay(UTC).toEpochSecond()` 一致）；
//! - 依赖方向：本 crate 零依赖，供 vrs/events/tournaments 等消费（Kotlin 中 `vrs→time`）。

pub mod civil;
pub mod clock;
pub mod window;

pub use civil::{civil_from_days, days_from_civil, days_in_month, is_leap_year};
pub use clock::SimClock;
pub use window::{TimeWindow, remap_value_clamped};
