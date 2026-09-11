//! 玩家的职业 / 经济 / 生涯数据（Kotlin `CareerInfo.kt` + `Honours.kt` + `PlayerFinance.kt`）。

use serde::{Deserialize, Serialize};

use csc_domain::season_goal::SeasonGoal;
use csc_domain::tournament::Tournament;
use csc_domain::tourney_tier::TourneyTier;

use crate::mark::CareerMark;
use crate::transfer_contact::TeamContact;

/// 个人荣誉类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IndividualHonourType {
    /// 年度 TOP20 选手
    Top20,
    /// 赛事 MVP（最有价值选手）
    Mvp,
    /// 赛事 EVP（极具价值选手——四强内非 MVP 但数据拔尖）
    Evp,
    /// 年度最佳阵容
    AllStarTeam,
    /// 年度最佳新秀
    RookieOfTheYear,
    /// 最佳残局/关键时刻选手
    BestClutchPlayer,
    /// 其他个人荣誉
    Other,
}

/// 一条个人荣誉。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndividualHonour {
    pub r#type: IndividualHonourType,
    /// 获奖年份
    pub year: i32,
    /// 补充说明（纯展示文案，如「TOP20 第 5 名」「2025 上海 Major MVP」）
    pub detail: String,
    /// 荣誉所属赛事层级（MVP/EVP 计分用；TOP20 等年度荣誉为 None）
    #[serde(default)]
    pub tier: Option<TourneyTier>,
    /// 荣誉所属赛事名（MVP/EVP；TOP20/年度阵容等为 None）
    /// —— 结构化字段，替代从 `detail` 字符串解析（防文案格式漂移破坏前端）。
    #[serde(default)]
    pub event_name: Option<String>,
    /// 名次（TOP20 第 N 名等；非排名类荣誉为 None）
    #[serde(default)]
    pub rank: Option<i32>,
}

/// 团队荣誉（获得的一项冠军）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamHonour {
    /// 赛事名称，如「2025 IEM Katowice」
    pub title: String,
    /// 夺冠年份
    pub year: i32,
    /// 赛事等级，决定冠军含金量
    pub tier: TourneyTier,
    /// 补充说明（MVP 归属、奖金等，可空）
    pub detail: String,
}

/// 玩家身份状态：由已有比赛、荣誉、资历与关系事实派生，不参与比赛模拟。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub performance: f64,
    pub reputation: f64,
    pub leadership: f64,
    pub influence: f64,
}

impl Default for PlayerStatus {
    fn default() -> Self {
        Self {
            performance: 50.0,
            reputation: 50.0,
            leadership: 50.0,
            influence: 0.0,
        }
    }
}

/// 职业生涯记忆类型。只追加首次里程碑或明确的职业节点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CareerMemoryKind {
    FirstT1Contract,
    FirstMajor,
    MajorPlayoff,
    MajorFinal,
    MajorChampion,
    Mvp,
    Top20,
    TeamJoin,
    TeamDeparture,
    MajorKeyMoment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CareerMemory {
    pub kind: CareerMemoryKind,
    pub year: i32,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub event_name: Option<String>,
    pub detail: String,
}

/// 玩家获得的荣誉集合（荣誉是可增长的列表，随生涯推进不断追加）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Honours {
    /// 个人荣誉（TOP20、MVP...）
    pub individual: Vec<IndividualHonour>,
    /// 团队荣誉（获得的冠军）
    pub team: Vec<TeamHonour>,
}

/// 一份代言合同。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sponsorship {
    pub brand: String,
    /// 年付金额
    pub annual_value: i64,
    /// 剩余年限（每年结算 -1）
    pub years_left: i32,
    pub signed_year: i32,
}

impl Sponsorship {
    /// 年度结转（跨年调用）。
    pub fn tick_year(&self) -> Self {
        Self {
            years_left: (self.years_left - 1).max(0),
            ..self.clone()
        }
    }

    /// 是否已到期。
    pub fn expired(&self) -> bool {
        self.years_left <= 0
    }
}

/// 玩家财务 —— 生涯经济系统的**玩家侧状态**（队伍侧是 `Team.budget`）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlayerFinance {
    /// 可支配现金（奖金分成 + 代言收入累计）
    pub cash: i64,
    /// 生涯总收益
    pub career_earnings: i64,
    /// 生效中的代言合同
    pub sponsors: Vec<Sponsorship>,
    /// 上次代言评估年份（防同窗重复评估；跨年由 EconomyEngine 检查）
    pub last_sponsor_check_year: i32,
    /// 逐年收益明细（year → 收入，供生涯档案 SeasonRecord.earnings）
    pub earnings_by_year: std::collections::HashMap<i32, i64>,
    /// CS 饰品藏品（2026 复审新增：资金运用「购买饰品」所得；v7 存档）
    #[serde(default)]
    pub skins: Vec<SkinOwned>,
    /// 上次「投资自己」年份（每年限一次；v7 存档）
    #[serde(default)]
    pub last_invest_year: i32,
}

impl PlayerFinance {
    /// 本年度累计收入。
    pub fn earnings_of(&self, year: i32) -> i64 {
        self.earnings_by_year.get(&year).copied().unwrap_or(0)
    }
}

/// 一件 CS 饰品藏品（资金运用「购买饰品」产出；纯展示 + 声誉，无市场交易）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinOwned {
    /// 饰品名（如「AWP | Dragon Lore」）
    pub name: String,
    /// 稀有度：standard / rare
    pub rarity: String,
    /// 入手价（货币单位）
    pub value: i64,
    /// 入手年份
    pub year: i32,
}

/// 玩家的职业 / 经济 / 生涯数据（由 [`crate::character::PlayerCharacter`] 的 `career` 承载）。
///
/// 转写差异：Kotlin `CareerInfo.team: Team`（对象引用）在 Rust 中被移除——
/// 归属统一到实体的 `team: Option<TeamId>` 字段（单一事实来源，见 lib.rs 说明）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CareerInfo {
    /// 年薪（货币单位）
    pub salary: i64,
    /// 剩余合同年限
    pub contract_years: i32,
    /// 知名度 (0..100)，影响转会与代言
    pub reputation: i32,
    /// 生涯胜场数
    pub career_wins: i32,
    /// 生涯击杀数
    pub career_kills: i32,
    /// 生涯死亡数
    pub career_deaths: i32,
    /// 职业生涯起始年份
    pub career_start_year: i32,
    /// 伤病史（记录历次伤病，便于健康恢复逻辑）
    pub injury_history: Vec<String>,
    /// 玩家荣誉（个人与团队）
    pub honours: Honours,
    /// 参赛赛事对象（每场新建的 Tournament，按参赛顺序记录）
    pub attended_tournaments: Vec<Tournament>,
    /// 生涯印记（flag）：场内操作/决策的长期影响
    pub marks: Vec<CareerMark>,
    /// 玩家财务（奖金分成/代言）
    pub finance: PlayerFinance,
    /// 本月休养标记（伤病决策 REST 置位，痊愈后复位）
    pub resting: bool,
    /// 本年度伤病缺勤天数（生涯档案素材，跨年清零）
    pub injury_days_this_year: i32,
    /// **连续待业月数**（2026 合同状态机：自由身 STAY 每次转会窗 +4；签约归零；
    /// ≥12 个月强制「赋闲/待业」——薪资归零、不再发放原薪水）。存档 v2 新增。
    #[serde(default)]
    pub months_unsigned: i32,
    /// **待执行训练计划**（玩家主动触发，非月度强制决策）：
    /// 值为 `TrainingFocus.name()`（AIM/UTILITY/…/REST）。存于状态中，
    /// 在下一次 `advance_month` 开头应用并清空——存档/回放天然可复现。
    /// 存档 v3 新增。
    #[serde(default)]
    pub pending_training: Option<String>,
    /// 转会市场「主动接触」：表达招募兴趣的队伍（非决策点，市场情报）。
    /// 存档 v4 新增。
    #[serde(default)]
    pub transfer_contacts: Vec<TeamContact>,
    /// 当前赛季主目标（产品层赛季闭环）。存档 v6 新增。
    #[serde(default)]
    pub season_goal: Option<SeasonGoal>,
    /// 目标设定的年份（跨年结算时清空，下赛季需重新选择）。
    #[serde(default)]
    pub season_goal_year: i32,
    /// 最近一次场外人生事件类型（Sprint 5 冷却用；String 保持实体层不依赖决策层）。
    #[serde(default)]
    pub life_event_last_kind: Option<String>,
    /// 最近一次场外人生事件的月序（year*12+month）。
    #[serde(default)]
    pub life_event_last_month: i32,
    /// 玩家主动设定的赛前战术预案（Major BP / 关键赛事打法）：
    /// "AGGRESSIVE" | "BALANCED" | "CONSERVATIVE"。存档 v8 新增。
    /// 由 `/actions/match-plan` 写入，conductor 将其作为场内干预的默认基线。
    #[serde(default)]
    pub match_style: Option<String>,
    /// 最近一次公开表态（"CONFIDENT" | "HUMBLE" | "PROVOKE"）；
    /// 展示层回显用，效果在表态时立即结算。存档 v8 新增。
    #[serde(default)]
    pub public_stance: Option<String>,
    /// P1.0 career projection；不参与比赛/排行计算。
    #[serde(default)]
    pub player_status: PlayerStatus,
    /// P1.0 结构化职业记忆；按确定性顺序追加。
    #[serde(default)]
    pub career_memory: Vec<CareerMemory>,
}

impl CareerInfo {
    pub fn remember(&mut self, memory: CareerMemory) {
        if !self.career_memory.iter().any(|m| m.kind == memory.kind) {
            self.career_memory.push(memory);
        }
    }

    /// 自由身初始值（= Kotlin `RandomPlayerGenerator.generatePlayer` 的默认 CareerInfo：
    /// 0 薪资、0 合同年、reputation=50、起始年份 2026、空荣誉/伤病/印记）。
    pub fn free_agent_default(career_start_year: i32) -> Self {
        Self {
            salary: 0,
            contract_years: 0, // 0 = 自由身，可被签约
            reputation: 50,
            career_wins: 0,
            career_kills: 0,
            career_deaths: 0,
            career_start_year,
            injury_history: Vec::new(),
            honours: Honours::default(),
            attended_tournaments: Vec::new(),
            marks: Vec::new(),
            finance: PlayerFinance::default(),
            resting: false,
            injury_days_this_year: 0,
            months_unsigned: 0,
            pending_training: None,
            transfer_contacts: Vec::new(),
            season_goal: None,
            season_goal_year: 0,
            life_event_last_kind: None,
            life_event_last_month: 0,
            match_style: None,
            public_stance: None,
            player_status: PlayerStatus::default(),
            career_memory: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sponsorship_tick_and_expiry() {
        let s = Sponsorship {
            brand: "Nike".into(),
            annual_value: 100_000,
            years_left: 2,
            signed_year: 2026,
        };
        assert!(!s.expired());
        let s1 = s.tick_year();
        assert_eq!(s1.years_left, 1);
        let s2 = s1.tick_year();
        assert_eq!(s2.years_left, 0);
        assert!(s2.expired());
        // 不再扣成负数
        assert_eq!(s2.tick_year().years_left, 0);
    }

    #[test]
    fn finance_earnings_of_defaults_zero() {
        let f = PlayerFinance::default();
        assert_eq!(f.earnings_of(2026), 0);
        assert_eq!(f.cash, 0);
    }

    #[test]
    fn free_agent_default_matches_kotlin() {
        let c = CareerInfo::free_agent_default(2026);
        assert_eq!(c.salary, 0);
        assert_eq!(c.contract_years, 0);
        assert_eq!(c.reputation, 50);
        assert_eq!(c.career_start_year, 2026);
        assert!(c.attended_tournaments.is_empty());
        assert!(c.marks.is_empty());
        assert!(!c.resting);
        assert_eq!(c.injury_days_this_year, 0);
    }

    #[test]
    fn career_layer_defaults_and_memory_dedup_are_compatible() {
        let mut career = CareerInfo::free_agent_default(2026);
        career.remember(CareerMemory {
            kind: CareerMemoryKind::FirstMajor,
            year: 2026,
            date: None,
            event_name: Some("Major".into()),
            detail: "First Major".into(),
        });
        career.remember(CareerMemory {
            kind: CareerMemoryKind::FirstMajor,
            year: 2027,
            date: None,
            event_name: Some("Another Major".into()),
            detail: "must not duplicate".into(),
        });
        assert_eq!(career.career_memory.len(), 1);
        assert_eq!(career.player_status, PlayerStatus::default());

        let mut legacy = serde_json::to_value(&career).unwrap();
        legacy.as_object_mut().unwrap().remove("player_status");
        legacy.as_object_mut().unwrap().remove("career_memory");
        let restored: CareerInfo = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.player_status, PlayerStatus::default());
        assert!(restored.career_memory.is_empty());
    }

    #[test]
    fn honours_serde() {
        let h = Honours {
            individual: vec![IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year: 2026,
                detail: "TOP20 第 5 名".into(),
                tier: None,
                event_name: None,
                rank: Some(5),
            }],
            team: vec![TeamHonour {
                title: "2025 IEM Katowice".into(),
                year: 2025,
                tier: TourneyTier::SuperElite,
                detail: String::new(),
            }],
        };
        let json = serde_json::to_string(&h).unwrap();
        assert!(json.contains(r#""TOP20""#));
        assert!(json.contains(r#""SUPERELITE""#));
        let back: Honours = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }
}
