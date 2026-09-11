//! 实力计算器（Kotlin `PowerCalculator.kt` 的转写）。

use crate::attributes::{BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes};
use crate::character::PlayerCharacter;
use crate::role::Role;
use crate::role_profile::{Attr, RoleProfiles};

/// 实力计算器。
///
/// 差异化设计：`player_power` 按选手角色的偏好向量加权——AWP 权重高、IGL 的
/// 战术权重高，个体角色直接贡献到实力值，而不是把所有属性平均成「六边形」。
///
/// 位级一致性：全部运算为 IEEE754 精确的 + - * /（无 libm 超越函数），
/// Kotlin 与 Rust 输出逐位一致（golden 测试锁死）。
pub struct PowerCalculator;

impl PowerCalculator {
    /// 单个选手综合实力（0..100；= Kotlin `playerPower(PlayerCharacter)`）。
    ///
    /// 按角色偏好加权：power = Σ(pref_i · value_i) / Σ(pref_i)。
    pub fn player_power(pc: &PlayerCharacter) -> f64 {
        Self::player_power_components(pc.role, &pc.base, &pc.skill, &pc.pro, &pc.weapon)
    }

    /// 属性组版综合实力（0..100）：供生成阶段（尚未构造实体）与成长曲线使用；
    /// 语义与 [`Self::player_power`]（角色加权）完全一致。
    pub fn player_power_components(
        role: Role,
        base: &BaseAttributes,
        skill: &SkillAttributes,
        pro: &ProAttributes,
        weapon: &WeaponAttributes,
    ) -> f64 {
        let profile = RoleProfiles::of(role); // 选手角色的属性偏好向量
        let mut weighted_sum = 0.0; // 加权和（pref × value 累加）
        let mut weight_sum = 0.0; // 权重和（pref 累加）

        // 把单个属性按偏好权重加入加权和
        macro_rules! add {
            ($attr:expr, $value:expr) => {{
                let w = profile.pref($attr);
                weighted_sum += w * $value as f64;
                weight_sum += w;
            }};
        }

        // —— 基础属性组 ——
        add!(Attr::Reaction, base.reaction);
        add!(Attr::Stability, base.stability);
        add!(Attr::Endurance, base.endurance);
        add!(Attr::Stamina, base.stamina);
        add!(Attr::Health, base.health);
        // —— 技能属性组 ——
        add!(Attr::Aim, skill.aim);
        add!(Attr::Leader, skill.leader);
        add!(Attr::Communication, skill.communication);
        add!(Attr::Clutch, skill.clutch);
        // —— 心理属性组 ——
        add!(Attr::Mentality, pro.mentality);
        add!(Attr::Confidence, pro.confidence);
        add!(Attr::TeamSpirit, pro.team_spirit);
        add!(Attr::Loyalty, pro.loyalty);
        add!(Attr::Morale, pro.morale);
        // —— 武器熟练度组 ——
        add!(Attr::Ak, weapon.ak);
        add!(Attr::Awp, weapon.awp);
        add!(Attr::Pistol, weapon.pistol);
        add!(Attr::Smoke, weapon.smoke);
        add!(Attr::Utility, weapon.utility);

        // 加权平均；权重为 0（理论不可能）时兜底返回 0
        if weight_sum == 0.0 {
            0.0
        } else {
            weighted_sum / weight_sum
        }
    }

    /// 队伍的角色构成（便于调试 / 展示队伍风格；= Kotlin `roleComposition`）。
    pub fn role_composition(roster: &[&PlayerCharacter]) -> [i32; 6] {
        let mut counts = [0i32; 6];
        for pc in roster {
            counts[pc.role as usize] += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::PlayerCharacter;
    use crate::injury::Injury;
    use csc_util::id::PlayerId;

    fn pc(role: Role) -> PlayerCharacter {
        PlayerCharacter {
            id: PlayerId(0),
            name: "t".into(),
            age: 20,
            role,
            base: BaseAttributes {
                reaction: 80,
                stability: 80,
                endurance: 80,
                stamina: 80,
                health: 80,
            },
            skill: SkillAttributes {
                aim: 80,
                leader: 50,
                communication: 50,
                clutch: 70,
            },
            pro: ProAttributes {
                mentality: 70,
                confidence: 70,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: crate::attributes::WeaponAttributes {
                position: role,
                ak: 60,
                awp: 90,
                pistol: 80,
                smoke: 50,
                utility: 50,
            },
            potential: 80,
            fatigue: 0.0,
            injury: None::<Injury>,
            career: None,
            team: None,
            retired: false,
        }
    }

    #[test]
    fn player_power_role_weighted() {
        // AWP 选手：awp=90 高偏好 → 实力应高于把属性平均（六边形 ~70）
        let awper = pc(Role::Awp);
        let power = PowerCalculator::player_power(&awper);
        assert!(power > 70.0, "AWP 加权实力应高于平均: {power}");
        assert!(power <= 100.0);
    }

    #[test]
    fn player_power_components_equals_entity_version() {
        let p = pc(Role::Igl);
        let a = PowerCalculator::player_power(&p);
        let b =
            PowerCalculator::player_power_components(p.role, &p.base, &p.skill, &p.pro, &p.weapon);
        assert_eq!(a, b);
    }

    #[test]
    fn role_composition_counts() {
        let roster = [pc(Role::Igl), pc(Role::Awp), pc(Role::Igl)];
        let refs: Vec<&PlayerCharacter> = roster.iter().collect();
        let comp = PowerCalculator::role_composition(&refs);
        assert_eq!(comp[Role::Igl as usize], 2);
        assert_eq!(comp[Role::Awp as usize], 1);
        assert_eq!(comp.iter().sum::<i32>(), 3);
    }

    /// 跨语言 golden：Kotlin `PowerCalculator` 权威输出（tools/gen_entities_golden.kt，
    /// 固定属性组 base(80×5)/skill(80,50,50,70)/pro(70,70,60,50,60)/weapon(60,90,80,50,50)）。
    #[test]
    fn golden_player_power_bits() {
        let base = BaseAttributes {
            reaction: 80,
            stability: 80,
            endurance: 80,
            stamina: 80,
            health: 80,
        };
        let skill = SkillAttributes {
            aim: 80,
            leader: 50,
            communication: 50,
            clutch: 70,
        };
        let pro = ProAttributes {
            mentality: 70,
            confidence: 70,
            team_spirit: 60,
            loyalty: 50,
            morale: 60,
        };
        let weapon = crate::attributes::WeaponAttributes {
            position: Role::Awp,
            ak: 60,
            awp: 90,
            pistol: 80,
            smoke: 50,
            utility: 50,
        };
        // Kotlin toRawBits：IGL=40502bdd2b899407、AWP=4051f202a4caaf5e、RIFLER=405167b23a5440cf
        // （2026 IGL 校准：指挥 aim/ak/awp 偏好下调 0.40/0.35/0.30 → 0.18/0.18/0.15，
        //   IGL 综合实力更偏战术而非枪械，IGL 加权 power 位模式随之更新）
        assert_eq!(
            PowerCalculator::player_power_components(Role::Igl, &base, &skill, &pro, &weapon)
                .to_bits(),
            0x4050_2BDD_2B89_9407
        );
        assert_eq!(
            PowerCalculator::player_power_components(Role::Awp, &base, &skill, &pro, &weapon)
                .to_bits(),
            0x4051_f202_a4ca_af5e
        );
        assert_eq!(
            PowerCalculator::player_power_components(Role::Rifler, &base, &skill, &pro, &weapon)
                .to_bits(),
            0x4051_67b2_3a54_40cf
        );
    }
}
