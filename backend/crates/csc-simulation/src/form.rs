//! 状态/士气模型（Kotlin `FormModel.kt` 转写）——胜负与荣誉 → 士气与声誉的**因果反馈闭环**。

use csc_entities::character::PlayerCharacter;

/// 状态/士气模型——纯计算，不持有状态；由结算层在每场 series 后调用。
pub struct FormModel;

impl FormModel {
    /// 胜方士气增量（每场系列赛）
    pub const MORALE_WIN_DELTA: i32 = 2;
    /// 败方士气减量（每场系列赛）
    pub const MORALE_LOSS_DELTA: i32 = -3;
    /// 胜场声誉增量（仅玩家；负场不扣声誉，避免负反馈螺旋）。
    ///
    /// **2026 试玩修订**：1 → 0——此前每场小比赛胜利 +1，T2 月赛让新秀半年内
    /// 声誉 50→100 封顶（节奏失真）。声誉改由「冠军/MVP」驱动 + 年度回归。
    pub const REPUTATION_WIN_DELTA: i32 = 0;
    /// 赛事冠军声誉加成（仅玩家；2026 修订 5→3）
    pub const REPUTATION_CHAMPION_BONUS: i32 = 3;
    /// 赛事 MVP 声誉加成（仅玩家；2026 修订 8→6）
    pub const REPUTATION_MVP_BONUS: i32 = 6;
    /// 年度声誉回归率（跨年结算调用）：每年声誉向基准 50 收敛 15%——
    /// 持续辉煌才能维持高声誉，长期拉胯名声自然回落。
    pub const REPUTATION_YEARLY_DECAY: f64 = 0.85;

    /// 应用一场系列赛的胜负结果：全员士气波动；玩家（胜方）声誉微增。
    pub fn apply_series_outcome(pc: &mut PlayerCharacter, won: bool) {
        let delta = if won {
            Self::MORALE_WIN_DELTA
        } else {
            Self::MORALE_LOSS_DELTA
        };
        pc.pro.morale = (pc.pro.morale + delta).clamp(0, 100);
        if won && let Some(career) = pc.career.as_mut() {
            career.reputation = (career.reputation + Self::REPUTATION_WIN_DELTA).clamp(0, 100);
        }
    }

    /// 年度声誉回归（跨年结算调用；仅玩家）。
    pub fn yearly_reputation_decay(pc: &mut PlayerCharacter) {
        if let Some(career) = pc.career.as_mut() {
            let drifted = 50.0 + (career.reputation - 50) as f64 * Self::REPUTATION_YEARLY_DECAY;
            career.reputation = drifted.round().clamp(0.0, 100.0) as i32;
        }
    }

    /// 赛事冠军声誉加成（仅玩家；由荣誉颁发处调用）。
    pub fn apply_champion_bonus(pc: &mut PlayerCharacter) {
        if let Some(career) = pc.career.as_mut() {
            career.reputation = (career.reputation + Self::REPUTATION_CHAMPION_BONUS).clamp(0, 100);
        }
    }

    /// 赛事 MVP 声誉加成（仅玩家；由荣誉颁发处调用）。
    pub fn apply_mvp_bonus(player: &mut PlayerCharacter) {
        if let Some(career) = player.career.as_mut() {
            career.reputation = (career.reputation + Self::REPUTATION_MVP_BONUS).clamp(0, 100);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::career::CareerInfo;

    fn npc() -> PlayerCharacter {
        csc_entities::character::PlayerCharacter {
            id: csc_util::id::PlayerId(0),
            name: "npc".into(),
            age: 22,
            role: csc_entities::role::Role::Rifler,
            base: csc_entities::attributes::BaseAttributes {
                reaction: 80,
                stability: 80,
                endurance: 80,
                stamina: 80,
                health: 80,
            },
            skill: csc_entities::attributes::SkillAttributes {
                aim: 80,
                leader: 50,
                communication: 50,
                clutch: 70,
            },
            pro: csc_entities::attributes::ProAttributes {
                mentality: 70,
                confidence: 70,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: csc_entities::attributes::WeaponAttributes {
                position: csc_entities::role::Role::Rifler,
                ak: 80,
                awp: 50,
                pistol: 70,
                smoke: 50,
                utility: 50,
            },
            potential: 90,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        }
    }

    fn player() -> PlayerCharacter {
        let mut p = npc();
        p.career = Some(CareerInfo::free_agent_default(2026));
        p
    }

    #[test]
    fn morale_moves_with_outcome() {
        let mut p = npc();
        FormModel::apply_series_outcome(&mut p, true);
        assert_eq!(p.pro.morale, 62);
        FormModel::apply_series_outcome(&mut p, false);
        assert_eq!(p.pro.morale, 59);
        // clamp 0..100
        let mut p = npc();
        p.pro.morale = 0;
        FormModel::apply_series_outcome(&mut p, false);
        assert_eq!(p.pro.morale, 0);
        p.pro.morale = 99;
        FormModel::apply_series_outcome(&mut p, true);
        assert_eq!(p.pro.morale, 100);
    }

    #[test]
    fn reputation_only_for_player() {
        // NPC：无 career，不涨声誉
        let mut n = npc();
        FormModel::apply_series_outcome(&mut n, true);
        assert_eq!(n.career, None);
        // 玩家：小比赛胜场不加声誉（2026 修订），负场不扣
        let mut p = player();
        FormModel::apply_series_outcome(&mut p, true);
        assert_eq!(p.career.as_ref().unwrap().reputation, 50, "胜场不再加声誉");
        FormModel::apply_series_outcome(&mut p, false);
        assert_eq!(p.career.as_ref().unwrap().reputation, 50, "负场不扣声誉");
        // 冠军 +3、MVP +6
        FormModel::apply_champion_bonus(&mut p);
        assert_eq!(p.career.as_ref().unwrap().reputation, 53);
        FormModel::apply_mvp_bonus(&mut p);
        assert_eq!(p.career.as_ref().unwrap().reputation, 59);
    }

    #[test]
    fn yearly_reputation_decays_toward_50() {
        let mut p = player();
        p.career.as_mut().unwrap().reputation = 100;
        FormModel::yearly_reputation_decay(&mut p);
        // 100 → 50 + 50×0.85 = 92.5 → 93（四舍五入）
        assert_eq!(p.career.as_ref().unwrap().reputation, 93);
        // 低声誉同样向 50 回归（不跌穿）
        p.career.as_mut().unwrap().reputation = 20;
        FormModel::yearly_reputation_decay(&mut p);
        assert_eq!(
            p.career.as_ref().unwrap().reputation,
            25,
            "50 + (20-50)×0.85 = 24.5 → 25"
        );
    }
}
