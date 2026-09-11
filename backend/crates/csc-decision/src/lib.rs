//! # csc-decision —— 决策系统
//!
//! Kotlin `decision/`（5 文件 / 281 行）的转写（M5 前置：被 tournaments/systems/core 消费）。
//!
//! 事件驱动决策流的核心：引擎推进到"需要外部输入"的时刻产出 [`DecisionPoint`]（纯值、
//! 可序列化）→ 交给 [`DecisionSource`] 决策 → 返回 [`PlayerDecision`] 应用后继续。
//! 全部决策记录进 [`DecisionLog`]——**种子 + 决策日志 = 完整可复现（重放）**。
//!
//! 转写差异：`sealed interface` → enum（6 变体）；`fun interface` → trait；
//! 实体引用（TransferOffer 的 Player/Team）→ **ID + 签名**。

pub mod log;
pub mod offer;
pub mod point;
pub mod recorder;
pub mod source;
pub mod validation;

pub use log::DecisionLog;
pub use offer::{TransferOffer, TransferTarget};
pub use point::{
    DecisionPoint, InjuryOption, InterventionOption, PlayerDecision, SponsorOption, TrainingOption,
};
pub use recorder::DecisionRecorder;
pub use source::{AutoDecisionSource, DecisionSource};
pub use validation::{ValidationError, candidate_option_ids, validate_submission};
