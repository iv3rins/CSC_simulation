//! 一场 CS2 赛事（Kotlin `Tournament.kt`）。

use serde::{Deserialize, Serialize};

use crate::city::City;
use crate::event_importance::EventImportance;
use crate::match_result::MatchVenue;
use crate::tournament_format::TournamentFormat;
use crate::tourney_tier::TourneyTier;

/// 赛事承办方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Organizer {
    /// Valve（顶层规则制定者，通常不直接办赛）
    Valve,
    /// ESL
    Esl,
    /// BLAST
    Blast,
    /// PGL
    Pgl,
    /// 二三线 CCT
    Cct,
    /// 其他承办方
    Other,
}

/// 参赛资格 / 直邀方式（依托 VRS 积分体系）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InvitePolicy {
    /// 依 VRS 全球积分直邀
    VrsGlobal,
    /// 依区域积分直邀（各区域分配名额）
    VrsRegional,
    /// 预选赛决出资格
    Qualifier,
    /// 公开海选
    Open,
}

/// 一场 CS2 赛事。
///
/// 赛事生态（由 Valve 顶层规则、不直接办赛；顶级赛事交由 ESL / BLAST / PGL 承办）：
/// - 等级分 [`TourneyTier`]：MAJOR（官方最高荣誉）、SUPERELITE（卡托维兹/科隆）、T1 / T2、QUALIFY（预选）
/// - 直邀资格依赖 VRS 全球积分（见 `vrs_weight` 与 `invite_policy`）
/// - 对抗结构由 `format` 决定（瑞士轮/双败/单败）
///
/// 三档赛事结构：
/// - T1：16 支参赛 = 12 直邀（前 12 名，拒绝按 VRS 次序替补）+ 4 支公开预选
/// - T2：持续举办，邀请 T2 队伍（线上赛 + T1 预选赛）
/// - T3：网吧赛
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tournament {
    /// 赛事名称，如「2025 IEM Katowice」「2025 BLAST Bounty」
    pub name: String,
    /// 赛事昵称（短名，如「Katowice」「EPL」「Major」；展示层优先用昵称）
    ///
    /// 存档兼容：旧存档缺省反序列化为空串，展示层回退到 [`Self::name`]。
    #[serde(default)]
    pub nickname: String,
    /// 赛事等级（决定冠军含金量与积分权重）
    pub tier: TourneyTier,
    /// 承办方（Valve 不直接办赛，顶级赛事由 ESL/BLAST/PGL 承办）
    pub organizer: Organizer,
    /// 举办城市
    pub city: City,
    /// 本赛事提供的 VRS 积分权重（越高说明直邀价值越高）
    pub vrs_weight: i32,
    /// 直邀 / 资格获取方式
    pub invite_policy: InvitePolicy,
    /// 参赛队伍总名额（T1=16）
    pub team_slots: i32,
    /// 直邀名额（按 VRS 名次序，拒绝则替补）
    pub direct_invites: i32,
    /// 公开预选赛名额（T1=4）
    pub open_qualifier_slots: i32,
    /// 比赛场地（T1 线下 / T2 线上 / T3 网吧）
    pub venue: MatchVenue,
    /// 赛事开始日期（由模拟时钟排程写入，如「2026-06-08」）
    pub date: String,
    /// 赛事持续天数（排程占用时间片；每赛程持续时间可不同）
    pub duration_days: i32,
    /// 对抗结构赛制（默认单败，兼容旧行为）
    pub format: TournamentFormat,
    /// 玩家体验层：赛事仍会完整模拟，只有高价值赛事进入 LIVE。
    #[serde(default)]
    pub importance: EventImportance,
    /// 参赛队数上限（None = 该档全池；用于 T2/T3 每月多场时每场取不同队伍）
    pub teams_per_event: Option<i32>,
    /// 邀请池轮转偏移（同档多场时从池中不同排名段取队）
    pub pool_offset: i32,
}

impl Tournament {
    /// 构造赛事（必填参数；其余字段取 Kotlin 默认值，构造后可按需覆盖）。
    ///
    /// 与 Kotlin `data class` 默认参数语义一致：`team_slots=16`、`direct_invites=12`、
    /// `open_qualifier_slots=4`、`venue=Lan`、`date="TBD"`、`duration_days=3`、
    /// `format=SingleElim`、`teams_per_event=None`、`pool_offset=0`。
    pub fn new(
        name: impl Into<String>,
        tier: TourneyTier,
        organizer: Organizer,
        city: City,
        vrs_weight: i32,
        invite_policy: InvitePolicy,
    ) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            nickname: name,
            tier,
            organizer,
            city,
            vrs_weight,
            invite_policy,
            team_slots: 16,
            direct_invites: 12,
            open_qualifier_slots: 4,
            venue: MatchVenue::Lan,
            date: "TBD".to_string(),
            duration_days: 3,
            format: TournamentFormat::SingleElim,
            importance: EventImportance::default(),
            teams_per_event: None,
            pool_offset: 0,
        }
    }

    // —— 可变字段的 builder 方法（Kotlin 命名参数覆盖的 Rust 形态）——

    pub fn with_team_slots(mut self, v: i32) -> Self {
        self.team_slots = v;
        self
    }

    pub fn with_direct_invites(mut self, v: i32) -> Self {
        self.direct_invites = v;
        self
    }

    pub fn with_open_qualifier_slots(mut self, v: i32) -> Self {
        self.open_qualifier_slots = v;
        self
    }

    pub fn with_venue(mut self, v: MatchVenue) -> Self {
        self.venue = v;
        self
    }

    pub fn with_date(mut self, v: impl Into<String>) -> Self {
        self.date = v.into();
        self
    }

    pub fn with_format(mut self, v: TournamentFormat) -> Self {
        self.format = v;
        self
    }

    pub fn with_importance(mut self, v: EventImportance) -> Self {
        self.importance = v;
        self
    }

    pub fn with_duration_days(mut self, v: i32) -> Self {
        self.duration_days = v;
        self
    }

    pub fn with_vrs_weight(mut self, v: i32) -> Self {
        self.vrs_weight = v;
        self
    }

    pub fn with_nickname(mut self, v: impl Into<String>) -> Self {
        self.nickname = v.into();
        self
    }

    pub fn with_teams_per_event(mut self, v: Option<i32>) -> Self {
        self.teams_per_event = v;
        self
    }

    pub fn with_pool_offset(mut self, v: i32) -> Self {
        self.pool_offset = v;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::Region;

    fn katowice() -> City {
        City::new("卡托维兹", "波兰", Region::Europe)
    }

    #[test]
    fn defaults_match_kotlin() {
        let t = Tournament::new(
            "2025 IEM Katowice",
            TourneyTier::SuperElite,
            Organizer::Esl,
            katowice(),
            100,
            InvitePolicy::VrsGlobal,
        );
        assert_eq!(t.team_slots, 16);
        assert_eq!(t.direct_invites, 12);
        assert_eq!(t.open_qualifier_slots, 4);
        assert_eq!(t.venue, MatchVenue::Lan);
        assert_eq!(t.date, "TBD");
        assert_eq!(t.duration_days, 3);
        assert_eq!(t.format, TournamentFormat::SingleElim);
        assert_eq!(t.importance, EventImportance::Background);
        assert_eq!(t.teams_per_event, None);
        assert_eq!(t.pool_offset, 0);
    }

    #[test]
    fn field_override_after_new() {
        let mut t = Tournament::new(
            "BLAST Bounty",
            TourneyTier::T1,
            Organizer::Blast,
            katowice(),
            50,
            InvitePolicy::VrsGlobal,
        );
        t.format = TournamentFormat::swiss_playoff();
        t.date = "2026-06-08".to_string();
        assert_eq!(t.format, TournamentFormat::swiss_playoff());
        assert_eq!(t.date, "2026-06-08");
    }

    #[test]
    fn serde_roundtrip() {
        let t = Tournament::new(
            "2025 IEM Katowice",
            TourneyTier::SuperElite,
            Organizer::Esl,
            katowice(),
            100,
            InvitePolicy::VrsGlobal,
        );
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#""organizer":"ESL""#));
        assert!(json.contains(r#""tier":"SUPERELITE""#));
        assert!(json.contains(r#""format":"SINGLE_ELIM""#));
        let back: Tournament = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }
}
