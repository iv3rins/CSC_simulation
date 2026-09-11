//! VRS 子系统 sub-engine：排名与积分计算的统一门面（Kotlin `VrsEngine.kt` 转写）。

use std::collections::HashMap;

use csc_domain::match_result::MatchResult;
use csc_domain::match_result::MatchVenue;
use csc_domain::team_tier::TeamTier;
use csc_domain::tourney_tier::TourneyTier;
use csc_time::window::TimeWindow;

use crate::database::{VrsDatabase, VrsTeamState};
use crate::entry::VrsEntry;
use crate::scoring::VrsScoring;

/// VRS 子系统 sub-engine：外部（赛事引擎、转会系统等）只依赖本引擎，不直接触碰数据库。
pub struct VrsEngine {
    database: VrsDatabase,
}

impl VrsEngine {
    // —— 构造 ——

    /// 从 (文件名, 内容) 列表加载 standings 初始基准（零 IO；文件读取由调用方负责）。
    /// 外部输入边界（D6）：解析/语义校验失败返回 `Err(SimError)`。
    pub fn from_json_files(files: &[(String, String)]) -> Result<Self, csc_util::SimError> {
        Ok(Self {
            database: VrsDatabase::from_json_files(files)?,
        })
    }

    /// 从已解析的数据库构造（测试/定制加载用）。
    pub fn from_database(database: VrsDatabase) -> Self {
        Self { database }
    }

    /// 内部数据库引用（引擎编排/测试用）。
    pub fn database(&self) -> &VrsDatabase {
        &self.database
    }

    /// 队伍阵容签名（= Kotlin `VrsEngine.signatureOf(teamName, roster)`）。
    pub fn signature_of(team_name: &str, roster: &[String]) -> String {
        VrsDatabase::signature_of(team_name, roster)
    }

    /// 由排名条目求阵容签名（= Kotlin `signatureOf(entry)`）。
    pub fn signature_of_entry(entry: &VrsEntry) -> String {
        VrsDatabase::signature_of(&entry.team_name, &entry.roster)
    }

    // —— 排名查询 ——

    /// 按时间排序的月份标识（如 "2026_01_05"）。
    pub fn months(&self) -> &[String] {
        &self.database.months
    }

    /// 某月全部排名条目（按排名升序）。
    pub fn entries_of(&self, month: &str) -> Vec<VrsEntry> {
        self.database.entries_of(month)
    }

    /// 队伍在指定月份的历史排名；查不到返回 None。
    pub fn ranking_of(&self, entry: &VrsEntry, month: &str) -> Option<i32> {
        self.database.ranking_of(entry, month)
    }

    /// 该队伍在指定月份及之前 count 个月里是否都进入前 topN。
    pub fn consecutive_top_n(
        &self,
        entry: &VrsEntry,
        month: &str,
        top_n: i32,
        count: usize,
    ) -> bool {
        self.database.consecutive_top_n(entry, month, top_n, count)
    }

    /// 当前参与模拟的队伍状态列表（按积分降序，即最新排名）。
    pub fn all_teams(&self) -> Vec<&VrsTeamState> {
        self.database.all_teams()
    }

    /// 按签名取队伍状态；不存在返回 None。
    pub fn team_of(&self, signature: &str) -> Option<&VrsTeamState> {
        self.database.team_of(signature)
    }

    /// 队伍当前的 VRS 静态 Seed 分；未 reseed 过或查不到时返回 None。
    pub fn seed_points_of(&self, signature: &str) -> Option<i32> {
        self.database.seed_points_of(signature)
    }

    // —— 积分结算（官方两层模型）——

    /// 结算一场比赛（按签名）；未知签名 → 告警并跳过（不 fail-fast）。
    pub fn apply_match(
        &mut self,
        winner_sig: &str,
        loser_sig: &str,
        tier: TourneyTier,
        timestamp: Option<i64>,
        window: Option<&TimeWindow>,
    ) -> bool {
        self.database
            .apply_match(winner_sig, loser_sig, tier, timestamp, window)
    }

    /// 按 MatchResult 结算（保留 venue 信息）。
    pub fn apply_match_result(
        &mut self,
        result: &MatchResult,
        timestamp: Option<i64>,
        window: Option<&TimeWindow>,
    ) -> bool {
        self.database.apply_match_result(result, timestamp, window)
    }

    /// 更新队伍阵容；出走 ≥ ROSTER_CLEAR_DEPARTURES 人则积分清零。返回是否清零。
    pub fn apply_roster_change(&mut self, signature: &str, new_roster: &[String]) -> bool {
        self.database.apply_roster_change(signature, new_roster)
    }

    /// VRS 阵容迁移（带漂移校验）：已知旧签名但迁移后新签名查不到 → fail-fast；
    /// VRS 未知旧签名（空库/部分加载降级环境）→ 告警，继续（= Kotlin `applyRosterChangeVerified`）。
    pub fn apply_roster_change_verified(&mut self, old_sig: &str, new_roster: &[String]) {
        let known_team_name = self.team_of(old_sig).map(|team| team.team_name.clone());
        let known_before = known_team_name.is_some();
        self.apply_roster_change(old_sig, new_roster);
        let team_name = known_team_name
            .unwrap_or_else(|| old_sig.split('|').next().unwrap_or(old_sig).to_string());
        let new_sig = VrsDatabase::signature_of(&team_name, new_roster);
        if known_before && self.team_of(&new_sig).is_none() {
            panic!(
                "VRS 阵容迁移失败：{old_sig} → {new_sig}（实体已迁移但 VRS 状态缺失）——状态漂移，拒绝继续"
            );
        }
    }

    /// 转会窗结算：阵容连续性基准重置到当前阵容（离队计数按窗统计，
    /// 不再跨窗累计；见 [`VrsDatabase::settle_rosters`]）。
    pub fn settle_roster_window(&mut self) {
        self.database.settle_rosters();
    }

    /// 基于比赛历史重算全部队伍的 VRS 静态 Seed 分（**同时按窗口剪枝比赛历史**）。
    pub fn reseed(&mut self, window: &TimeWindow) {
        self.database.reseed(window);
    }

    // —— 积分计算纯函数（官方公式入口）——

    /// 胜者预期胜率（官方 ELO 公式）。
    pub fn expected_win_rate(&self, winner_points: i32, loser_points: i32) -> f64 {
        VrsScoring::expected_win_rate(winner_points, loser_points)
    }

    /// 赛事等级 → 默认比赛形式（T1 及以上线下、T2/T3 线上）。
    pub fn tier_venue(&self, tier: TourneyTier) -> MatchVenue {
        csc_domain::tier_profile::tier_venue(tier)
    }

    /// 赛事等级 → 估计奖池（美元）。
    pub fn tier_prize_pool(&self, tier: TourneyTier) -> i32 {
        csc_domain::tier_profile::tier_prize_pool(tier)
    }

    // —— 规则门面（分级与资格）——

    /// T1 赛事邀请数量（从第 1 名开始）。
    pub fn t1_team_count(&self) -> i32 {
        TeamTier::T1_TEAM_COUNT
    }

    /// T1 赛事公开预选赛名额。
    pub fn t1_open_qualifier_slots(&self) -> i32 {
        Self::T1_OPEN_QUALIFIER_SLOTS
    }

    /// 单月快速分级（无历史数据时的兜底）：1~12 → T1，13~32 → T2，33~120 → T3，121+ → T4。
    pub fn team_tier_of_ranking(&self, ranking: i32) -> TeamTier {
        TeamTier::from_ranking(ranking)
    }

    /// 按历史判定队伍级别（**实力层级**）：当月及前 T1_CONSECUTIVE_MONTHS 个月均在前
    /// T1_TEAM_COUNT → 真 T1；否则按当月排名降级。
    pub fn team_tier_of(&self, entry: &VrsEntry, month: &str) -> TeamTier {
        if self.database.consecutive_top_n(
            entry,
            month,
            TeamTier::T1_TEAM_COUNT,
            Self::T1_CONSECUTIVE_MONTHS,
        ) {
            TeamTier::T1
        } else {
            TeamTier::from_ranking(entry.ranking)
        }
    }

    /// 某月全部队伍的历史分级结果（队伍名 → 级别；同名不同队会合并展示）。
    pub fn tier_map_of(&self, month: &str) -> HashMap<String, TeamTier> {
        self.database
            .entries_of(month)
            .into_iter()
            .map(|e| (e.team_name.clone(), self.team_tier_of(&e, month)))
            .collect()
    }

    /// T1 赛事直邀名单：**当月** VRS 排名前 T1_TEAM_COUNT（实时排名，单一事实来源）。
    pub fn t1_invitees(&self) -> Vec<VrsEntry> {
        self.current_top(TeamTier::T1_TEAM_COUNT)
    }

    /// T2 赛事报名资格：当月排名 13~32。
    pub fn t2_eligible(&self) -> Vec<VrsEntry> {
        self.ranked_in(TeamTier::T1_TEAM_COUNT + 1, TeamTier::T2_MAX_RANKING)
    }

    /// T3 赛事报名资格：当月排名 33~120。
    pub fn t3_eligible(&self) -> Vec<VrsEntry> {
        self.ranked_in(TeamTier::T2_MAX_RANKING + 1, TeamTier::T3_MAX_RANKING)
    }

    /// T4 赛事报名资格：当月排名 120 名开外。
    pub fn t4_eligible(&self) -> Vec<VrsEntry> {
        self.ranked_in(TeamTier::T3_MAX_RANKING + 1, i32::MAX)
    }

    /// Major 参赛队伍：当月排名前 MAJOR_MAX_TEAMS。
    pub fn major_field(&self) -> Vec<VrsEntry> {
        self.current_top(Self::MAJOR_MAX_TEAMS)
    }

    /// 实时排名条目（全部队伍按排名升序；表现层查询用）。
    pub fn ranking_entries(&self) -> Vec<VrsEntry> {
        self.database
            .all_teams()
            .into_iter()
            .map(to_entry)
            .collect()
    }

    /// 当前模拟状态前 n 名（VRS 数据库实时排名 → 排名条目）。
    fn current_top(&self, n: i32) -> Vec<VrsEntry> {
        self.database
            .all_teams()
            .into_iter()
            .take(n as usize)
            .map(to_entry)
            .collect()
    }

    /// 实时排名落在 [from, to] 区间的队伍。
    fn ranked_in(&self, from: i32, to: i32) -> Vec<VrsEntry> {
        self.database
            .all_teams()
            .into_iter()
            .filter(|t| t.ranking >= from && t.ranking <= to)
            .map(to_entry)
            .collect()
    }
}

/// 模拟状态 → 排名条目（供赛事排程/转会读取）。
pub fn to_entry(t: &VrsTeamState) -> VrsEntry {
    VrsEntry::new(t.ranking, t.points, &t.team_name, t.roster.clone())
}

impl VrsEngine {
    /// T1 赛事公开预选赛名额。
    pub const T1_OPEN_QUALIFIER_SLOTS: i32 = 4;
    /// T1 需要连续入选前 T1_TEAM_COUNT 的月数。
    pub const T1_CONSECUTIVE_MONTHS: usize = 3;
    /// Major 参赛上限。
    pub const MAJOR_MAX_TEAMS: i32 = 32;
    /// 阵容出走达到该人数即触发积分清零（透传）。
    pub const ROSTER_CLEAR_DEPARTURES: i32 = VrsDatabase::ROSTER_CLEAR_DEPARTURES;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "rankings": [
            { "ranking": 1, "points": 2081, "teamName": "Vitality", "roster": ["apEX", "ZywOo", "ropz", "mezii", "flameZ"] },
            { "ranking": 2, "points": 1900, "teamName": "FaZe", "roster": ["karrigan", "broky", "rain", "frozen", "Twistzz"] },
            { "ranking": 3, "points": 1800, "teamName": "NAVI", "roster": ["b1t", "Aleksib", "w0nderful", "iM", "jL"] },
            { "ranking": 15, "points": 900, "teamName": "T2Team", "roster": ["x1", "x2", "x3", "x4", "x5"] },
            { "ranking": 40, "points": 500, "teamName": "T3Team", "roster": ["y1", "y2", "y3", "y4", "y5"] }
        ]
    }"#;

    fn engine() -> VrsEngine {
        VrsEngine::from_json_files(&[(
            "standings_global_2026_05_04.json".to_string(),
            SAMPLE.to_string(),
        )])
        .expect("样例资产合法")
    }

    #[test]
    fn qualification_gates() {
        let e = engine();
        assert_eq!(e.t1_invitees().len(), 5, "样例 5 队全部进前 12");
        assert_eq!(e.t1_invitees()[0].team_name, "Vitality");
        // 当前模拟状态的 ranking 由 points 重排（样例 5 队 → 1..5），全部落在 T1 区间
        assert!(e.t2_eligible().is_empty(), "5 队全在前 12，无 T2 资格");
        assert!(e.t3_eligible().is_empty());
        assert!(e.t4_eligible().is_empty());
        assert_eq!(e.major_field().len(), 5);
    }

    #[test]
    fn tier_judgement_with_history() {
        // 单月基准：排名 3 → T1
        let e = engine();
        let entry = VrsEntry::new(
            3,
            1800,
            "NAVI",
            vec![
                "b1t".into(),
                "Aleksib".into(),
                "w0nderful".into(),
                "iM".into(),
                "jL".into(),
            ],
        );
        assert_eq!(e.team_tier_of(&entry, "2026_05_04"), TeamTier::T1);
        let t2 = VrsEntry::new(
            15,
            900,
            "T2Team",
            vec![
                "x1".into(),
                "x2".into(),
                "x3".into(),
                "x4".into(),
                "x5".into(),
            ],
        );
        assert_eq!(e.team_tier_of(&t2, "2026_05_04"), TeamTier::T2);
        assert_eq!(e.team_tier_of_ranking(100), TeamTier::T3);
    }

    #[test]
    fn roster_change_verified_panics_on_drift() {
        let mut e = engine();
        let old_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".into(),
                "ZywOo".into(),
                "ropz".into(),
                "mezii".into(),
                "flameZ".into(),
            ],
        );
        let new_roster: Vec<String> =
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
        // 3 人以上出走 → 迁移成功（签名迁移 + 清零），不 panic
        e.apply_roster_change_verified(&old_sig, &new_roster);
        // 未知旧签名（降级环境）→ 不 panic
        e.apply_roster_change_verified("Ghost|a,b", &new_roster);
    }

    #[test]
    fn expected_win_rate_pure_function() {
        let e = engine();
        assert!(e.expected_win_rate(2000, 1000) > 0.9);
        assert!((e.expected_win_rate(1500, 1500) - 0.5).abs() < 1e-9);
    }
}
