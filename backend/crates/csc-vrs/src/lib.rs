//! # csc-vrs —— VRS 排名系统
//!
//! Kotlin `me.iverins.csc.vrs/`（5 文件 / 980 行）的 Rust 转写（M4）。
//!
//! 职责（官方两层模型）：
//! - **静态 Seed**（[`modifiers`]）：综合奖金/对手质量/线下战绩重算，remap 到 [400, 2000]；
//! - **动态 ELO**（[`scoring`]）：官方 Glicko 退化（`setFixedRD(75)`），逐场结算；
//! - **数据库**（[`database`]）：standings 初始基准 + 模拟推进状态 + 比赛历史。
//!
//! 转写差异（对应 docs/09 B2 + 蓝本 §3）：
//! - **matchHistory 窗口剪枝**：Kotlin 无界增长（20 年 ~15K 条）→ `reseed` 时
//!   按时间窗口裁剪旧记录（B2 消化）；
//! - **零 IO**：Kotlin `load(File)` → `from_json_files(&[(file_name, content)])`，
//!   文件读取由调用方（server/CLI）负责；
//! - 未知签名告警 `System.err` → `eprintln!`（不 fail-fast，部分库降级语义保留）；
//! - 依赖方向：`csc-domain` + `csc-time` + `csc-util`（被 csc-systems/csc-tournaments/csc-core 消费；
//!   `VrsEntry` 值对象已下沉 `csc-domain`，本 crate 经 `entry` re-export 兼容旧引用）。

pub mod database;
pub mod engine;
pub mod entry;
pub mod modifiers;
pub mod scoring;

pub use database::{VrsDatabase, VrsTeamState};
pub use engine::VrsEngine;
pub use entry::VrsEntry;
pub use modifiers::{MatchRecord, VrsModifiers};
pub use scoring::VrsScoring;
