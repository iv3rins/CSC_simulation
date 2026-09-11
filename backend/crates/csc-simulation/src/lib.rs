//! # csc-simulation —— 对战模拟规则系统（纯函数层）
//!
//! Kotlin `me.iverins.csc.simulation/`（13 文件）+ `hltv/`（RatingCalculator）的转写（M3）。
//!
//! 架构契约（与 Kotlin 一致）：
//! - **纯函数规则层**：零 IO、零引擎依赖；消费 entities 的实体/状态、domain 的值对象；
//! - 引擎（编排）在 `csc-tournaments`/`csc-systems`/`csc-core`，本 crate 只提供规则；
//! - 随机源一律显式传 `&mut Xoshiro256StarStar`（确定性；Kotlin 的默认 `Random.Default` 不转写）；
//! - 可复现性：**调用顺序与 Kotlin 逐位一致**（gaussian 消耗 2 个 nextDouble、pickIndex 1 个等）。
//!
//! 转写差异要点：
//! - `SeriesResult` 持 `Team` 实体引用 → **`TeamId` + 签名快照**（纯值，跨引擎传递）；
//! - `Team.roster` → `SeriesTeam { id, signature, roster: Vec<&PlayerCharacter> }`（World 组装）；
//! - `RatingCalculator`（hltv 包）并入本 crate 的 `rating` mod（唯一消费方是 MatchSimulator）；
//! - `TransferRules.eligibleTeams` 依赖 `csc-domain::VrsEntry`（值对象已下沉 domain）；
//! - Kotlin `Random.Default` 默认参数全部改为显式 rng 参数（确定性铁律）。

pub mod calibration;
pub mod chemistry_model;
pub mod condition;
pub mod directives;
pub mod finance;
pub mod form;
pub mod growth;
pub mod kills_alloc;
pub mod live_match;
pub mod mark_effects;
pub mod match_simulator;
pub mod rating;
pub mod replay;
pub mod series;
pub mod training;
pub mod transfer_rules;
pub mod volatility;
pub mod win_rate;

pub use calibration::{
    BASELINE_YEAR, CalibrationProfile, CalibrationReport, EcoCalibrator, PeakAgeStats,
    Top20AgeSample,
};
pub use chemistry_model::{BlunderReaction, ChemistryModel};
pub use condition::ConditionModel;
pub use directives::{BlunderKind, MatchDirectives, Playstyle};
pub use finance::FinanceModel;
pub use form::FormModel;
pub use growth::GrowthModel;
pub use kills_alloc::allocate_kills;
pub use live_match::{
    BombState, LiveMapConfig, LiveMatchEngine, LivePlayerSlot, LivePlayerState, LiveRoundDecision,
    LiveRoundEvent, LiveRoundEventKind, LiveRoundOutput, LiveRoundScore, LiveRoundState,
    LiveTacticalState, MR12_WIN_SCORE, SIMULATION_VERSION,
};
pub use mark_effects::MarkEffects;
pub use match_simulator::{MatchSimulator, SeriesTeam};
pub use rating::{MatchStats, RatingCalculator, RoundStats};
pub use replay::{
    Highlight, HighlightKind, HighlightRef, MAP_POOL_2026, MapReplay, MatchReplay, ReplayBuilder,
    RoundEvent, RoundReplay, RoundType, VetoAction, VetoStep, WeaponKind, fnv1a64,
};
pub use series::{MapScore, PlayerLine, SeriesResult, SeriesStage};
pub use training::{TrainingContext, TrainingFocus, TrainingModel};
pub use transfer_rules::TransferRules;
pub use volatility::VolatilityModel;
pub use win_rate::WinRateCalculator;
