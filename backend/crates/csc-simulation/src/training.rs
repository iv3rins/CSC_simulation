//! 场外养成模型（Kotlin `TrainingModel.kt` 转写 + 2026 深度改造）——训练决策的规则层（纯函数）。
//!
//! 2026 改造（治「训练=打卡按钮」）：训练不再是每月无脑 +1，而是有代价、有回报、
//! 有环境的养成系统：
//! - **精力/疲劳门控**：训练消耗精力（`fatigue` 上升），疲惫时训练效果大打折扣，
//!   硬撑过度训练甚至可能引发伤病；休息（[`TrainingFocus::Rest`]）恢复精力；
//! - **回报递减**：属性越接近潜力，单月提升越小（`diminishing`）；
//! - **年龄学习曲线**：年轻选手成长快、老将趋缓；
//! - **环境影响**：队伍凝聚力高 → 训练氛围好 → 效果加成。

use csc_domain::season_goal::SeasonGoal;
use csc_entities::character::PlayerCharacter;
use csc_entities::power::PowerCalculator;
use csc_util::rng::Xoshiro256StarStar;

/// 训练重点（由月度训练决策点选择；NPC 每月按角色画像自动训练）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrainingFocus {
    Aim,
    Utility,
    Clutch,
    Physical,
    Mental,
    Communication,
    /// 休息恢复：不做专项训练，恢复精力（降低 fatigue）
    Rest,
}

impl TrainingFocus {
    /// 枚举名（= Kotlin `name`；决策选项 id 用，如 "AIM"）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Aim => "AIM",
            Self::Utility => "UTILITY",
            Self::Clutch => "CLUTCH",
            Self::Physical => "PHYSICAL",
            Self::Mental => "MENTAL",
            Self::Communication => "COMMUNICATION",
            Self::Rest => "REST",
        }
    }

    /// 决策选项 id → 训练重点（= Kotlin `TrainingFocus.valueOf`；未知 id 回退 Aim）。
    pub fn from_name(name: &str) -> Self {
        match name {
            "UTILITY" => Self::Utility,
            "CLUTCH" => Self::Clutch,
            "PHYSICAL" => Self::Physical,
            "MENTAL" => Self::Mental,
            "COMMUNICATION" => Self::Communication,
            "REST" => Self::Rest,
            _ => Self::Aim, // 含 "AIM" 与未知（防御）
        }
    }

    /// 中文标签（= Kotlin `label`）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Aim => "瞄准特训",
            Self::Utility => "道具训练",
            Self::Clutch => "残局特训",
            Self::Physical => "体能训练",
            Self::Mental => "心理训练",
            Self::Communication => "沟通训练",
            Self::Rest => "休整恢复",
        }
    }

    /// 中文描述（= Kotlin `description`）。
    pub const fn description(self) -> &'static str {
        match self {
            Self::Aim => "提升瞄准与枪法（消耗精力）",
            Self::Utility => "提升道具与战术配合（消耗精力）",
            Self::Clutch => "提升残局处理与关键时刻（消耗精力）",
            Self::Physical => "提升体能/精力/健康，降低伤病风险（消耗精力）",
            Self::Mental => "提升抗压/自信/士气（消耗精力）",
            Self::Communication => "提升沟通与团队协作（消耗精力）",
            Self::Rest => "不做专项训练，恢复精力、降低疲劳",
        }
    }
}

/// 训练环境上下文（训练氛围的输入）。
#[derive(Debug, Clone, Copy)]
pub struct TrainingContext {
    /// 团队凝聚力（0..100；越高训练氛围越好，效果加成）
    pub team_cohesion: f64,
    /// 赛季目标（None = 未选择；训练调制依据）
    pub season_goal: Option<SeasonGoal>,
}

impl TrainingContext {
    /// 中性环境（无队伍/自由身）。
    pub fn neutral() -> Self {
        Self {
            team_cohesion: 50.0,
            season_goal: None,
        }
    }
}

/// 单月训练结果。
#[derive(Debug, Clone, Copy)]
pub struct TrainingOutcome {
    /// 是否发生了属性/精力变化
    pub changed: bool,
    /// 本次训练实际获得的属性点（Rest 为 0）
    pub gain: i32,
    /// 疲劳变化（+消耗 / -恢复）
    pub fatigue_delta: f64,
    /// 是否因过度训练触发了伤病风险（体力透支）
    pub overtraining: bool,
}

/// 场外养成模型——训练重点每月小幅提升对应属性组，clamp 到潜力上限。
pub struct TrainingModel;

impl TrainingModel {
    /// 每月属性增幅（基础值，受精力/年龄/递减/环境调制）
    pub const MONTHLY_GAIN: i32 = 1;
    /// 刻苦训练触发概率（+1 → +2，且可累积 HARD_WORKER 印记的判定依据）
    pub const HARD_WORK_CHANCE: f64 = 0.15;
    /// 训练一次的基础疲劳消耗（≈ 一场 bo3 的疲劳量级；高强度训练是体力负担）
    pub const FATIGUE_COST: f64 = 12.0;
    /// 休息一次的基础疲劳恢复
    pub const REST_RECOVERY: f64 = 22.0;
    /// 过度训练阈值：疲劳 > 此值硬撑训练 → 伤病风险
    pub const OVERTRAIN_THRESHOLD: f64 = 80.0;
    /// 过度训练的伤病概率（疲惫硬撑）
    pub const OVERTRAIN_INJURY_CHANCE: f64 = 0.06;
    /// 越疲惫训练效果越差的敏感度
    pub const ENERGY_SENSITIVITY: f64 = 0.7;
    /// 回报递减：属性离潜力越近增益越小（潜力附近的保留比例）
    pub const DIMINISHING_MIN: f64 = 0.25;

    /// 训练效果（返回是否发生了属性变化；= Kotlin `applyTraining` + 2026 深度）。
    ///
    /// 机制：
    /// - **精力/疲劳**：训练消耗 `FATIGUE_COST`，效果按精力（100−fatigue）衰减；
    ///   休息（Rest）恢复 `REST_RECOVERY` 疲劳；
    /// - **过度训练**：疲劳已高于阈值仍专项训练 → 效果近乎为零 + 伤病风险；
    /// - **回报递减**：目标属性接近潜力时增益按 [`Self::diminishing`] 收缩；
    /// - **年龄曲线**：年轻学得快、老将趋缓（[`Self::age_factor`]）；
    /// - **环境**：队伍凝聚力加权（[`TrainingContext`]）。
    ///
    /// 随机消耗：1 个 next_u64（HARD_WORK 判定，整数化 roll_bp）。
    pub fn apply_training(
        pc: &mut PlayerCharacter,
        focus: TrainingFocus,
        rng: &mut Xoshiro256StarStar,
        ctx: TrainingContext,
    ) -> TrainingOutcome {
        // 休息分支：恢复精力，不做专项训练；赛季目标「恢复状态」加成。
        if focus == TrainingFocus::Rest {
            return Self::rest(pc, ctx.season_goal == Some(SeasonGoal::RecoverForm));
        }

        let energy = (1.0 - pc.fatigue / 100.0).clamp(0.0, 1.0);
        let overtraining = pc.fatigue >= Self::OVERTRAIN_THRESHOLD;
        // 过度训练伤病风险（疲劳阈值之上硬撑）
        if overtraining && rng.roll_bp(Self::OVERTRAIN_INJURY_CHANCE) {
            // 触发轻度伤病（肩/腕疲劳；用已有 injury 结构）
            if pc.injury.is_none() {
                use csc_entities::injury::{Injury, InjuryKind, InjurySeverity};
                pc.injury = Some(Injury {
                    kind: InjuryKind::Shoulder,
                    severity: InjurySeverity::Minor,
                    days_left: InjurySeverity::Minor.recovery_days(),
                    sustained_date: String::new(),
                    source: "过度训练".to_string(),
                    ticked: false,
                });
            }
        }

        // 有效增益 = 基础 × 精力 × 年龄 × 递减 × 环境
        let stamina = pc.base.stamina as f64 / 100.0; // 精力上限（低精力体质恢复慢、训练效率低）
        let energy_factor = if overtraining {
            0.0 // 体力透支，训练无效
        } else {
            // 当前状态（1−疲劳）与精力体质共同决定效率
            let rest = (energy * (1.0 - Self::ENERGY_SENSITIVITY) + Self::ENERGY_SENSITIVITY)
                .clamp(0.05, 1.0);
            (rest * (0.4 + 0.6 * stamina)).clamp(0.05, 1.0)
        };
        let age_factor = Self::age_factor(pc.age);
        let mut env_factor =
            (0.85 + ctx.team_cohesion.clamp(0.0, 100.0) / 100.0 * 0.3).clamp(0.85, 1.15);
        // 赛季目标调制（Sprint 3）：冲 Rating 时枪法/道具/残局训练更高效。
        if ctx.season_goal == Some(SeasonGoal::ImproveRating)
            && matches!(
                focus,
                TrainingFocus::Aim | TrainingFocus::Utility | TrainingFocus::Clutch
            )
        {
            env_factor *= 1.10;
        }

        let gain = if rng.roll_bp(Self::HARD_WORK_CHANCE) {
            Self::MONTHLY_GAIN + 1
        } else {
            Self::MONTHLY_GAIN
        };

        let before = PowerCalculator::player_power(pc);
        let potential = pc.potential;
        match focus {
            TrainingFocus::Aim => {
                let d = Self::diminishing(pc.skill.aim, potential);
                pc.skill.aim = Self::grow(
                    pc.skill.aim,
                    potential,
                    Self::scale(gain, d, energy_factor, age_factor, env_factor),
                );
            }
            TrainingFocus::Utility => {
                let d = Self::diminishing((pc.weapon.smoke + pc.weapon.utility) / 2, potential);
                let g = Self::scale(gain, d, energy_factor, age_factor, env_factor);
                pc.weapon.smoke = Self::grow(pc.weapon.smoke, potential, g);
                pc.weapon.utility = Self::grow(pc.weapon.utility, potential, g);
            }
            TrainingFocus::Clutch => {
                let d = Self::diminishing(pc.skill.clutch, potential);
                pc.skill.clutch = Self::grow(
                    pc.skill.clutch,
                    potential,
                    Self::scale(gain, d, energy_factor, age_factor, env_factor),
                );
            }
            TrainingFocus::Physical => {
                let d = Self::diminishing(
                    (pc.base.endurance + pc.base.stamina + pc.base.health) / 3,
                    potential,
                );
                let g = Self::scale(gain, d, energy_factor, age_factor, env_factor);
                pc.base.endurance = Self::grow(pc.base.endurance, potential, g);
                pc.base.stamina = Self::grow(pc.base.stamina, potential, g);
                pc.base.health = Self::grow(pc.base.health, potential, g);
            }
            TrainingFocus::Mental => {
                let d = Self::diminishing((pc.pro.mentality + pc.pro.confidence) / 2, potential);
                let g = Self::scale(gain, d, energy_factor, age_factor, env_factor);
                pc.pro.mentality = Self::grow(pc.pro.mentality, potential, g);
                pc.pro.confidence = Self::grow(pc.pro.confidence, potential, g);
            }
            TrainingFocus::Communication => {
                let d =
                    Self::diminishing((pc.skill.communication + pc.skill.leader) / 2, potential);
                let g = Self::scale(gain, d, energy_factor, age_factor, env_factor);
                pc.skill.communication = Self::grow(pc.skill.communication, potential, g);
                pc.skill.leader = Self::grow(pc.skill.leader, potential, g);
            }
            TrainingFocus::Rest => unreachable!(), // 已在上面处理
        }

        let changed = PowerCalculator::player_power(pc) != before;
        let fatigue_delta = if overtraining {
            // 硬撑训练：继续透支，额外疲劳
            Self::FATIGUE_COST + 6.0
        } else {
            Self::FATIGUE_COST
        };
        pc.fatigue = (pc.fatigue + fatigue_delta).min(100.0);
        TrainingOutcome {
            changed,
            gain: if changed { gain } else { 0 },
            fatigue_delta,
            overtraining,
        }
    }

    /// 休息恢复（降低疲劳）。
    fn rest(pc: &mut PlayerCharacter, recovery_goal: bool) -> TrainingOutcome {
        let before = pc.fatigue;
        let recovery = if recovery_goal {
            Self::REST_RECOVERY * 1.25
        } else {
            Self::REST_RECOVERY
        };
        pc.fatigue = (pc.fatigue - recovery).max(0.0);
        TrainingOutcome {
            changed: pc.fatigue != before,
            gain: 0,
            fatigue_delta: -(recovery),
            overtraining: false,
        }
    }

    /// 回报递减：离潜力越近增益越小（接近潜力时保留最小比例）。
    fn diminishing(value: i32, potential: i32) -> f64 {
        let room = (potential - value).max(0) as f64;
        // 潜力附近 20 点内开始收缩
        let ratio = (room / 20.0).clamp(0.0, 1.0);
        Self::DIMINISHING_MIN + ratio * (1.0 - Self::DIMINISHING_MIN)
    }

    /// 年龄学习曲线：年轻学得快、老将趋缓。
    fn age_factor(age: i32) -> f64 {
        match age {
            a if a <= 20 => 1.35,
            a if a <= 23 => 1.15,
            a if a <= 26 => 1.0,
            a if a <= 30 => 0.8,
            _ => 0.6,
        }
    }

    /// 综合缩放（把整数 gain 按因子缩放）。
    ///
    /// 正常训练（离潜力远、精力足）保底 +1；只有**接近潜力**（回报递减）或
    /// **体力透支**时才可能归零——这才不是「每月必有 +1」的打卡按钮。
    fn scale(gain: i32, d: f64, energy: f64, age: f64, env: f64) -> i32 {
        let raw = gain as f64 * d * energy * age * env;
        if d < Self::DIMINISHING_MIN + 0.15 || energy < 0.1 {
            // 接近潜力 / 透支 → 可能归零
            raw.round().max(0.0) as i32
        } else {
            raw.round().max(1.0) as i32
        }
    }

    /// NPC 自动训练重点：按角色画像的短板补强（无决策，纯规则；= Kotlin `autoFocusFor`）。
    pub fn auto_focus_for(pc: &PlayerCharacter) -> TrainingFocus {
        if pc.fatigue >= Self::OVERTRAIN_THRESHOLD {
            return TrainingFocus::Rest; // 体力透支先休息
        }
        if pc.base.health < 75 {
            TrainingFocus::Physical
        } else if pc.pro.mentality < 70 || pc.pro.confidence < 70 {
            TrainingFocus::Mental
        } else if pc.skill.communication < 70 || pc.skill.leader < 70 {
            TrainingFocus::Communication
        } else if pc.weapon.utility < 70 {
            TrainingFocus::Utility
        } else if pc.skill.clutch < 70 {
            TrainingFocus::Clutch
        } else {
            TrainingFocus::Aim
        }
    }

    /// 向潜力收敛的单属性增幅（已达潜力则不再增长——训练只增不减；= Kotlin `grow`）。
    fn grow(value: i32, potential: i32, gain: i32) -> i32 {
        if value >= potential || gain <= 0 {
            value
        } else {
            (value + gain).min(potential)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::attributes::{
        BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
    };
    use csc_entities::role::Role;

    fn pc() -> PlayerCharacter {
        PlayerCharacter {
            id: csc_util::id::PlayerId(0),
            name: "t".into(),
            age: 20,
            role: Role::Rifler,
            base: BaseAttributes {
                reaction: 50,
                stability: 50,
                endurance: 50,
                stamina: 50,
                health: 50,
            },
            skill: SkillAttributes {
                aim: 50,
                leader: 50,
                communication: 50,
                clutch: 50,
            },
            pro: ProAttributes {
                mentality: 50,
                confidence: 50,
                team_spirit: 50,
                loyalty: 50,
                morale: 50,
            },
            weapon: WeaponAttributes {
                position: Role::Rifler,
                ak: 50,
                awp: 50,
                pistol: 50,
                smoke: 50,
                utility: 50,
            },
            potential: 80,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        }
    }

    #[test]
    fn labels_match_kotlin() {
        assert_eq!(TrainingFocus::Aim.label(), "瞄准特训");
        assert_eq!(TrainingFocus::Rest.label(), "休整恢复");
        assert_eq!(
            TrainingFocus::Physical.description(),
            "提升体能/精力/健康，降低伤病风险（消耗精力）"
        );
    }

    #[test]
    fn aim_training_raises_aim_only_and_costs_fatigue() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        let before = p.skill.aim;
        TrainingModel::apply_training(
            &mut p,
            TrainingFocus::Aim,
            &mut rng,
            TrainingContext::neutral(),
        );
        assert!(p.skill.aim > before);
        // 训练消耗精力（疲劳上升）
        assert!(p.fatigue > 0.0, "训练应消耗精力: fatigue={}", p.fatigue);
        // 其它属性不变
        assert_eq!(p.base.reaction, 50);
        assert_eq!(p.weapon.ak, 50);
    }

    #[test]
    fn training_caps_at_potential() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        p.skill.aim = 79; // 潜力 80
        TrainingModel::apply_training(
            &mut p,
            TrainingFocus::Aim,
            &mut rng,
            TrainingContext::neutral(),
        );
        assert!(p.skill.aim <= 80, "不得超过潜力");
    }

    #[test]
    fn training_never_reduces() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        p.skill.aim = 90; // 超潜力（历史 bug 场景）
        TrainingModel::apply_training(
            &mut p,
            TrainingFocus::Aim,
            &mut rng,
            TrainingContext::neutral(),
        );
        assert_eq!(p.skill.aim, 90, "超潜力属性不应被拉低");
    }

    #[test]
    fn rest_reduces_fatigue_without_attr_gain() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        p.fatigue = 60.0;
        let before = p.skill.aim;
        let out = TrainingModel::apply_training(
            &mut p,
            TrainingFocus::Rest,
            &mut rng,
            TrainingContext::neutral(),
        );
        assert!(p.fatigue < 60.0, "休息应恢复精力: fatigue={}", p.fatigue);
        assert_eq!(p.skill.aim, before, "休息不改属性");
        assert!(out.fatigue_delta < 0.0, "休息疲劳为负（恢复）");
        assert!(!out.overtraining);
    }

    #[test]
    fn overtraining_when_exhausted_grants_almost_no_gain() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        p.fatigue = 90.0; // 体力透支仍硬撑训练
        let before = p.skill.aim;
        let out = TrainingModel::apply_training(
            &mut p,
            TrainingFocus::Aim,
            &mut rng,
            TrainingContext::neutral(),
        );
        assert!(out.overtraining, "疲劳 90 训练应标记过度训练");
        assert_eq!(p.skill.aim, before, "体力透支训练几乎无效果");
    }

    #[test]
    fn diminishing_returns_near_potential() {
        // 属性接近潜力 → 单月增益可能为 0（不再是「必有 +1」）
        let mut rng = Xoshiro256StarStar::seed(1);
        let mut p = pc();
        p.skill.aim = 79; // 潜力 80，只差 1
        // 多次训练都应收敛，且不会超过潜力
        for _ in 0..5 {
            TrainingModel::apply_training(
                &mut p,
                TrainingFocus::Aim,
                &mut rng,
                TrainingContext::neutral(),
            );
        }
        assert!(p.skill.aim <= 80, "不应超过潜力");
        // 低增益场景：疲惫时接近潜力几乎无提升
        let mut p2 = pc();
        p2.skill.aim = 79;
        p2.fatigue = 60.0;
        TrainingModel::apply_training(
            &mut p2,
            TrainingFocus::Aim,
            &mut rng,
            TrainingContext::neutral(),
        );
        // 只要不超潜力即可（增益可能 0 或 1）
        assert!(p2.skill.aim <= 80);
    }

    #[test]
    fn auto_focus_priority() {
        let mut p = pc();
        // 体力透支 → 先休息
        p.fatigue = 90.0;
        assert_eq!(TrainingModel::auto_focus_for(&p), TrainingFocus::Rest);
        // 健康低 → PHYSICAL
        let mut p = pc();
        p.base.health = 60;
        assert_eq!(TrainingModel::auto_focus_for(&p), TrainingFocus::Physical);
        // 心态低 → MENTAL（health 已高，避免 PHYSICAL 优先）
        let mut p = pc();
        p.base.health = 80;
        p.pro.mentality = 60;
        assert_eq!(TrainingModel::auto_focus_for(&p), TrainingFocus::Mental);
        // 全满 → AIM
        let mut full = pc();
        full.base.health = 80;
        full.pro.mentality = 80;
        full.pro.confidence = 80;
        full.skill.communication = 80;
        full.skill.leader = 80;
        full.weapon.utility = 80;
        full.skill.clutch = 80;
        assert_eq!(TrainingModel::auto_focus_for(&full), TrainingFocus::Aim);
    }
}
