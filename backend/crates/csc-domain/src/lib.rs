//! # csc-domain —— 跨系统共享值对象
//!
//! Kotlin `me.iverins.csc.domain/`（8 文件）的 Rust 转写（转写蓝本：`ARCHITECTURE-MAPPING.md` M0）。
//!
//! 架构契约（与 Kotlin 一致）：
//! - **零依赖**：本 crate 不依赖任何其他 csc crate（cargo 编译期强制）；
//! - **纯值模型**：全部类型 `Copy + Clone + PartialEq + Serialize/Deserialize`，
//!   无行为副作用（判定/查表函数均为纯函数）；
//! - serde 枚举序列化使用 `SCREAMING_SNAKE_CASE`，**JSON 名与 Kotlin 枚举常量名一致**
//!   （`MAJOR`/`TIER0`/`VRS_GLOBAL`...），便于跨语言对照与未来兼容存档。
//!
//! 模块映射（Kotlin 文件 → Rust 模块）：
//!
//! | Kotlin | Rust | 说明 |
//! |---|---|---|
//! | City.kt | [`city`] | 城市 + 区域 |
//! | MatchResult.kt | [`match_result`] | 比赛结果 + 场地 |
//! | TeamTier.kt | [`team_tier`] | 队伍级别 + 静态分级 |
//! | Tier.kt | [`tier`] | 选手实力档位 + 区间表 |
//! | TierProfile.kt | [`tier_profile`] | 赛事画像 + 奖池/场地/赛制表 |
//! | Tournament.kt | [`tournament`] | 赛事值对象 |
//! | TournamentFormat.kt | [`tournament_format`] | 赛制（sealed → enum） |
//! | TourneyTier.kt | [`tourney_tier`] | 赛事等级 |
//! | VrsEntry.kt（原 vrs 包） | [`vrs_entry`] | 排名记录值对象（下沉：被 entities/simulation/vrs 三方消费） |

pub mod attr_range;
pub mod city;
pub mod event_importance;
pub mod live;
pub mod match_result;
pub mod real_top20;
pub mod season_goal;
pub mod team_tier;
pub mod tier;
pub mod tier_profile;
pub mod tournament;
pub mod tournament_format;
pub mod tourney_tier;
pub mod vrs_entry;

pub use attr_range::AttrRange;
pub use city::{City, Region};
pub use event_importance::EventImportance;
pub use live::{
    BombSite, EconomyBuy, LiveDecisionFeedback, LiveDecisionKind, LiveEconomy, LiveMatchPhase,
    LiveMatchState, MatchOutcomeAnalysis, Pace, Side, UtilityInventory, UtilityPlan, WeaponClass,
};
pub use match_result::{MatchResult, MatchVenue};
pub use real_top20::{RealTop20Index, RealTop20Player, RealTop20Year};
pub use team_tier::TeamTier;
pub use tier::{Tier, TierRanges, TierTable};
pub use tier_profile::{
    TierProfile, tier_best_of, tier_importance, tier_lan, tier_prize_pool, tier_venue,
};
pub use tournament::{InvitePolicy, Organizer, Tournament};
pub use tournament_format::TournamentFormat;
pub use tourney_tier::TourneyTier;
pub use vrs_entry::VrsEntry;
