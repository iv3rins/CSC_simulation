//! # csc-events —— 世界事件日志
//!
//! Kotlin `events/WorldJournal.kt` 的转写（M5 前置：被 tournaments/systems/core 消费）。
//!
//! 所有事件为**纯值**（无对象引用，仅签名/名字/ID），可序列化、可存档、
//! 可增量拉取（seq 游标）——可复现性与表现层的地基。
//!
//! 转写差异：Kotlin `sealed interface` + `copy(seq=...)` stamp → Rust enum +
//! 手动 stamp（match 重建变体）；serde externally-tagged + SCREAMING_SNAKE_CASE
//! （`MATCH_PLAYED` 等，与 Kotlin 变体名一致）。

pub mod event;
pub mod journal;
pub mod narrator;
pub mod projection;

pub use event::{WorldEvent, WorldEventKind};
pub use journal::WorldJournal;
pub use narrator::{Narration, Narrator, NarratorView};
pub use projection::{JournalChannel, JournalProjection, JournalTier, ProjectionContext};
