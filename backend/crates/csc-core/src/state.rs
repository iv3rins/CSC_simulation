//! 世界状态快照（Kotlin `GameState.kt` 转写）——**可序列化单一聚合根**。
//!
//! 转写差异（Rust 值模型优势）：Kotlin GameState 因对象引用限制只收敛 Engine
//! 顶层状态 + 时钟 + RNG + 决策日志 + 事件流 + 档案（"第 2 块聚合根"）；Rust 中
//! `World`（ID arena）、`VrsDatabase`、`TournamentResult`、`YearlyRatingTracker`
//! 均为纯值——**本结构收敛全部模拟状态**，存档/读档一次到位（docs/09 阶段 2）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use csc_career::archive::SeasonRecord;
use csc_decision::point::{DecisionPoint, PlayerDecision};
use csc_entities::narrative::NarrativeProgress;
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_tournaments::scheduled::TournamentResult;
use csc_tournaments::yearly_rating::YearlyRatingTracker;
use csc_util::id::{PlayerId, TeamId};
use csc_vrs::database::VrsDatabase;

/// 旧执行草稿中的推进粒度；不是已实现的可恢复 step 协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Month,
    Day,
}

/// v10 遗留的执行草稿。AwaitingDecision 缺少真实赛事/月度 continuation，
/// 为识别旧档保留其 serde 形状，但加载时明确拒绝，不做月初重放。
/// 目前可恢复的场景选择通过 NarrativeProgress.active_scene 保存，execution=Idle。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionState {
    /// 空闲：无在本步执行。
    #[default]
    Idle,
    /// 等待玩家决策：本步尚未完成（或场景等待）。
    AwaitingDecision {
        /// 当前待决策批次号（对外身份；跨恢复单调不回退）。0 = 场景等待（无决策批次）。
        batch_id: u64,
        /// 批次发布时的下一个批次号（恢复后从该值继续，保证身份不碰撞）。
        next_batch_id: u64,
        /// 待决策点（展示/校验用；重放后应与新生成的 pending 一致）。场景等待为空。
        points: Vec<DecisionPoint>,
        /// 已就绪的决策（多批次暂停时，先前批次已答的答案；重放时回灌）。
        #[serde(default)]
        armed_decisions: Vec<PlayerDecision>,
        /// 批次日期（展示用）。
        date: Option<String>,
        /// 恢复时要推进的单位数（见 `replay_current` 决定是否含当前步）。
        remaining_steps: u32,
        /// 推进粒度（月步/日步）。
        step_kind: StepKind,
        /// 故事模式标记（True = Human 故事模式，关键选择持续等待）。
        story: bool,
        /// 恢复时是否重放当前步：
        /// - `true`（决策批次暂停）：本步被中断并已回滚到步前状态，恢复需重放当前步；
        /// - `false`（场景暂停）：本步已完整执行，恢复只推进后续步。
        #[serde(default)]
        replay_current: bool,
    },
}

impl ExecutionState {
    /// 是否正在等待决策。
    pub fn is_awaiting(&self) -> bool {
        matches!(self, ExecutionState::AwaitingDecision { .. })
    }
}

/// 赛事队伍锁的值记录（对应 `SeasonLoop.locks` 的序列化形态）。
///
/// @param event_name     赛事名（同月唯一，锁的 key）
/// @param end_day        锁的结束日（当月日号；时钟推进到该日或跨月时解锁）
/// @param start_epoch_day 锁的开始日（绝对日序表达；days_from_civil 计算，跨月语义正确）
/// @param end_epoch_day   锁的结束日（绝对日序表达；start_epoch + duration - 1）
/// @param team_ids        被锁队伍的稳定 ID 列表（恢复时直接反解，不做签名往返）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRecord {
    pub event_name: String,
    #[serde(default)]
    pub end_day: i32,
    #[serde(default)]
    pub start_epoch_day: i64,
    #[serde(default)]
    pub end_epoch_day: i64,
    pub team_ids: Vec<TeamId>,
}

/// 世界级模拟器版本标签（指纹链专用，与 LIVE 回放链的
/// `csc_simulation::live_match::SIMULATION_VERSION` 是两回事）。
/// 任何改变世界级模拟语义/RNG 消费序的改动（M4/M5/M1 这类确定性修复、
/// 文案池增删、阶段管线重排）都必须 bump 本值——指纹门禁因此产生的
/// 差异是「预期漂移」，在提交说明与 docs/SAVE-FORMAT.md 同步记录。
pub const WORLD_SIM_VERSION: u32 = 1;

/// 世界状态快照——**完整存档聚合根**（无对象引用、无副作用、serde 派生）。
///
/// **格式版本契约**：`version` 是存档格式的演进口。任何字段增删/语义变更必须
/// bump [`GameState::CURRENT_VERSION`] 并在 [`GameState::migrate`] 提供旧版迁移；
/// 未知版本拒绝加载（`from_json_str` 显式校验），未知字段拒绝解析
/// （`deny_unknown_fields`——格式漂移静默默认化是存档损坏的温床）。
/// 版本演进集中记录见 `docs/SAVE-FORMAT.md`（bump 必须同步该文件）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameState {
    /// 存档格式版本（迁移挂载点；当前 = [`GameState::CURRENT_VERSION`]）
    pub version: u32,
    /// 赛季月份计数（0 起，每次 `Engine::advance_month` +1）
    pub month: i32,
    /// 上次合同结转的年份（跨年检测锚点）
    pub last_contract_year: i32,
    /// 本月进行中赛事的队伍级锁（值记录：队伍以稳定 ID 标识）
    pub locks: Vec<LockRecord>,
    /// 模拟时钟日期（结构化三字段便于序列化与恢复）
    pub sim_year: i32,
    pub sim_month: u32,
    pub sim_day: u32,
    /// 确定性 RNG 状态（4×u64；恢复后模拟可无缝重放）
    pub rng_state: [u64; 4],
    /// 决策日志（全部 PlayerDecision，按时间顺序）
    pub decisions: Vec<PlayerDecision>,
    /// 世界事件流（含 seq 游标，增量拉取用）
    pub journal: Vec<WorldEvent>,
    /// 生涯档案（PlayerId → 逐赛季记录）
    pub archive: HashMap<PlayerId, Vec<SeasonRecord>>,
    /// 实体 arena（选手/队伍/自由市场——完整存档）
    pub world: World,
    /// VRS 数据库（排名/积分/比赛历史——完整存档）
    pub vrs: VrsDatabase,
    /// 已完赛的全部赛事结果
    pub events: Vec<TournamentResult>,
    /// 年度 Rating 累计器（TOP20 结算的跨月状态）
    pub yearly_rating: YearlyRatingTracker,
    /// 历届年度 TOP20 榜单快照（含 NPC 名次与入选依据——历史榜单复盘；
    /// `serde(default)`：v2 旧存档读入时补空列表，向后兼容）
    #[serde(default)]
    pub top20_history: Vec<csc_tournaments::top20::Top20YearBoard>,
    /// 已排程确认赛事的权威记录（P1-1 future 赛程事实源；`serde(default)`：
    /// v8 旧存档读入补空列表，向后兼容，不破坏确定性内核）
    #[serde(default)]
    pub scheduled_records: Vec<csc_tournaments::scheduled::ScheduledTournamentRecord>,
    /// 世界级模拟器版本标签（指纹门禁组成项；`Engine::snapshot` 写入、
    /// `Engine::restore` 忽略；旧存档 `serde(default)` 补 1，不破坏向后兼容）
    #[serde(default = "default_sim_version")]
    pub sim_version: u32,
    /// 剧情进度（弧进度/承诺/flags/冷却/已见场景/终态；v10 新增）。
    /// `serde(default)`：v9 旧档读入补最小进度（不追加未发生剧情）。
    #[serde(default)]
    pub narrative: NarrativeProgress,
    /// 可序列化执行状态（持久暂停；v10 新增）。`serde(default)`：旧档读入补 `Idle`。
    #[serde(default)]
    pub execution: ExecutionState,
}

/// 世界级模拟器版本标签的 serde 默认值（旧存档读入补 1）。
fn default_sim_version() -> u32 {
    WORLD_SIM_VERSION
}

impl GameState {
    /// 当前存档格式版本。
    ///
    /// v8（2026 主动操作）：`CareerInfo.match_style / public_stance`
    /// ——`serde(default)`，旧档读入自动补 None。
    /// v7（2026 资金运用）：`PlayerFinance.skins / last_invest_year`
    /// ——`serde(default)`，旧档读入自动补空/0。
    /// v6（2026 赛季目标）：`CareerInfo.season_goal / season_goal_year`
    /// ——`serde(default)`，旧档读入自动补 None/0。
    /// v5（2026 TOP20 荣誉量纲校准）：`YearlyRatingTracker.honor_points` 从
    /// MVP=10/EVP=5 绝对分迁移为 Rating 等值单位（MVP=1.0/EVP≤0.5），
    /// 读入 v1–v4 旧档时 ×0.1 归一。
    /// v4（2026 转会市场主动接触）：`CareerInfo.transfer_contacts`（队伍招募情报）
    /// ——`serde(default)`，旧存档读入时自动补空，向后兼容。
    /// v3（主动训练）为历史格式：`pending_training` 已于 v3 引入。
    /// v9（2026 结构整理）：`scheduled_records`（future 赛程权威记录）与
    /// `sim_version`（世界级模拟器指纹版本标签）落地为存档字段
    /// ——均 `serde(default)`，旧档读入自动补空/1，迁移 = 无操作（向后兼容）。
    /// v10（2026 文字生涯）：`narrative`（剧情进度/承诺/flags/冷却/已见场景/终态）
    /// 与 `execution`（执行草稿；并不具备真实持久暂停）落地
    /// ——均 `serde(default)`，旧档读入补最小 `NarrativeProgress` / `ExecutionState::Idle`，
    /// **不给旧档追加未发生的过去剧情**（23 §4.3），迁移 = 无操作（向后兼容）。
    /// v11：叙事分支/关闭弧/结构化选择历史；拒绝无法恢复的 v10 执行草稿。
    pub const CURRENT_VERSION: u32 = 11;

    /// 不把旧步前快照当作真实暂停点；显式拒绝，保持调用方原局不变。
    pub fn validate_execution(&self) -> Result<(), csc_util::SimError> {
        if !matches!(self.execution, ExecutionState::Idle) {
            return Err(csc_util::SimError::save(
                "此存档含旧版执行暂停草稿，缺少真实 continuation，无法安全恢复；请使用空闲存档",
            ));
        }
        Ok(())
    }

    /// 从 JSON 解析存档（外部输入边界，**纯解析不做迁移**）：拒绝未知版本
    /// （> CURRENT）、拒绝未知字段；版本不符/格式漂移返回 `Err(SimError::Save)`。
    ///
    /// 本方法只做「解析 + 版本门禁」，**不执行旧档迁移**（v1-v4 荣誉积分 ×0.1 归一等
    /// 语义变更见 [`Self::migrate_state`]）。需要「解析即迁移」的宿主一律走 [`Self::migrate`]
    /// ——所有外部读档入口必须收敛到带迁移的入口，否则旧档会带着旧版本号/旧量纲进入引擎
    /// （P1-3 收敛）。
    pub fn from_json_str(json: &str) -> Result<Self, csc_util::SimError> {
        let state: GameState = serde_json::from_str(json)
            .map_err(|e| csc_util::SimError::save(format!("存档解析失败：{e}")))?;
        if state.version > Self::CURRENT_VERSION {
            return Err(csc_util::SimError::save(format!(
                "存档版本不兼容：文件 v{}，当前支持 v{}（需迁移）",
                state.version,
                Self::CURRENT_VERSION
            )));
        }
        state.validate_execution()?;
        // v1/v2/v3/v4 → v5 → v6：
        // - 新字段由 serde(default) 补默认值
        //   （months_unsigned / rating_all / maps_all / pending_training / transfer_contacts）
        // - 旧荣誉积分（MVP=10/EVP=5 绝对分）→ Rating 等值单位 ×0.1
        Ok(state)
    }

    /// 旧版本存档迁移：读旧版 → 转换 → 写新版。
    pub fn migrate(json: &str) -> Result<Self, csc_util::SimError> {
        Ok(Self::from_json_str(json)?.migrate_state())
    }

    /// 已解析存档的就地迁移（服务端 `/load` 收到反序列化后的对象时调用）。
    pub fn migrate_state(mut self) -> Self {
        if self.version < Self::CURRENT_VERSION {
            if self.version < 5 {
                // v5：TOP20 荣誉量纲从绝对分（10/5）校准为 Rating 等值单位（1.0/0.5）
                self.yearly_rating.rescale_honour_points(0.1);
            }
            self.version = Self::CURRENT_VERSION;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_replay_pause_is_rejected_instead_of_reexecuting_month() {
        let mut state = minimal_state();
        state.version = 10;
        state.execution = ExecutionState::AwaitingDecision {
            batch_id: 1,
            next_batch_id: 2,
            points: vec![],
            armed_decisions: vec![],
            date: None,
            remaining_steps: 1,
            step_kind: StepKind::Month,
            story: true,
            replay_current: true,
        };
        assert!(GameState::migrate(&serde_json::to_string(&state).unwrap()).is_err());
        state.execution = ExecutionState::Idle;
        let restored = GameState::migrate(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.version, GameState::CURRENT_VERSION);
    }

    /// 最小合法 v1 存档（结构用代码构造，避免手写 JSON 与实际 serde 形状漂移）。
    fn minimal_state() -> GameState {
        GameState {
            version: GameState::CURRENT_VERSION,
            month: 0,
            last_contract_year: 2026,
            locks: vec![],
            sim_year: 2026,
            sim_month: 1,
            sim_day: 1,
            rng_state: [0, 0, 0, 0],
            decisions: vec![],
            journal: vec![],
            archive: HashMap::new(),
            world: World::new(),
            vrs: VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: YearlyRatingTracker::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: WORLD_SIM_VERSION,
            narrative: NarrativeProgress::default(),
            execution: ExecutionState::Idle,
        }
    }

    #[test]
    fn lock_record_serde() {
        let l = LockRecord {
            event_name: "T1 Monthly #1".into(),
            end_day: 6,
            start_epoch_day: 10,
            end_epoch_day: 17,
            team_ids: vec![TeamId(3)],
        };
        let json = serde_json::to_string(&l).unwrap();
        let back: LockRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }

    #[test]
    fn version_mismatch_rejected() {
        let state = minimal_state();
        let mut json = serde_json::to_string(&state).unwrap();
        json = json.replacen(
            &format!("\"version\":{}", GameState::CURRENT_VERSION),
            "\"version\":99",
            1,
        );
        assert!(
            GameState::from_json_str(&json).is_err(),
            "未知版本必须拒绝加载"
        );
    }

    #[test]
    fn v1_save_backward_compatible() {
        // v1 存档（无 months_unsigned / rating_all / maps_all / pending_training /
        // skins / last_invest_year）→ 读入后补默认值 + migrate 归一当前版本
        let state = minimal_state();
        let mut json = serde_json::to_string(&state).unwrap();
        json = json.replacen(
            &format!("\"version\":{}", GameState::CURRENT_VERSION),
            "\"version\":1",
            1,
        );
        let loaded = GameState::from_json_str(&json).expect("v1 存档应向后兼容");
        assert_eq!(loaded.version, 1);
        assert!(loaded.yearly_rating.stats_all().is_empty());
        let migrated = GameState::migrate(&json).expect("migrate 应成功");
        assert_eq!(
            migrated.version,
            GameState::CURRENT_VERSION,
            "migrate 归一版本号"
        );
    }

    /// v8→v9 升级契约：v9 引入 `scheduled_records` / `sim_version` 两个字段，
    /// 都是 `serde(default)`。真正 v8 的旧档（无这两个字段）读入后应补默认值
    /// 并升到 `CURRENT_VERSION`——迁移 = 无操作。
    #[test]
    fn v8_save_upgrades_to_v9_and_backfills_new_fields() {
        let state = minimal_state();
        let mut json = serde_json::to_string(&state).unwrap();
        // 构造一个真实的 v8 档：字段存在但版本号为 8（v8 时代这两个字段不存在）
        json = json.replacen(
            &format!("\"version\":{}", GameState::CURRENT_VERSION),
            "\"version\":8",
            1,
        );
        // 移除此字段，模拟真实 v8 存档（无 scheduled_records / sim_version）
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut obj = value.as_object().unwrap().clone();
        obj.remove("scheduled_records");
        obj.remove("sim_version");
        let v8_json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();

        let migrated = GameState::migrate(&v8_json).expect("v8 档应可迁移");
        assert_eq!(
            migrated.version,
            GameState::CURRENT_VERSION,
            "v8 → v9 迁移归一版本号"
        );
        assert_eq!(
            migrated.sim_version, WORLD_SIM_VERSION,
            "v8 档无 sim_version → serde(default) 补 WORLD_SIM_VERSION"
        );
        assert!(
            migrated.scheduled_records.is_empty(),
            "v8 档无 scheduled_records → serde(default) 补空列表"
        );
    }

    /// v9→v10 升级契约：v10 引入 `narrative` / `execution` 两个字段，均为
    /// `serde(default)`。真正 v9 的旧档（无这两个字段）读入后应补最小进度与 `Idle`，
    /// 并升到 `CURRENT_VERSION`——**不给旧档追加未发生的过去剧情**（迁移 = 无操作）。
    #[test]
    fn v9_save_upgrades_to_v10_and_backfills_narrative_and_execution() {
        let state = minimal_state();
        let mut json = serde_json::to_string(&state).unwrap();
        json = json.replacen(
            &format!("\"version\":{}", GameState::CURRENT_VERSION),
            "\"version\":9",
            1,
        );
        // 构造真实 v9 档：移除 v10 新增字段。
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut obj = value.as_object().unwrap().clone();
        obj.remove("narrative");
        obj.remove("execution");
        let v9_json = serde_json::to_string(&serde_json::Value::Object(obj)).unwrap();

        let migrated = GameState::migrate(&v9_json).expect("v9 档应可迁移");
        assert_eq!(
            migrated.version,
            GameState::CURRENT_VERSION,
            "v9 → v10 迁移归一版本号"
        );
        assert!(
            migrated.narrative.seen_scenes.is_empty(),
            "v9 档无 narrative → serde(default) 补空进度（不追加过去剧情）"
        );
        assert_eq!(
            migrated.execution,
            ExecutionState::Idle,
            "v9 档无 execution → serde(default) 补 Idle"
        );
    }

    #[test]
    fn v4_honor_points_rescaled_on_migrate() {
        // v4 旧档：MVP 荣誉积分以绝对分 10.0 存储 → v5 迁移后 1.0（Rating 等值单位）
        let mut tracker = YearlyRatingTracker::default();
        tracker.record_honour(PlayerId(1), true, 10.0);
        let mut state = minimal_state();
        state.version = 4;
        state.yearly_rating = tracker;
        let migrated = state.migrate_state();
        assert_eq!(migrated.version, GameState::CURRENT_VERSION);
        assert_eq!(
            migrated.yearly_rating.honour_of(PlayerId(1)),
            Some((1, 0, 1.0)),
            "旧荣誉绝对分应 ×0.1 归一为 Rating 等值单位"
        );
    }

    #[test]
    fn unknown_field_rejected() {
        let state = minimal_state();
        let mut json = serde_json::to_string(&state).unwrap();
        assert!(
            GameState::from_json_str(&json).is_ok(),
            "合法 v1 存档应可解析"
        );
        // 注入未知字段 → deny_unknown_fields 拒绝（防格式漂移静默默认化）
        json.insert_str(1, "\"ghost_field\":1,");
        assert!(
            GameState::from_json_str(&json).is_err(),
            "未知字段必须拒绝（防格式漂移静默）"
        );
    }
}
