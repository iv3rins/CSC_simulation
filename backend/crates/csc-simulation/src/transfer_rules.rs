//! 转会 / 签约门槛机制（Kotlin `TransferRules.kt` 转写）。

use csc_domain::VrsEntry;
use csc_domain::team_tier::TeamTier;
use csc_entities::character::PlayerCharacter;
use csc_entities::power::PowerCalculator;

/// 转会 / 签约门槛机制。
///
/// 因果方向：**选手变强才加入指定队伍**——选手的个人实力决定他能进入
/// 哪个级别的队伍，而不是进入队伍后才变强。
pub struct TransferRules;

impl TransferRules {
    /// 进入 T1 队伍所需的最低实力
    pub const T1_POWER_THRESHOLD: f64 = 85.0;
    /// 进入 T2 队伍所需的最低实力
    pub const T2_POWER_THRESHOLD: f64 = 75.0;
    /// 进入 T3 队伍所需的最低实力
    pub const T3_POWER_THRESHOLD: f64 = 60.0;
    /// NPC 转会市场：单名「低就」NPC 被更强队伍挖角的成交概率（每转会窗逐名掷骰；
    /// 游戏平衡参数，随版本可调）。
    pub const NPC_MARKET_DEAL_PROBABILITY: f64 = 0.5;

    /// 选手是否有资格加入指定级别的队伍。
    pub fn can_join(power: f64, team_tier: TeamTier) -> bool {
        match team_tier {
            TeamTier::T1 => power >= Self::T1_POWER_THRESHOLD,
            TeamTier::T2 => power >= Self::T2_POWER_THRESHOLD,
            TeamTier::T3 => power >= Self::T3_POWER_THRESHOLD,
            TeamTier::T4 => true, // 无门槛，新人可加入
        }
    }

    /// 选手当前可加入的最高队伍级别（升序：T4 → T3 → T2 → T1）。
    pub fn max_joinable_tier(power: f64) -> TeamTier {
        if power >= Self::T1_POWER_THRESHOLD {
            TeamTier::T1
        } else if power >= Self::T2_POWER_THRESHOLD {
            TeamTier::T2
        } else if power >= Self::T3_POWER_THRESHOLD {
            TeamTier::T3
        } else {
            TeamTier::T4
        }
    }

    /// 签约 / 续约时长：由选手当前实力决定（能力越强签得越久）。
    pub fn contract_duration(power: f64) -> i32 {
        if power >= 90.0 {
            4
        } else if power >= 80.0 {
            3
        } else if power >= 70.0 {
            2
        } else {
            1
        }
    }

    /// 年薪：实力基础薪 + **顶级溢价**（power>75 平方项——TOP 选手薪资跃升）+
    /// 声誉溢价（reputation 50 = 基准；每高 1 点 +2000/年）。
    ///
    /// **2026 试玩修订**：原 `power×5000` 斜率过浅（实力 69 与 90 薪资仅差
    /// 13 万）。新公式下 69→27.6 万、85→46 万、90→63 万、95→86 万——
    /// 顶级选手与普通选手拉开 2-3 倍（贴近真实 CS 薪资结构）。
    pub fn salary_of(power: f64, reputation: i32) -> i64 {
        let base = power * 4_000.0;
        let elite = if power > 75.0 {
            (power - 75.0).powi(2) * 1_200.0
        } else {
            0.0
        };
        (base + elite + ((reputation - 50) * 2_000) as f64).max(0.0) as i64
    }

    /// 选手当前实力（便捷入口，= `PowerCalculator::player_power`）。
    pub fn power_of(player: &PlayerCharacter) -> f64 {
        PowerCalculator::player_power(player)
    }

    /// 从候选队伍中筛出该选手可加入的队伍（= Kotlin `eligibleTeams`）。
    ///
    /// @param candidates 候选队伍（含 ranking，用于判定队伍级别）
    /// @return 可加入的队伍（排名升序，便于挑最好的）
    pub fn eligible_teams(player: &PlayerCharacter, candidates: &[VrsEntry]) -> Vec<VrsEntry> {
        let power = PowerCalculator::player_power(player); // 选手当前实力
        let mut eligible: Vec<VrsEntry> = candidates
            .iter()
            .filter(|c| Self::can_join(power, TeamTier::from_ranking(c.ranking))) // 只留够门槛的队伍
            .cloned()
            .collect();
        eligible.sort_by_key(|c| c.ranking); // 按排名升序（优先强队）
        eligible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_teams_filters_by_threshold() {
        let player = PlayerCharacter {
            id: csc_util::id::PlayerId(0),
            name: "rookie".into(),
            age: 20,
            role: csc_entities::role::Role::Rifler,
            base: csc_entities::attributes::BaseAttributes {
                reaction: 70,
                stability: 70,
                endurance: 70,
                stamina: 70,
                health: 70,
            },
            skill: csc_entities::attributes::SkillAttributes {
                aim: 70,
                leader: 50,
                communication: 50,
                clutch: 60,
            },
            pro: csc_entities::attributes::ProAttributes {
                mentality: 60,
                confidence: 60,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: csc_entities::attributes::WeaponAttributes {
                position: csc_entities::role::Role::Rifler,
                ak: 70,
                awp: 50,
                pistol: 60,
                smoke: 50,
                utility: 50,
            },
            potential: 85,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        };
        // 实力 ≈ 62：T3（排名 40）可去、T2（排名 15）不可
        let candidates = vec![
            VrsEntry::new(15, 900, "T2Team", vec![]),
            VrsEntry::new(40, 500, "T3Team", vec![]),
            VrsEntry::new(150, 100, "T4Team", vec![]),
        ];
        let eligible = TransferRules::eligible_teams(&player, &candidates);
        assert_eq!(eligible.len(), 2);
        assert_eq!(eligible[0].team_name, "T3Team", "排名升序");
        assert_eq!(eligible[1].team_name, "T4Team");
    }

    #[test]
    fn thresholds_match_kotlin() {
        assert_eq!(TransferRules::T1_POWER_THRESHOLD, 85.0);
        assert_eq!(TransferRules::T2_POWER_THRESHOLD, 75.0);
        assert_eq!(TransferRules::T3_POWER_THRESHOLD, 60.0);
    }

    #[test]
    fn can_join_boundaries() {
        assert!(TransferRules::can_join(85.0, TeamTier::T1));
        assert!(!TransferRules::can_join(84.9, TeamTier::T1));
        assert!(TransferRules::can_join(75.0, TeamTier::T2));
        assert!(!TransferRules::can_join(74.9, TeamTier::T2));
        assert!(TransferRules::can_join(60.0, TeamTier::T3));
        assert!(!TransferRules::can_join(59.9, TeamTier::T3));
        assert!(TransferRules::can_join(0.0, TeamTier::T4), "T4 无门槛");
    }

    #[test]
    fn max_joinable_tier_ascending() {
        assert_eq!(TransferRules::max_joinable_tier(90.0), TeamTier::T1);
        assert_eq!(TransferRules::max_joinable_tier(85.0), TeamTier::T1);
        assert_eq!(TransferRules::max_joinable_tier(80.0), TeamTier::T2);
        assert_eq!(TransferRules::max_joinable_tier(65.0), TeamTier::T3);
        assert_eq!(TransferRules::max_joinable_tier(50.0), TeamTier::T4);
    }

    #[test]
    fn contract_duration_by_power() {
        assert_eq!(TransferRules::contract_duration(95.0), 4);
        assert_eq!(TransferRules::contract_duration(90.0), 4);
        assert_eq!(TransferRules::contract_duration(85.0), 3);
        assert_eq!(TransferRules::contract_duration(75.0), 2);
        assert_eq!(TransferRules::contract_duration(69.9), 1);
    }

    #[test]
    fn salary_with_reputation_premium() {
        // 2026 修订公式：base 4000/点 + power>75 平方溢价 + 声誉溢价
        // power 80、reputation 50 → 80×4000 + 5²×1200 = 320,000 + 30,000 = 350,000
        assert_eq!(TransferRules::salary_of(80.0, 50), 350_000);
        // reputation 60 → +20,000
        assert_eq!(TransferRules::salary_of(80.0, 60), 370_000);
        // reputation 40 → -20,000
        assert_eq!(TransferRules::salary_of(80.0, 40), 330_000);
        // 顶级溢价拉开差距：90 vs 69 应 >2 倍
        let rookie = TransferRules::salary_of(69.0, 50);
        let star = TransferRules::salary_of(90.0, 50);
        assert!(
            star > rookie * 2,
            "顶级选手薪资应显著拉开: {rookie} vs {star}"
        );
        // 不会为负
        assert!(TransferRules::salary_of(0.0, 0) >= 0);
    }
}
