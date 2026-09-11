//! # csc-core —— M7：总引擎接线层 + 完整存档聚合根 + 赛季推进编排 + 查询层
//!
//! Kotlin `core/` 的转写：
//!
//! | 模块 | Kotlin 源 | 职责 |
//! |---|---|---|
//! | `state` | GameState | **完整存档聚合根**（推进状态 + 时钟 + RNG + 决策日志 + 事件流 + 档案 + 实体 arena + VRS + 赛事结果 + 年度累计） |
//! | `engine` | Engine | 总引擎接线层（装配/推进/存档/门面） |
//! | `season` | SeasonDirector | 月度阶段管线（8 步推进） |
//! | `loop` | SeasonLoop | 队伍级互斥锁 + 月度赛事编排 |
//! | `settlement` | YearlySettlement | 跨年结算 7 步（颁奖/归档/成长/人口/薪资/代言/合同） |
//! | `batch` | WorldDecisionBatch | 世界级决策批次（收集 → 决策 → 记录 → 应用） |
//! | `query` | WorldQuery | 只读查询层（表现层/服务端协议边界） |
//!
//! ## 转写核心设计
//!
//! - **完整存档一次到位**：Kotlin GameState 只是"第 2 块聚合根"（实体/VRS/赛事
//!   待后续并入，受对象引用限制）；Rust 为值模型——`World`（arena）、`VrsDatabase`、
//!   `TournamentResult`、`YearlyRatingTracker` 全部可序列化，`GameState` 直接收敛
//!   **全部模拟状态**（docs/09 路线图阶段 2 在 Rust 侧一次消化）；
//! - **字段级 split borrow**：Engine 各字段同时可变借用（Rust 保证无别名），
//!   编排函数以参数传递，Kotlin 的构造注入（依赖顺序易错）成为编译期检查；
//! - **锁 = 值记录**：`Team.current_tournament_id`（实体字段）+ `SeasonLoop.locks`
//!   （签名值列表）双端，存档反解签名 → TeamId（失败显式抛错，拒绝静默丢锁）。

pub mod batch;
pub mod client;
pub mod context;
pub mod engine;
pub mod insights;
pub mod loop_;
pub mod nan;
pub mod narrative;
pub mod query;
pub mod season;
pub mod settlement;
pub mod state;
pub mod world_stories;

pub use client::{
    CLIENT_VIEW_VERSION, ClientBracketSlot, ClientSeason, ClientState, ClientTournamentResult,
    ClientWorld, EventAggregate, EventChampion, EventTeam, MajorPresentation, PlayerCalendarEvent,
    TeamRef, calendar_results_for_year, event_aggregate, player_calendar_events,
    season_calendar_plan,
};
pub use context::WorldContext;
pub use csc_tournaments::top20::Top20Evaluator;
pub use engine::{CalibrationAssets, Engine};
pub use insights::{MatchInsight, TeamStatusInsight, match_insight, team_status_insight};
pub use query::{PlayerProfile, Top20Entry, WorldQuery, WorldSummary, top20_of};
pub use state::{GameState, LockRecord};
pub use world_stories::{Rivalry, TeamStory, WorldStories, world_stories};
