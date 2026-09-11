//! 世界推进上下文（World Context）——聚合「推进一个月」各阶段共享的全部可变世界状态引用。
//!
//! 收敛原因：`season`/`loop_`/`settlement`/`batch` 的编排函数原先各自携带
//! 8~11 个参数（world/vrs/tournaments/clock/decision_log/journal/archive…），
//! 全部是 Engine 字段的 split borrow。本结构体把这些共享引用打包为一，
//! 各阶段函数签名从 `(&mut World, &mut VrsEngine, &mut TournamentEngine, &mut SimClock,
//! &mut DecisionLog, &mut WorldJournal, &mut CareerArchive, …)` 收敛为
//! `(&mut WorldContext, …)`，消除 `#[allow(clippy::too_many_arguments)]`。
//!
//! **不含** [`SeasonDirector`]（其 `rng` 需单独 split borrow）、[`SeasonLoop`]
//! （作为 `&mut self` 方法接收者）、[`DecisionSource`]（引擎外部注入的 trait object）、
//! [`SeasonCalendar`]（不可变配置）——这些保持独立参数，生命周期与借用语义更清晰。
//!
//! ## 字段语义分区（2026 收窄）
//!
//! 7 个字段按「读写角色」分三组，帮助维护者辨别「谁在阶段间流动、谁只被少数阶段消费」：
//!
//! | 组 | 字段 | 角色 | 消费阶段 |
//! |---|---|---|---|
//! | 世界状态 | `world` / `vrs` / `tournaments` / `clock` | 跨月推进**持久状态**（存档全量收敛） | 几乎所有阶段 |
//! | 记录侧 | `journal` / `decision_log` | 事件/决策**只追加**的可复现性流水 | 几乎所有阶段 |
//! | 档案 | `archive` | 仅跨年结算写入、只读查询出口 | 仅 `settlement` |
//!
//! **为何不用「子 context」拆分（决策记录）**：7 字段中 6 个几乎被所有阶段消费，
//! 唯一低使用率的是 `archive`（仅 `settlement` 写）。按阶段拆 `BatchContext`/
//! `SettlementContext`/`LoopContext` 会引入大量重复结构体定义与字段拷贝样板，
//! 而收益仅是「编译期约束 `batch` 不碰 `archive`」——一个低价值约束：`&mut`
//! 独占借用已保证各阶段不可能出现别名/数据竞争，误触字段只是逻辑错误而非内存错误。
//! 故保留单 `WorldContext`，用上方语义分区 + 本段决策记录表达边界，而非机械拆分。

use csc_career::archive::CareerArchive;
use csc_decision::log::DecisionLog;
use csc_entities::baseline::RatingProfile;
use csc_entities::world::World;
use csc_events::journal::WorldJournal;
use csc_text::TextBundle;
use csc_time::clock::SimClock;
use csc_tournaments::engine::TournamentEngine;
use csc_vrs::engine::VrsEngine;

/// 世界推进上下文：各阶段共享的可变世界状态引用（字段级 split borrow）。
///
/// 字段读写角色见模块文档的「语义分区」三组表格。
pub struct WorldContext<'a> {
    // —— 世界状态（跨月持久，存档全量收敛）——
    /// 实体 arena（选手/队伍/自由市场）
    pub world: &'a mut World,
    /// VRS 子系统（排名结算）
    pub vrs: &'a mut VrsEngine,
    /// 赛事子系统（排程/赛制/结算编排）
    pub tournaments: &'a mut TournamentEngine,
    /// 共享日期线程
    pub clock: &'a mut SimClock,
    // —— 记录侧（事件/决策只追加，可复现性流水）——
    /// 世界事件日志（叙事/复盘/增量同步）
    pub journal: &'a mut WorldJournal,
    /// 决策日志（可复现性第二支柱）
    pub decision_log: &'a mut DecisionLog,
    // —— 档案（仅跨年结算写入）——
    /// 生涯档案（逐赛季记录）
    pub archive: &'a mut CareerArchive,
    // —— 装配配置（只读，跨年结算消费）——
    /// 新选手实力分布模型（`rating_profile.json`；None = 新秀固定 Tier4 旧逻辑）
    pub rating_profile: Option<&'a RatingProfile>,
    /// 文案包（`assets/text/zh-CN.json`；叙事/直播文案按键取词，纯展示数据）
    pub text: &'a TextBundle,
}
