//! # csc-career —— 生涯档案系统
//!
//! Kotlin `career/`（archive 部分）的转写（M5 前置：被 systems/core 消费）。
//!
//! - [`archive`]：CareerArchive（逐赛季明细 + 生涯汇总，纯值存储可存档）。
//!
//! 解耦说明：原 `yearly_rating`（YearlyRatingTracker）是**赛事结算的跨调用状态**，
//! 消费方是 csc-tournaments（SeriesSettlement），且消除 tournaments → career 的反向
//! 依赖后本 crate 仅依赖 entities——故已迁至 csc-tournaments（见其 `yearly_rating` 模块）。
//! （实际依赖 = domain + entities + util：`SeasonRecord` 携带 `SeasonGoal`、评价器消费
//! `TourneyTier`/`PlayerId`；2026 结构拆分已更正此注释。）
//! 转写差异：YearlyRatingTracker 的 Kotlin `playerOf: (String) -> Player?` 闭包
//! （解析实体并**就地写荣誉**）→ Rust 解耦：`award` 只返回排名结果（名字 + 场均），
//! 荣誉写入由结算层（csc-tournaments）负责——符合所有权边界。

pub mod archive;
pub mod career_ending;

pub use archive::{CareerArchive, SeasonRecord, SeasonTotals};
pub use career_ending::{CareerEnding, CareerEndingEvaluator, LegacyTier};
