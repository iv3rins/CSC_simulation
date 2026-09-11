//! # csc-tournaments —— 赛事系统
//!
//! Kotlin `tournaments/`（15 文件 / 1894 行）的转写（M5 分两轮）：
//!
//! **本轮（自包含部分）**：
//! - [`format`]：赛制引擎（纯对阵推进，经 `MatchRunner` 解耦——可独立测试）；
//! - [`invite_model`]：直邀拒绝模型（sigmoid 中心分布）；
//! - [`scheduled`]：ScheduledTournament / TournamentResult；
//! - [`calendar`]：赛季日历（月份 → 赛事模板 + 时间片）；
//! - [`scheduler`]：排程器（VRS 邀请 + 队伍池切片）。
//!
//! **下轮（依赖 csc-systems 的 chemistry/economy）**：
//! MatchConductor（图间决策循环）/ SeriesSettlement（结算）/ TournamentEngine（编排壳）。
//!
//! 核心转写设计：Kotlin 赛制引擎操作 `Team` 实体（标识+数据一体）→ Rust 分离为
//! [`format::TeamEntry`]（id + 签名，仅标识）与 `MatchRunner` 回调（结算闭包，
//! 由引擎持有 World 组装完整 roster）——赛制引擎零依赖实体，可独立测试。

pub mod calendar;
pub mod conductor;
pub mod engine;
pub mod format;
pub mod honours;
pub mod invite_model;
pub mod scheduled;
pub mod scheduler;
pub mod settlement;
pub mod top20;
pub mod yearly_rating;

pub use calendar::{ScheduledEvent, SeasonCalendar};
pub use conductor::{run_lod_series, run_lod_series_with_importance};
pub use engine::TournamentEngine;
pub use honours::{HonourDecision, HonourEvaluator, RatingAccum};
pub use invite_model::InviteModel;
pub use scheduled::{
    ScheduledStatus, ScheduledTournament, ScheduledTournamentRecord, TournamentResult,
};
pub use scheduler::TournamentScheduler;
pub use settlement::SeriesSettlement;
pub use top20::{Top20BoardEntry, Top20Commentary, Top20Evaluator, Top20Score, Top20YearBoard};
pub use yearly_rating::{YearlyRatingTracker, YearlyStat, YearlyStatAll};
