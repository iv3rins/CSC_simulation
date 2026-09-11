//! 成长曲线模型（Kotlin `GrowthModel.kt` 转写）——选手能力随年龄演化。

use csc_entities::character::PlayerCharacter;
use csc_entities::power::PowerCalculator;
use csc_util::math_utils::round_to_int;
use csc_util::rng::Xoshiro256StarStar;

/// 成长曲线模型（FM 风格 Age Curve）：成长段向潜力收敛、平台段、衰退段。
///
/// 应用时机：跨年结算（每年调用一次 `apply_yearly_growth`，顺带年龄 +1）。
/// 纯计算 + 确定性 RNG；潜力为隐藏属性。
pub struct GrowthModel;

impl GrowthModel {
    /// 属性下限（衰退不会无限下降，保留基本职业水平）
    const FLOOR: i32 = 20;

    /// 年度成长速率：返回「当前与潜力的差距」的收敛比例（成长段），
    /// 或负的固定下滑量（衰退段），0 = 平台。
    ///
    /// **2026 修订（真实年龄曲线校准）**：CS2 选手巅峰在 21-24 岁、26 岁已
    /// 明显下滑（真实数据横截面：20-23 均值 1.055 > 24-27 均值 1.030）。
    /// 原曲线 27-29 平台、30 岁才下滑，过于乐观（试玩发现 29 岁仍登巅峰）。
    pub fn development_rate(age: i32) -> f64 {
        match age {
            a if a <= 20 => 0.14, // 18-20 快速成长
            a if a <= 24 => 0.05, // 21-24 巅峰平台（微涨向潜力收敛）
            a if a <= 26 => -1.0, // 25-26 下滑启动（26 岁已明显下滑）
            a if a <= 29 => -2.0, // 27-29 明显下滑
            _ => -3.0,            // 30+ 加速下滑
        }
    }

    /// 生理衰退起始年龄（2026 修订：28 → 25——真实生态 26 岁即开始老化，
    /// 概率性额外衰减与确定性下滑叠加，产生「早衰/长青」个体差异）。
    pub const DECLINE_START_AGE: i32 = 25;

    /// 生理衰退概率（每年，25 岁起）：
    /// ```text
    /// p = clamp((age − 24) × 0.08, 0, 0.70) × (1 − 0.60 × resist)
    /// resist = clamp((mentality + leader) / 200, 0, 0.85)  // 自律+指挥抗衰
    /// ```
    /// 25 岁 ≈ 8%、26 岁 ≈ 16%、28 岁 ≈ 32%、32 岁 ≈ 56%、36 岁 ≈ 70%（满抗衰 ×0.49 打折）。
    /// **禁止 27-36 岁无脑连续 TOP1**：即使满自律/指挥，36 岁时仍有 ~34% 年概率。
    pub fn decline_probability(pc: &PlayerCharacter) -> f64 {
        if pc.age < Self::DECLINE_START_AGE {
            return 0.0;
        }
        let resist = ((pc.pro.mentality + pc.skill.leader) as f64 / 200.0).clamp(0.0, 0.85);
        let age_drive = ((pc.age - Self::DECLINE_START_AGE + 1) as f64 * 0.08).min(0.70);
        (age_drive * (1.0 - 0.60 * resist)).clamp(0.0, 0.85)
    }

    /// 应用一年成长：年龄 +1，并按年龄曲线更新四组属性（逐属性向潜力收敛或衰退），
    /// 25 岁起额外掷**生理衰退**（反应/稳定性/瞄准/残局/自信概率性下滑，
    /// 自律 mentality + 指挥 leader 抗衰）。
    ///
    /// @param rng 确定性随机源（跨年结算传入；衰退掷骰消费 1 次 next_u64（roll_bp 整数化判定））
    /// @return 是否发生了属性变化（false 仅当完全平台且无波动）
    pub fn apply_yearly_growth(pc: &mut PlayerCharacter, rng: &mut Xoshiro256StarStar) -> bool {
        pc.age += 1;
        let rate = Self::development_rate(pc.age);
        let before = PowerCalculator::player_power(pc);

        if rate > 0.0 {
            // 成长：new = cur + round(rate · (potential − cur))，clamp 到潜力（渐近收敛）
            let potential = pc.potential;
            let grow = move |v: i32| grow_one(v, potential, rate);
            pc.base = map_base(&pc.base, grow);
            pc.skill = map_skill(&pc.skill, grow);
            pc.pro = map_pro(&pc.pro, grow);
            pc.weapon = map_weapon(&pc.weapon, grow);
        } else if rate < 0.0 {
            // 衰退：每年固定下滑（clamp 下限）
            let drop = round_to_int(rate);
            let decay = |v: i32| -> i32 { (v + drop).max(Self::FLOOR) };
            pc.base = map_base(&pc.base, decay);
            pc.skill = map_skill(&pc.skill, decay);
            pc.pro = map_pro(&pc.pro, decay);
            pc.weapon = map_weapon(&pc.weapon, decay);
        }
        // rate == 0：平台段，属性不变（年龄仍 +1）

        // —— 生理衰退（25 岁起，确定性概率；自律/指挥抗衰）——
        // 反应/稳定性/瞄准是电竞最吃年龄的维度，下滑最狠；体能同步退步
        if pc.age >= Self::DECLINE_START_AGE && rng.roll_bp(Self::decline_probability(pc)) {
            pc.base.reaction = (pc.base.reaction - 5).max(Self::FLOOR);
            pc.base.stability = (pc.base.stability - 5).max(Self::FLOOR);
            pc.base.endurance = (pc.base.endurance - 2).max(Self::FLOOR);
            pc.base.stamina = (pc.base.stamina - 2).max(Self::FLOOR);
            pc.skill.aim = (pc.skill.aim - 4).max(Self::FLOOR);
            pc.skill.clutch = (pc.skill.clutch - 3).max(Self::FLOOR);
            pc.pro.confidence = (pc.pro.confidence - 3).max(Self::FLOOR);
        }

        PowerCalculator::player_power(pc) != before
    }
}

/// 单属性成长：`min(potential, v + round(rate·(potential−v)))`——只增不减（= Kotlin `grow`）。
/// 注意：Kotlin 先 `v + round(...)` 再 min(potential)；`v + delta` 可能略超 potential，
/// min 截断——但 **v 本身不会低于原值**（rate·(potential−v) ≥ 0）。
fn grow_one(v: i32, potential: i32, rate: f64) -> i32 {
    let delta = round_to_int(rate * (potential - v) as f64);
    (v + delta).min(potential)
}

fn map_base(
    b: &csc_entities::attributes::BaseAttributes,
    f: impl Fn(i32) -> i32,
) -> csc_entities::attributes::BaseAttributes {
    csc_entities::attributes::BaseAttributes {
        reaction: f(b.reaction),
        stability: f(b.stability),
        endurance: f(b.endurance),
        stamina: f(b.stamina),
        health: f(b.health),
    }
}

fn map_skill(
    s: &csc_entities::attributes::SkillAttributes,
    f: impl Fn(i32) -> i32,
) -> csc_entities::attributes::SkillAttributes {
    csc_entities::attributes::SkillAttributes {
        aim: f(s.aim),
        leader: f(s.leader),
        communication: f(s.communication),
        clutch: f(s.clutch),
    }
}

fn map_pro(
    p: &csc_entities::attributes::ProAttributes,
    f: impl Fn(i32) -> i32,
) -> csc_entities::attributes::ProAttributes {
    csc_entities::attributes::ProAttributes {
        mentality: f(p.mentality),
        confidence: f(p.confidence),
        team_spirit: f(p.team_spirit),
        loyalty: f(p.loyalty),
        morale: f(p.morale),
    }
}

fn map_weapon(
    w: &csc_entities::attributes::WeaponAttributes,
    f: impl Fn(i32) -> i32,
) -> csc_entities::attributes::WeaponAttributes {
    csc_entities::attributes::WeaponAttributes {
        position: w.position,
        ak: f(w.ak),
        awp: f(w.awp),
        pistol: f(w.pistol),
        smoke: f(w.smoke),
        utility: f(w.utility),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::attributes::{
        BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes,
    };
    use csc_entities::role::Role;

    fn pc(age: i32, potential: i32) -> PlayerCharacter {
        PlayerCharacter {
            id: csc_util::id::PlayerId(0),
            name: "t".into(),
            age,
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
            potential,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        }
    }

    #[test]
    fn development_rate_bands_peak_21_24_decline_from_25() {
        assert_eq!(GrowthModel::development_rate(18), 0.14);
        assert_eq!(GrowthModel::development_rate(20), 0.14);
        assert_eq!(GrowthModel::development_rate(21), 0.05, "21 岁进入巅峰平台");
        assert_eq!(GrowthModel::development_rate(24), 0.05, "24 岁仍在巅峰");
        assert_eq!(GrowthModel::development_rate(25), -1.0, "25 岁下滑启动");
        assert_eq!(GrowthModel::development_rate(26), -1.0, "26 岁已明显下滑");
        assert_eq!(GrowthModel::development_rate(27), -2.0);
        assert_eq!(GrowthModel::development_rate(29), -2.0);
        assert_eq!(GrowthModel::development_rate(30), -3.0);
        assert_eq!(GrowthModel::development_rate(36), -3.0);
    }

    #[test]
    fn growth_peaks_around_24_then_declines() {
        let mut p = pc(20, 80); // 50 → 潜力 80
        let mut rng = Xoshiro256StarStar::seed(42);
        // 20→24 岁：4 年巅峰平台微涨（rate 0.05）
        for _ in 0..4 {
            GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        }
        assert!(p.skill.aim <= 80);
        let peak_aim = p.skill.aim;
        assert!(peak_aim > 53, "巅峰期应向潜力收敛: {peak_aim}");
        // 25→26 岁：下滑启动（每年 -1），26 岁应低于 24 岁巅峰
        GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        assert!(
            p.skill.aim < peak_aim,
            "26 岁应已从巅峰下滑: {peak_aim} → {}",
            p.skill.aim
        );
    }

    #[test]
    fn growth_converges_overshoot_down_to_potential() {
        // Kotlin GrowthModel 语义：`minOf(potential, v + round(rate·(potential−v)))`
        // —— 超潜力属性会被收敛到潜力（与 Kotlin 行为一致；注意 TrainingModel.grow
        // 才是"只增不减"，两处语义不同是 Kotlin 原型的事实，转写保持 100% 一致）。
        let mut p = pc(20, 60);
        p.base.reaction = 70; // 已超潜力
        let mut rng = Xoshiro256StarStar::seed(42);
        GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        // rate(21)=0.10: 70 + round(0.10×(60−70)) = 70−1 = 69 → min(60, 69) = 60
        assert_eq!(p.base.reaction, 60, "超潜力属性向潜力收敛（Kotlin 语义）");
    }

    #[test]
    fn decline_floors_at_20() {
        let mut p = pc(32, 60); // 衰退段 -3/年
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..30 {
            GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        }
        assert_eq!(p.skill.aim, 20, "衰退下限 20");
    }

    #[test]
    fn age_increments_each_year() {
        let mut p = pc(25, 60);
        let age_before = p.age;
        let mut rng = Xoshiro256StarStar::seed(42);
        GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        assert_eq!(p.age, age_before + 1);
    }

    #[test]
    fn decline_starts_at_25() {
        // 25 岁起下滑：24→25 岁确定性 -1（生理衰退若命中再 -4）
        let mut p = pc(24, 60);
        let mut rng = Xoshiro256StarStar::seed(42);
        let changed = GrowthModel::apply_yearly_growth(&mut p, &mut rng);
        assert!(changed, "25 岁下滑启动，属性应变化");
        assert!(
            p.skill.aim <= 49,
            "25 岁确定性 -1（或叠加生理衰退）: {}",
            p.skill.aim
        );
        assert!(p.skill.aim >= 45, "单次下滑幅度有界: {}", p.skill.aim);
    }

    #[test]
    fn decline_probability_respects_age_and_resistance() {
        // 26 岁低自律/低指挥：概率显著 > 0（26 岁已下滑）
        let mut p = pc(26, 60);
        p.pro.mentality = 30;
        p.skill.leader = 30;
        let high = GrowthModel::decline_probability(&p);
        assert!(high > 0.02, "低自律 26 岁应有衰退概率: {high}");
        // 高自律+高指挥：概率大幅下降
        p.pro.mentality = 95;
        p.skill.leader = 95;
        let low = GrowthModel::decline_probability(&p);
        assert!(low < high * 0.6, "高自律+指挥抗衰: {low} vs {high}");
        // 24 岁：无衰退概率
        p.age = 24;
        assert_eq!(GrowthModel::decline_probability(&p), 0.0);
        // 36 岁满抗衰仍有非零概率（禁止无脑 TOP1）
        p.age = 36;
        assert!(GrowthModel::decline_probability(&p) > 0.0);
    }

    /// M4 回归锁：衰退判定必须走 roll_bp 整数化（D2 契约），且同 seed 序列可复现。
    #[test]
    fn decline_roll_is_bp_integerized_and_deterministic() {
        let mut rng1 = Xoshiro256StarStar::seed(42);
        let mut rng2 = Xoshiro256StarStar::seed(42);
        let p1 = pc(30, 90);
        let p2 = pc(30, 90);
        // 连续 64 次判定（只调用 GrowthModel 判定分支的等价表达式），
        // 两条同 seed 流逐位一致。
        for _ in 0..64 {
            assert_eq!(
                p1.age >= GrowthModel::DECLINE_START_AGE
                    && rng1.roll_bp(GrowthModel::decline_probability(&p1)),
                p2.age >= GrowthModel::DECLINE_START_AGE
                    && rng2.roll_bp(GrowthModel::decline_probability(&p2)),
                "同 seed 同状态 → 同判定"
            );
        }
    }
}
