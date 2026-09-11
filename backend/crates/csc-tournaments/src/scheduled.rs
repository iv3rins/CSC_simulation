//! 已排定赛事 + 赛事结果（Kotlin `ScheduledTournament.kt` + `TournamentResult.kt` 转写）。

use csc_domain::city::{City, Region};
use csc_domain::event_importance::EventImportance;
use csc_domain::match_result::MatchResult;
use csc_domain::tier_profile::{tier_importance, tier_venue};
use csc_domain::tournament::{InvitePolicy, Organizer, Tournament};
use csc_domain::tournament_format::TournamentFormat;
use csc_domain::tourney_tier::TourneyTier;
use csc_simulation::series::SeriesResult;
use csc_util::id::TeamId;

/// 一次已排定的赛事及其参赛名单（队伍 ID）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScheduledTournament {
    pub tier: TourneyTier,
    /// 玩家体验层；世界仍完整模拟，只有 LIVE 赛事产生现场决策。
    #[serde(default)]
    pub importance: EventImportance,
    /// 直邀 / 直接参赛的队伍
    pub direct_invitees: Vec<TeamId>,
    /// 拒绝直邀的队伍（信息用途）
    pub replacements: Vec<TeamId>,
    /// 公开预选赛 / 其他通道进入的队伍
    pub qualifiers: Vec<TeamId>,
    /// 赛事日期（模拟时钟写入，如「2026-06-08」）
    pub event_date: String,
    /// 对抗结构赛制（随赛事配置透传）
    pub format: TournamentFormat,
}

impl ScheduledTournament {
    /// 全部参赛队伍（直邀 + 预选）。
    pub fn all_participants(&self) -> Vec<TeamId> {
        let mut v = self.direct_invitees.clone();
        v.extend(self.qualifiers.iter().cloned());
        v
    }

    /// 折算为赛事描述（排程阶段的轻量 Tournament；= Kotlin `toEvent`）。
    pub fn to_event(&self) -> Tournament {
        self.build_event(format!("{} Event", self.tier.name()))
    }

    /// 以**真实赛事名**折算赛前/完赛存档用的轻量 `Tournament`（R2 修复）。
    ///
    /// 此前 `to_event` 用占位名 `"{tier} Event"`（如 `QUALIFY Event`）写入
    /// `attended_tournaments` 与 `TournamentResult.event`，而排程/页面按真实名
    /// （`Exort Fiesta Series #1-8` 等）查询 → **按名关联永远命中不了**，
    /// 表现为「本队完赛记录 0 场 / 无比分 / 冠军列显示 WW 占位」。
    /// 本方法从 `ScheduledTournamentRecord.event_name` 传入真实名，保证赛事
    /// 聚合根在排程与结算两阶段名字一致。
    pub fn to_event_named(&self, name: impl Into<String>) -> Tournament {
        self.build_event(name.into())
    }

    fn build_event(&self, name: String) -> Tournament {
        Tournament::new(
            name,
            self.tier,
            Organizer::Other,
            City::new("TBD", "TBD", Region::Other),
            1,
            match self.tier {
                TourneyTier::Major
                | TourneyTier::SuperElite
                | TourneyTier::Elite
                | TourneyTier::T1 => InvitePolicy::VrsGlobal,
                TourneyTier::T2 => InvitePolicy::Qualifier,
                TourneyTier::Qualify => InvitePolicy::Open,
            },
        )
        .with_team_slots(self.all_participants().len() as i32)
        .with_direct_invites(self.direct_invitees.len() as i32)
        .with_open_qualifier_slots(self.qualifiers.len() as i32)
        .with_venue(tier_venue(self.tier)) // 供展示；运行时由 MatchResult 按 tier 推导
        .with_date(self.event_date.clone())
        .with_importance(tier_importance(self.tier))
        .with_format(self.format)
    }
}

/// 排程记录的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduledStatus {
    /// 已排程确认、尚未开始模拟（待开赛）。
    Future,
    /// 正在进行中。
    Active,
    /// 已完整结束。
    Completed,
    /// 参赛名单不足被取消（不产生对局/冠军/奖金）。
    Cancelled,
}

/// 一次已排定确认的赛事权威记录（P1-1 future 赛程事实源）。
///
/// 与 [`ScheduledTournament`] 不同，本记录进入 `GameState` 存档：
/// 它捕获**排程确认时点**的参赛名单，作为玩家/前端「我的赛事」的权威
/// 事实来源，而非依赖 `attended_tournaments` 的隐式写入时点。
/// 生命周期由 [`crate::engine::TournamentEngine`] 维护。
///
/// `fixtures` 承载**赛前物化的首轮对阵**（T3：赛事/赛程分离）：
/// 排程确认时点由参赛名单 + 赛制生成（纯函数、确定性），开赛前即可展示；
/// 赛事完整模拟后按进行顺序回填 `result`（原地更新，pending → 带比分）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScheduledTournamentRecord {
    /// 赛事名（同月唯一，对应 `GameState.locks` 的 event_name）。
    pub event_name: String,
    /// 赛事日期（排程确认/开赛日，形如「2026-06-08」）。
    pub date: String,
    /// 赛事等级。
    pub tier: TourneyTier,
    /// 实际参赛队伍（直邀 + 预选）。
    pub team_ids: Vec<TeamId>,
    /// 主角队伍 ID（若主角参赛）；否则 `None`。
    #[serde(default)]
    pub player_team_id: Option<TeamId>,
    /// 生命周期状态。
    pub status: ScheduledStatus,
    /// 赛前物化的首轮对阵（T3；旧存档缺失时为空，前端回退为无对阵展示）。
    #[serde(default)]
    pub fixtures: Vec<ScheduledFixture>,
    /// 赛事赛制（排程确认时点捕获；两阶段拆解后结算阶段据此重构 `ScheduledTournament`
    /// 进行确定性模拟。旧存档缺失时默认 `SingleElim`——旧档该记录必然已 Completed，
    /// 不会进入结算，默认值仅作形状兼容）。
    #[serde(default = "default_format")]
    pub format: TournamentFormat,
}

/// `ScheduledTournamentRecord.format` 的 serde 默认（旧存档兼容；仅在旧档含未完结记录时
/// 才可能被读取，而旧档记录恒 Completed，故取 `SingleElim` 即可安全兜底）。
fn default_format() -> TournamentFormat {
    TournamentFormat::SingleElim
}

/// 一场赛前物化的对阵（fixture）。
///
/// `result == None` 表示待开赛（pending，无比分）；赛事模拟完成后
/// 按队伍 ID 匹配回填 `result`（`Some(MatchResult)`）。
/// `team_b == TeamId::NONE` 表示轮空（BYE，无比赛）。
/// `home`/`away` 语义：`team_a` 为主队（home）、`team_b` 为客队（away）。
/// `fixture_id` 为赛事内稳定标识（同 seed 排程确定不变），`round` 为轮次（1 基）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScheduledFixture {
    /// 赛事内稳定对阵 ID（0 基递增；同 seed 排程确定不变）。
    #[serde(default)]
    pub fixture_id: u32,
    /// 轮次（1 基首轮）。
    #[serde(default)]
    pub round: u32,
    /// 对阵主队（种子序靠前）。
    pub team_a: TeamId,
    /// 对阵客队；`TeamId::NONE` = 轮空（无比赛）。
    pub team_b: TeamId,
    /// 阶段（小组赛/淘汰赛等；随赛制分派）。
    pub stage: csc_simulation::series::SeriesStage,
    /// 本场赛制（BO1/BO3/BO5）。
    pub best_of: i32,
    /// 对阵状态（pending/completed/bye）。
    #[serde(default)]
    pub status: FixtureStatus,
    /// 完赛结果（pending 时为 None）。
    #[serde(default)]
    pub result: Option<MatchResult>,
    /// R2：对局比分（HLTV `13:12` / 系列 `2:0` 样式；pending 时为 None）。
    ///
    /// 完赛回填时从系列赛逐图比分取**决定性图**的回合比分（BO1）或系列胜场数
    /// （BO3/BO5），以 fixture 的 team_a/team_b 方向表达。纯展示字段，不影响结算。
    #[serde(default)]
    pub score: Option<FixtureScore>,
    /// 玩家是否已对本场**跳过/关闭**比赛日 LIVE 提示（`skip` 语义标记）。
    ///
    /// 仅改变 `live/today` 闸门的可观测性（跳过的对阵不再反复弹出提示），
    /// **不改变比赛结算**：赛事仍在月末 `settle_month` 走确定性后台模拟并回填
    /// `result`（跳过的比赛照常结算出比分），杜绝「跳过即丢比赛结果」。
    #[serde(default)]
    pub skipped: bool,
    /// R2：玩家是否已**看完**本场回放/LIVE（`watched` 语义标记）。
    ///
    /// 看完后前端不再重复弹「观看回放」入口；纯展示语义，不改变结算。
    /// 与 `skipped` 正交（可先跳过提示、后补看回放，或先看完再跳过）。
    #[serde(default)]
    pub watched: bool,
}

/// 对阵生命周期状态。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FixtureStatus {
    /// 待开赛（无比分）。默认值。
    #[default]
    Pending,
    /// 已完赛（result 已回填）。
    Completed,
    /// 轮空（无比赛，永不回填）。
    Bye,
}

/// 对局比分（HLTV 样式）：以 fixture 的 team_a/team_b 方向表达。
/// `best_of == 1` 时 = 单图回合比分（如 13:12）；`best_of > 1` 时 = 系列胜场数（如 2:0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FixtureScore {
    pub team_a_score: i32,
    pub team_b_score: i32,
}

impl ScheduledFixture {
    /// 构造一场 pending 对阵（无比分；fixture_id/round 自动递增）。
    pub fn pending(
        fixture_id: u32,
        round: u32,
        team_a: TeamId,
        team_b: TeamId,
        stage: csc_simulation::series::SeriesStage,
        best_of: i32,
    ) -> Self {
        Self {
            fixture_id,
            round,
            team_a,
            team_b,
            stage,
            best_of,
            status: FixtureStatus::Pending,
            result: None,
            score: None,
            skipped: false,
            watched: false,
        }
    }

    /// 轮空占位（`team_b = TeamId::NONE`；无比赛、永不回填 result）。
    pub fn bye(
        fixture_id: u32,
        round: u32,
        team: TeamId,
        stage: csc_simulation::series::SeriesStage,
    ) -> Self {
        Self {
            fixture_id,
            round,
            team_a: team,
            team_b: TeamId::NONE,
            stage,
            best_of: 0,
            status: FixtureStatus::Bye,
            result: None,
            score: None,
            skipped: false,
            watched: false,
        }
    }

    /// 是否轮空（无比赛）。
    pub fn is_bye(&self) -> bool {
        self.team_b == TeamId::NONE
    }

    /// 主队（home）。
    pub fn home(&self) -> TeamId {
        self.team_a
    }

    /// 客队（away；轮空时为 `TeamId::NONE`）。
    pub fn away(&self) -> TeamId {
        self.team_b
    }
}

/// 赛前物化的首轮对阵（T3：赛事/赛程分离）。
///
/// 纯函数、确定性：**不消费 RNG**，只依据参赛名单（种子序）与赛制生成
/// 首轮配对——赛事开打前即可展示「打什么对手」。
///
/// 配对规则与赛制引擎的**运行时首轮配对语义完全一致**（P1 修复：
/// 预览必须=实际，否则赛后按队伍匹配回填会大面积落空）：
/// - [`TournamentFormat::SingleElim`]：标准种子 bracket 落位（1-8/4-5/2-7/3-6），
///   非 2 的幂补轮空（弱种子轮空）；`tier_best_of` 为常规轮赛制。
/// - [`TournamentFormat::SwissPlayoff`]：瑞士轮首轮按种子序两两配对
///   （1v2、3v4…；奇数轮空计一胜但无比赛），赛制 `swiss_best_of`。
/// - [`TournamentFormat::DoubleElimGroups`]：**蛇形分组**（snake seeding，
///   与 `DoubleElimGroupsBracket` 运行时一致）后组内种子配对（1v4、2v3）；
///   参赛队数非 4 的倍数时**降级单败淘汰**（与运行时 `DoubleElimGroupsBracket`
///   的弹性降级一致），按标准种子 bracket 落位。
///
/// 注：引擎实际运行时 SingleElim 会额外随机打乱（RNG 消费在闭包创建前），
/// 但首轮配对已按本预览落位（`legacy_single_elim` 消费 fixtures）——
/// 因此**运行时首轮对阵与预览一致**，赛后回填 100% 命中。
pub fn first_round_fixtures(
    format: TournamentFormat,
    participants: &[TeamId],
    tier: TourneyTier,
) -> Vec<ScheduledFixture> {
    use csc_simulation::series::SeriesStage;

    // 空参赛名单（排程取消路径）：不产生任何对阵（与运行时取消语义一致）。
    if participants.is_empty() {
        return Vec::new();
    }

    match format {
        TournamentFormat::SingleElim => {
            // 标准种子 bracket 落位（1,8,4,5,2,7,3,6…），再两两配对。
            single_elim_bracket_fixtures(participants, tier)
        }
        TournamentFormat::SwissPlayoff { swiss_best_of, .. } => {
            pair_seed_order(participants, SeriesStage::Group, swiss_best_of)
        }
        TournamentFormat::DoubleElimGroups { group_best_of, .. } => {
            // P1 修复：与运行时 `DoubleElimGroupsBracket` 完全同语义——
            // 1) 非 4 的倍数 → 弹性降级单败（bracket_order 种子落位）；
            // 2) 4 的倍数 → 蛇形分组（snake seeding），组内种子配对 [1v4, 2v3]。
            if !participants.len().is_multiple_of(DOUBLE_ELIM_GROUP_SIZE) {
                single_elim_bracket_fixtures(participants, tier)
            } else {
                let groups =
                    snake_seed_ids(participants, participants.len() / DOUBLE_ELIM_GROUP_SIZE);
                let mut out = Vec::new();
                for g in groups {
                    if g.len() >= 4 {
                        out.push(ScheduledFixture::pending(
                            out.len() as u32,
                            1,
                            g[0],
                            g[3],
                            SeriesStage::Group,
                            group_best_of,
                        ));
                        out.push(ScheduledFixture::pending(
                            out.len() as u32,
                            1,
                            g[1],
                            g[2],
                            SeriesStage::Group,
                            group_best_of,
                        ));
                    } else if g.len() == 3 {
                        out.push(ScheduledFixture::pending(
                            out.len() as u32,
                            1,
                            g[0],
                            g[2],
                            SeriesStage::Group,
                            group_best_of,
                        ));
                    } else if g.len() == 2 {
                        out.push(ScheduledFixture::pending(
                            out.len() as u32,
                            1,
                            g[0],
                            g[1],
                            SeriesStage::Group,
                            group_best_of,
                        ));
                    }
                }
                out
            }
        }
    }
}

/// 双败小组的固定规模（= `DoubleElimGroup::GROUP_SIZE`，运行时弹性赛制判断用）。
const DOUBLE_ELIM_GROUP_SIZE: usize = 4;

/// 标准种子单败 bracket 的赛前物化首轮对阵（奇偶轮空；与
/// `legacy_single_elim`/`SingleElimPlayoff` 的 `bracket_order` 落位一致）：
/// N 为奇数时最强种子（种子 1）轮空直接晋级，其余两两配对；
/// N 为偶数时全配对。首轮真赛 = floor(N/2)，轮空 = N%2。
fn single_elim_bracket_fixtures(
    participants: &[TeamId],
    tier: TourneyTier,
) -> Vec<ScheduledFixture> {
    use csc_simulation::series::SeriesStage;
    let entries: Vec<TeamId> = bracket_order_ids(participants);
    let best_of = csc_domain::tier_profile::tier_best_of(tier);
    let mut out = Vec::new();
    let mut i = 0;
    // 奇数队伍数：最强种子（种子 1 = entries[0]）轮空直接晋级。
    if entries.len() % 2 == 1 {
        out.push(ScheduledFixture::bye(
            out.len() as u32,
            1,
            entries[0],
            SeriesStage::Playoff,
        ));
        i = 1;
    }
    while i < entries.len() {
        let a = entries[i];
        let b = entries.get(i + 1).copied().unwrap_or(TeamId::NONE);
        if b == TeamId::NONE {
            out.push(ScheduledFixture::bye(
                out.len() as u32,
                1,
                a,
                SeriesStage::Playoff,
            ));
        } else {
            out.push(ScheduledFixture::pending(
                out.len() as u32,
                1,
                a,
                b,
                SeriesStage::Playoff,
                best_of,
            ));
        }
        i += 2;
    }
    out
}

/// 蛇形分组（snake seeding）：把按种子排序的队伍分进 `group_count` 个实力均衡
/// 的小组——与运行时 `BracketSeeding::snake_seed` 同语义（TeamId 版）。
fn snake_seed_ids(participants: &[TeamId], group_count: usize) -> Vec<Vec<TeamId>> {
    assert!(group_count > 0, "分组数必须为正");
    if participants.is_empty() {
        return vec![Vec::new(); group_count];
    }
    let mut groups: Vec<Vec<TeamId>> = vec![Vec::new(); group_count];
    let mut forward = true;
    for (index, team) in participants.iter().enumerate() {
        let group_index = if forward {
            index % group_count
        } else {
            group_count - 1 - index % group_count
        };
        groups[group_index].push(*team);
        if (index + 1) % group_count == 0 {
            forward = !forward;
        }
    }
    groups
}

/// 种子序两两配对（Swiss 首轮语义：1v2、3v4…；奇数末位轮空）。
fn pair_seed_order(
    participants: &[TeamId],
    stage: csc_simulation::series::SeriesStage,
    best_of: i32,
) -> Vec<ScheduledFixture> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < participants.len() {
        let a = participants[i];
        let b = participants.get(i + 1).copied().unwrap_or(TeamId::NONE);
        if b == TeamId::NONE {
            out.push(ScheduledFixture::bye(out.len() as u32, 1, a, stage));
        } else {
            out.push(ScheduledFixture::pending(
                out.len() as u32,
                1,
                a,
                b,
                stage,
                best_of,
            ));
        }
        i += 2;
    }
    out
}

/// 标准种子 bracket 下标（1 基）→ 队伍 ID 序列（= `BracketSeeding::bracket_order`
/// 的 ID 版）：递归镜像序列截断到 N 队，**不补 NONE 轮空**（奇偶轮空由
/// `single_elim_bracket_fixtures` 与运行时 `SingleElimPlayoff::run` 逐轮处理）。
fn bracket_order_ids(participants: &[TeamId]) -> Vec<TeamId> {
    let mut size = 1;
    while size < participants.len() {
        size *= 2;
    }
    let mut order = vec![1usize];
    while order.len() < size {
        let new_size = order.len() * 2;
        order = order
            .iter()
            .flat_map(|i| vec![*i, new_size + 1 - *i])
            .collect();
    }
    order
        .into_iter()
        .filter(|&i| i <= participants.len())
        .map(|i| participants[i - 1])
        .collect()
}

/// 一场完整赛事的模拟结果：赛事描述 → 全部系列赛 → 冠军。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TournamentResult {
    /// 赛事描述（含 tier / organizer / city / vrsWeight 等元数据）
    pub event: Tournament,
    /// 冠军队伍 ID
    pub champion: TeamId,
    /// 全部系列赛折算为 MatchResult（按进行顺序）
    pub matches: Vec<MatchResult>,
    /// 全部系列赛完整结果（逐图比分 + 全员 KDA）
    pub series: Vec<SeriesResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participants_and_to_event() {
        let s = ScheduledTournament {
            tier: TourneyTier::T1,
            importance: EventImportance::Important,
            direct_invitees: vec![TeamId(0), TeamId(1)],
            replacements: Vec::new(),
            qualifiers: vec![TeamId(2)],
            event_date: "2026-06-08".into(),
            format: TournamentFormat::SingleElim,
        };
        assert_eq!(s.all_participants(), vec![TeamId(0), TeamId(1), TeamId(2)]);
        let event = s.to_event();
        assert_eq!(event.name, "T1 Event");
        assert_eq!(event.team_slots, 3);
        assert_eq!(event.direct_invites, 2);
        assert_eq!(event.open_qualifier_slots, 1);
        assert_eq!(event.date, "2026-06-08");
    }

    #[test]
    fn to_event_named_uses_real_name_not_placeholder() {
        // R2 修复：`to_event_named` 以真实赛事名构建 Tournament（`to_event` 保留
        // 占位名 `{tier} Event` 仅供排程阶段轻量描述），保证结算归档
        // （attended_tournaments / TournamentResult.event）与排程记录同名，
        // 前端按名关联「已完赛回放/比分/冠军」才能命中。
        let s = ScheduledTournament {
            tier: TourneyTier::Qualify,
            importance: EventImportance::Background,
            direct_invitees: vec![TeamId(0), TeamId(1)],
            replacements: Vec::new(),
            qualifiers: Vec::new(),
            event_date: "2026-02-27".into(),
            format: TournamentFormat::DoubleElimGroups {
                groups: 2,
                group_best_of: 1,
                playoff_best_of: 3,
                final_best_of: 5,
            },
        };
        // 占位名（旧行为，排程阶段仍可用）。
        assert_eq!(s.to_event().name, "QUALIFY Event");
        // 真实名（R2 修复，结算归档用）。
        let named = s.to_event_named("Exort Fiesta Series #1-8");
        assert_eq!(named.name, "Exort Fiesta Series #1-8");
        assert_eq!(named.tier, TourneyTier::Qualify);
        assert_eq!(named.date, "2026-02-27");
        assert_eq!(named.team_slots, 2);
    }

    // —— T3：赛前赛程物化（fixtures）测试 ——

    fn participants(n: usize) -> Vec<TeamId> {
        (0..n as u32).map(TeamId).collect()
    }

    fn record_with(format: TournamentFormat, n: usize) -> ScheduledTournamentRecord {
        let ids = participants(n);
        ScheduledTournamentRecord {
            event_name: "Test Cup".into(),
            date: "2026-06-08".into(),
            tier: TourneyTier::T1,
            team_ids: ids.clone(),
            player_team_id: ids.first().copied(),
            status: ScheduledStatus::Future,
            fixtures: first_round_fixtures(format, &ids, TourneyTier::T1),
            format,
        }
    }

    #[test]
    fn single_elim_fixtures_are_deterministic_bracket_order() {
        // 8 队单败：标准种子落位 1,8,4,5,2,7,3,6 → 四场 pending 对阵。
        let r1 = record_with(TournamentFormat::SingleElim, 8);
        let r2 = record_with(TournamentFormat::SingleElim, 8);
        assert_eq!(r1.fixtures, r2.fixtures, "同名单必须产出相同对阵（确定性）");
        let pairs: Vec<(TeamId, TeamId)> =
            r1.fixtures.iter().map(|f| (f.team_a, f.team_b)).collect();
        assert_eq!(
            pairs,
            vec![
                (TeamId(0), TeamId(7)),
                (TeamId(3), TeamId(4)),
                (TeamId(1), TeamId(6)),
                (TeamId(2), TeamId(5)),
            ],
            "标准种子 bracket：1v8, 4v5, 2v7, 3v6"
        );
        assert!(
            r1.fixtures.iter().all(|f| f.result.is_none()),
            "赛前全部 pending（无比分）"
        );
        assert!(r1.fixtures.iter().all(|f| !f.is_bye()));
        // T3 字段规格：fixture_id 稳定递增、round=1、status=pending（home/away 语义）。
        for (i, f) in r1.fixtures.iter().enumerate() {
            assert_eq!(f.fixture_id, i as u32, "fixture_id 0 基递增");
            assert_eq!(f.round, 1, "首轮 round=1");
            assert_eq!(f.status, FixtureStatus::Pending, "赛前全部 pending 状态");
            assert_eq!(f.home(), f.team_a, "home = team_a");
            assert_eq!(f.away(), f.team_b, "away = team_b");
        }
    }

    #[test]
    fn single_elim_bye_fixture_for_non_power_of_two() {
        // 5 队单败（奇偶轮空）：bracket_order 截断到 5 → [1,4,2,5,3]
        // （标准序列 1,8,4,5,2,7,3,6 中下标 ≤5 者）；奇数队伍数 → 最强种子
        // （种子 1 = T0）轮空直接晋级，其余两两配对：floor(5/2)=2 真赛。
        let r = record_with(TournamentFormat::SingleElim, 5);
        let real: Vec<_> = r.fixtures.iter().filter(|f| !f.is_bye()).collect();
        let byes: Vec<_> = r.fixtures.iter().filter(|f| f.is_bye()).collect();
        assert_eq!(real.len(), 2, "5 队首轮 2 场真赛（floor(5/2)=2）");
        assert_eq!(byes.len(), 1, "5 队首轮 1 个轮空（最强种子 T0）");
        assert!(byes.iter().all(|f| f.result.is_none()));
        assert!(
            byes.iter().all(|f| f.status == FixtureStatus::Bye),
            "轮空状态 bye"
        );
        assert!(byes.iter().all(|f| f.is_bye()));
        // 轮空给最强种子（种子 1 = T0）。
        assert_eq!(byes[0].team_a, TeamId(0));
        assert_eq!(byes[0].team_b, TeamId::NONE);
        // 真赛配对：bracket_order(5) = [T0,T3,T4,T1,T2]（标准序列 1,8,4,5,2,7,3,6
        // 截断到 ≤5 → 1,4,5,2,3）→ 轮空 T0 后两两：(T3,T4) (T1,T2)。
        let pairs: std::collections::HashSet<(u32, u32)> = real
            .iter()
            .map(|f| (f.team_a.0.min(f.team_b.0), f.team_a.0.max(f.team_b.0)))
            .collect();
        assert_eq!(
            pairs,
            std::collections::HashSet::from([(3, 4), (1, 2)]),
            "5 队真赛配对 (T3,T4) (T1,T2)"
        );
        assert!(
            real.iter().all(|f| f.status == FixtureStatus::Pending),
            "真赛 pending"
        );
    }

    #[test]
    fn swiss_fixtures_pair_seed_order() {
        // Swiss 首轮：种子序两两（1v2, 3v4…），奇数末位轮空。
        let r = record_with(TournamentFormat::swiss_playoff(), 9);
        let pairs: Vec<(TeamId, TeamId)> =
            r.fixtures.iter().map(|f| (f.team_a, f.team_b)).collect();
        assert_eq!(
            pairs,
            vec![
                (TeamId(0), TeamId(1)),
                (TeamId(2), TeamId(3)),
                (TeamId(4), TeamId(5)),
                (TeamId(6), TeamId(7)),
                (TeamId(8), TeamId::NONE),
            ],
            "Swiss 首轮按种子序两两配对，奇数轮空"
        );
        assert_eq!(r.fixtures[0].best_of, 1, "Swiss 默认 BO1");
        assert_eq!(
            r.fixtures[0].stage,
            csc_simulation::series::SeriesStage::Group
        );
    }

    #[test]
    fn double_elim_fixtures_pair_within_group() {
        // 4 队单组双败：组内 1v4, 2v3（蛇形分组 1 组 = 顺序）。
        let r = record_with(TournamentFormat::double_elim_groups(), 4);
        let pairs: Vec<(TeamId, TeamId)> =
            r.fixtures.iter().map(|f| (f.team_a, f.team_b)).collect();
        assert_eq!(
            pairs,
            vec![(TeamId(0), TeamId(3)), (TeamId(1), TeamId(2))],
            "双败组内种子配对：1v4, 2v3"
        );
    }

    #[test]
    fn double_elim_fixtures_use_snake_seed_like_runtime() {
        // P1 修复：预览必须与运行时 `DoubleElimGroupsBracket`（snake_seed 分组）
        // 完全一致，否则赛后回填大面积落空。8 队 2 组蛇形：
        //   组1 = [1,4,5,8] → (1,8) (4,5)；组2 = [2,3,6,7] → (2,7) (3,6)
        let r = record_with(TournamentFormat::double_elim_groups(), 8);
        let pairs: Vec<(TeamId, TeamId)> =
            r.fixtures.iter().map(|f| (f.team_a, f.team_b)).collect();
        assert_eq!(
            pairs,
            vec![
                (TeamId(0), TeamId(7)),
                (TeamId(3), TeamId(4)),
                (TeamId(1), TeamId(6)),
                (TeamId(2), TeamId(5)),
            ],
            "蛇形分组：组1 = 1,4,5,8 → (1,8)(4,5)；组2 = 2,3,6,7 → (2,7)(3,6)"
        );
    }

    #[test]
    fn double_elim_fixtures_degrade_to_single_elim_for_non_multiple_of_four() {
        // P1 修复：9 队 T3（玩家队优先注入后非 4 倍数）→ 运行时
        // `DoubleElimGroupsBracket` 弹性降级单败（奇偶轮空）。
        // 预览必须与之一致：9 队 → bracket_order 截断到 9 →
        // [1,8,4,5,2,7,3,6,9]；奇数队伍数 → 最强种子（T0）轮空，
        // 其余两两配对：floor(9/2)=4 真赛。
        let r = record_with(TournamentFormat::double_elim_groups(), 9);
        let real: Vec<_> = r.fixtures.iter().filter(|f| !f.is_bye()).collect();
        assert_eq!(real.len(), 4, "9 队单败首轮 4 场真赛（floor(9/2)=4）");
        // BYE 占位：最强种子（种子 1 = T0）首轮轮空。
        let byes: Vec<_> = r.fixtures.iter().filter(|f| f.is_bye()).collect();
        assert_eq!(byes.len(), 1, "9 队首轮 1 个轮空（最强种子 T0）");
        assert!(byes.iter().all(|f| f.status == FixtureStatus::Bye));
        assert_eq!(byes[0].team_a, TeamId(0), "轮空给最强种子");
        // 真赛配对：bracket_order(9) = [T0,T7,T8,T3,T4,T1,T6,T2,T5]
        // （标准序列 1,8,4,5,2,7,3,6 与下一轮镜像 9..16 中 ≤9 者 →
        // 1,8,9,4,5,2,7,3,6 再截断）→ 轮空 T0 后两两：(T7,T8)(T3,T4)(T1,T6)(T2,T5)。
        let pairs: std::collections::HashSet<(u32, u32)> = real
            .iter()
            .map(|f| (f.team_a.0.min(f.team_b.0), f.team_a.0.max(f.team_b.0)))
            .collect();
        assert_eq!(
            pairs,
            std::collections::HashSet::from([(7, 8), (3, 4), (1, 6), (2, 5)]),
            "9 队真赛配对 (T7,T8)(T3,T4)(T1,T6)(T2,T5)"
        );
        // 覆盖全部 9 支参赛队（无队伍被预览遗漏）。
        let covered: std::collections::HashSet<u32> = r
            .fixtures
            .iter()
            .flat_map(|f| [f.team_a.0, f.team_b.0])
            .collect();
        for i in 0..9u32 {
            assert!(covered.contains(&i), "预览必须覆盖队伍 {i}");
        }
    }

    #[test]
    fn fixture_save_load_roundtrip() {
        // 存档/恢复：fixtures（含 pending 与已回填 result）序列化往返一致；
        // 旧存档缺失 fixtures 字段 → serde default 空数组（优雅降级）。
        let mut r = record_with(TournamentFormat::SingleElim, 4);
        r.fixtures[0].result = Some(MatchResult::new("TeamA", "TeamB", TourneyTier::T1));
        let json = serde_json::to_string(&r).unwrap();
        let back: ScheduledTournamentRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r, "fixtures 序列化往返必须逐位一致");
        // 旧存档（无 fixtures 字段）兼容。
        let legacy = r#"{"event_name":"Old Cup","date":"2025-01-01","tier":"T1","team_ids":[0,1],"player_team_id":0,"status":"completed"}"#;
        let old: ScheduledTournamentRecord = serde_json::from_str(legacy).unwrap();
        assert!(
            old.fixtures.is_empty(),
            "旧存档 fixtures 应为空（不 panic）"
        );
    }

    #[test]
    fn fixture_results_backfill_by_team_pair() {
        // 完成后回填：按队伍对匹配（方向无关），未匹配保持 pending。
        let mut r = record_with(TournamentFormat::SingleElim, 4);
        let m = MatchResult::new("T3", "T0", TourneyTier::T1); // 队伍 3 胜队伍 0（反向也匹配）
        // 模拟 backfill 逻辑（engine 侧 world 反查签名 → 这里直接按 ID 对匹配）。
        let hit = r
            .fixtures
            .iter_mut()
            .find(|f| {
                !f.is_bye()
                    && f.result.is_none()
                    && ((f.team_a == TeamId(3) && f.team_b == TeamId(0))
                        || (f.team_a == TeamId(0) && f.team_b == TeamId(3)))
            })
            .expect("1v4 对阵应存在");
        hit.result = Some(m);
        assert!(
            r.fixtures.iter().any(|f| f.result.is_some()),
            "回填后应有带结果的对阵"
        );
        assert_eq!(
            r.fixtures.iter().filter(|f| f.result.is_none()).count(),
            1,
            "其余保持 pending"
        );
    }

    #[test]
    fn fixture_status_progression_future_to_completed() {
        // 生命周期：Future（pending 对阵）→ Completed（结果回填）。
        let mut r = record_with(TournamentFormat::SingleElim, 2);
        assert_eq!(r.status, ScheduledStatus::Future);
        assert!(r.fixtures.iter().all(|f| f.result.is_none()));
        r.status = ScheduledStatus::Completed;
        for f in r.fixtures.iter_mut() {
            f.result = Some(MatchResult::new("T0", "T1", TourneyTier::T1));
        }
        assert!(r.fixtures.iter().all(|f| f.result.is_some()));
    }
}
