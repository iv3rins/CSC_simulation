//! 虚拟选手生成器（Kotlin `Players.kt` 的转写）。

use csc_domain::tier::{Tier, TierTable};
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;

use crate::attributes::{BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes};
use crate::baseline::{RatingBaseline, RatingProfile};
use crate::career::CareerInfo;
use crate::character::PlayerCharacter;
use crate::power::PowerCalculator;
use crate::role::{Role, random_role};
use crate::role_profile::{Attr, RoleProfiles};

/// 随机选手名池（青训新秀等需要唯一性的调用方配合查重重采样）。
const NAME_POOL: [&str; 40] = [
    "shadow", "Reaper", "Vortex", "Blaze", "Ghost", "Nomad", "Jinx", "Raptor", "Frost", "Cipher",
    "Venom", "Echo", "Kite", "Apex", "Havoc", "Pixel", "Nova", "Talon", "Zephyr", "Onyx", "Rogue",
    "Scout", "Viper", "Wraith", "Ember", "Falcon", "Glitch", "Hollow", "Ion", "Jester", "Kraken",
    "Lyric", "Mirage", "Night", "Oracle", "Phoenix", "Quill", "Raven", "Sable", "Tempo",
];

/// 生成的一名选手的属性与身份（NPC 与玩家共用同一套属性生成逻辑）。
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedPlayer {
    pub player_name: String,
    pub age: i32,
    pub role: Role,
    pub base: BaseAttributes,
    pub skill: SkillAttributes,
    pub pro: ProAttributes,
    pub weapon: WeaponAttributes,
    /// 潜力上限（隐藏属性，开局随机分配，玩家不可见）
    pub potential: i32,
}

/// 虚拟选手生成器：按档位 + 角色画像生成差异化选手。
pub struct RandomPlayerGenerator;

impl RandomPlayerGenerator {
    /// 从名字池随机采样一个名字（供需要唯一性的调用方查重后使用）。
    pub fn random_name(rng: &mut Xoshiro256StarStar) -> String {
        NAME_POOL[rng.next_i32_bound(NAME_POOL.len() as i32) as usize].to_string()
    }

    /// 按档位 + 角色生成选手属性（不含队伍归属；NPC 与玩家共用）。
    ///
    /// **随机调用顺序与 Kotlin 完全一致**（跨语言可复现的前置条件）：
    /// `z`（2 抽）→ 年龄（1 抽）→ 19 属性各 1 抽（reaction → utility）→ 潜力（1 抽）。
    pub fn generate_attributes(
        tier: Tier,
        role: Option<Role>,
        name: Option<&str>,
        rng: &mut Xoshiro256StarStar,
        age: Option<i32>,
    ) -> GeneratedPlayer {
        let r = TierTable::of(tier); // 取档位属性区间
        let chosen_role = role.unwrap_or_else(|| random_role(rng)); // 未指定则随机角色
        let profile = RoleProfiles::of(chosen_role); // 角色画像（偏好 + 波动）
        let player_name = name
            .map(str::to_string)
            .unwrap_or_else(|| Self::random_name(rng));

        // 风格纯度潜变量 z ∈ [0,1]：两个 uniform 的均值近似 Beta(2,2)，
        // 中心约 0.5（平庸轮廓），偶有接近 0/1（极端轮廓）。
        let z = (rng.next_double() + rng.next_double()) / 2.0;

        // 便捷函数：按角色画像在给定区间内生成一个属性值（内部消费一次 next_double）
        fn stat(
            attr: Attr,
            range: csc_domain::attr_range::AttrRange,
            profile: &crate::role_profile::RoleProfile,
            z: f64,
            rng: &mut Xoshiro256StarStar,
        ) -> i32 {
            RoleProfiles::stat_value(range, profile.pref(attr), z, rng.next_double())
        }

        let age = age.unwrap_or_else(|| rng.next_i32_in(18, 31)); // 随机年龄 18~30（双参数 nextInt(18,31)，stdlib 原语）

        let base = BaseAttributes {
            reaction: stat(Attr::Reaction, r.reaction, profile, z, rng),
            stability: stat(Attr::Stability, r.stability, profile, z, rng),
            endurance: stat(Attr::Endurance, r.endurance, profile, z, rng),
            stamina: stat(Attr::Stamina, r.stamina, profile, z, rng),
            health: stat(Attr::Health, r.health, profile, z, rng),
        };
        let skill = SkillAttributes {
            aim: stat(Attr::Aim, r.aim, profile, z, rng),
            leader: stat(Attr::Leader, r.leader, profile, z, rng),
            communication: stat(Attr::Communication, r.communication, profile, z, rng),
            clutch: stat(Attr::Clutch, r.clutch, profile, z, rng),
        };
        let pro = ProAttributes {
            mentality: stat(Attr::Mentality, r.mentality, profile, z, rng),
            confidence: stat(Attr::Confidence, r.confidence, profile, z, rng),
            team_spirit: stat(Attr::TeamSpirit, r.team_spirit, profile, z, rng),
            loyalty: stat(Attr::Loyalty, r.loyalty, profile, z, rng),
            morale: stat(Attr::Morale, r.morale, profile, z, rng),
        };
        let weapon = WeaponAttributes {
            position: chosen_role,
            ak: stat(Attr::Ak, r.ak, profile, z, rng),
            awp: stat(Attr::Awp, r.awp, profile, z, rng),
            pistol: stat(Attr::Pistol, r.pistol, profile, z, rng),
            smoke: stat(Attr::Smoke, r.smoke, profile, z, rng),
            utility: stat(Attr::Utility, r.utility, profile, z, rng),
        };

        GeneratedPlayer {
            player_name,
            age,
            role: chosen_role,
            base,
            skill,
            pro,
            weapon,
            potential: Self::potential_for(
                age,
                PowerCalculator::player_power_components(chosen_role, &base, &skill, &pro, &weapon)
                    .to_i32(),
                rng,
            ),
        }
    }

    /// 潜力上限（隐藏）：当前综合水平（角色加权实力）+ 年龄决定的上升空间。
    ///
    /// - ≤20 岁：15~40 上升空间（潜力可能远超当前，年轻小将）
    /// - 21-23：5~25
    /// - 24-26：0~15（巅峰前，潜力接近当前）
    /// - 27+：0（无成长空间，进入年龄曲线衰退段）
    ///
    /// clamp 到 0..100；潜力 ≥ 当前加权实力（成长只向上收敛，衰退由年龄曲线单独驱动）。
    pub fn potential_for(age: i32, current_level: i32, rng: &mut Xoshiro256StarStar) -> i32 {
        let upside = match age {
            // 双参数 nextInt(from, until)（stdlib 原语，与单参数 nextInt(until) 不同——见 csc-util 文档）
            a if a <= 20 => rng.next_i32_in(15, 41),
            a if a <= 23 => rng.next_i32_in(5, 26),
            a if a <= 26 => rng.next_i32_in(0, 16),
            _ => 0,
        };
        (current_level + upside).clamp(0, 100)
    }

    /// **生态校准版**潜力：在 [`Self::potential_for`] 基础上，按拟合出的「天才长尾比例」
    /// 把一小部分年轻新秀的潜力拉高到接近满值（常年 TOP20 级）——对应真实三年榜里
    /// 仅 19% 选手能连续登顶、56% 一闪而过的事实。
    ///
    /// **不另耗 RNG**：`tail_seed` 是调用方从"已消费过的随机量"派生的 `[0,1]` 均匀值
    /// （如新秀年龄抽值的归一化产物），用来判定是否命中长尾——不改变 RNG 序列位置，
    /// 从而不破坏 seed → 决策日志 = 一致世界的可复现性契约。
    ///
    /// @param tail_seed 已确定的 [0,1] 均匀量（判定是否命中天才长尾）
    /// @param elite_tail_ratio 拟合出的天才长尾比例（≈0.19）
    pub fn potential_for_calibrated(
        age: i32,
        current_level: i32,
        rng: &mut Xoshiro256StarStar,
        tail_seed: f64,
        elite_tail_ratio: f64,
    ) -> i32 {
        let base = Self::potential_for(age, current_level, rng);
        // 长尾：年轻（仍有成长空间）+ 命中稀缺比例 → 潜力拉满到接近 100
        if age <= 23 && tail_seed < elite_tail_ratio {
            return (base + 25).min(100);
        }
        base
    }

    /// 采样一名新选手的「当前实力」Rating（真实职业分布 × Box-Muller 标准
    /// 正态；采样实现 = `csc_util::gaussian`（Kotlin `RandomUtils.gaussian` 转写，
    /// u1 只拦下界 `max(1e-12)`；`next_double ∈ [0,1)` 永不为 1，上界 clamp 是多余防御）；
    /// clamp 到 profile 区间）。
    ///
    /// 采样算法放在生成器而非 [`RatingProfile`]（数据表）——distribution 只
    /// 提供参数，采样是"生成"职责（与 `generate_attributes` 同域）。
    pub fn sample_newcomer_rating(profile: &RatingProfile, rng: &mut Xoshiro256StarStar) -> f64 {
        let mean = profile.rookie_mean();
        let sd = profile.rookie_sd();
        let (lo, hi) = profile.rookie_clamp();
        (mean + sd * csc_util::gaussian(rng)).clamp(lo, hi)
    }

    /// 采样一名新选手的起始档位（真实分布 Rating → [`RatingBaseline::tier_for_rating`]）。
    /// `profile = None` 时回退旧逻辑（新秀固定 Tier4）。
    pub fn newcomer_tier(profile: Option<&RatingProfile>, rng: &mut Xoshiro256StarStar) -> Tier {
        profile.map_or(Tier::Tier4, |p| {
            RatingBaseline::tier_for_rating(Self::sample_newcomer_rating(p, rng))
        })
    }

    /// 生成一名指定档位、指定角色的 NPC（career = None）。
    pub fn generate_npc(
        tier: Tier,
        team: Option<TeamId>,
        role: Option<Role>,
        name: Option<&str>,
        rng: &mut Xoshiro256StarStar,
        age: Option<i32>,
    ) -> PlayerCharacter {
        let g = Self::generate_attributes(tier, role, name, rng, age);
        PlayerCharacter {
            id: PlayerId::NONE, // 由 World 登记时分配
            name: g.player_name,
            age: g.age,
            role: g.role,
            base: g.base,
            skill: g.skill,
            pro: g.pro,
            weapon: g.weapon,
            potential: g.potential,
            fatigue: 0.0,
            injury: None,
            career: None,
            team,
            retired: false,
        }
    }

    /// 生成一名**玩家**（主角）：属性与 NPC 同源，额外携带 [CareerInfo]
    /// （初始为自由身：team=None + 0 薪资 + 0 合同年，首个转会窗签约）。
    ///
    /// @param age 指定初始年龄（主角固定 16 岁起步）；None = 随机 18~30。
    /// @param role 指定角色定位（None = 随机）。
    pub fn generate_player(
        tier: Tier,
        name: Option<&str>,
        rng: &mut Xoshiro256StarStar,
        age: Option<i32>,
        role: Option<Role>,
    ) -> PlayerCharacter {
        let g = Self::generate_attributes(tier, role, name, rng, age);
        PlayerCharacter {
            id: PlayerId::NONE,
            name: g.player_name,
            age: g.age,
            role: g.role,
            base: g.base,
            skill: g.skill,
            pro: g.pro,
            weapon: g.weapon,
            potential: g.potential,
            fatigue: 0.0,
            injury: None,
            career: Some(CareerInfo::free_agent_default(2026)),
            team: None,
            retired: false,
        }
    }
}

// 辅助：f64 → i32 截断（= Kotlin Double.toInt()）
trait ToIntTrunc {
    fn to_i32(&self) -> i32;
}
impl ToIntTrunc for f64 {
    fn to_i32(&self) -> i32 {
        *self as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_pool_is_unique_and_40() {
        let mut seen = std::collections::HashSet::new();
        for n in NAME_POOL {
            assert!(seen.insert(n), "名字重复: {n}");
        }
        assert_eq!(NAME_POOL.len(), 40);
    }

    #[test]
    fn potential_for_age_bands() {
        let mut rng = Xoshiro256StarStar::seed(42);
        // ≤20：15~40 上升空间
        for _ in 0..50 {
            let p = RandomPlayerGenerator::potential_for(18, 50, &mut rng);
            assert!((65..=90).contains(&p), "18 岁潜力应在 65..=90: {p}");
        }
        // 27+：无上升空间
        assert_eq!(RandomPlayerGenerator::potential_for(27, 50, &mut rng), 50);
        assert_eq!(RandomPlayerGenerator::potential_for(30, 50, &mut rng), 50);
    }

    #[test]
    fn potential_clamped_to_100() {
        let mut rng = Xoshiro256StarStar::seed(7);
        for _ in 0..200 {
            let p = RandomPlayerGenerator::potential_for(18, 95, &mut rng);
            assert!(p <= 100);
        }
    }

    #[test]
    fn generate_attributes_respects_tier_range() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let g = RandomPlayerGenerator::generate_attributes(
            Tier::Tier2,
            Some(Role::Rifler),
            Some("Test"),
            &mut rng,
            Some(20),
        );
        assert_eq!(g.player_name, "Test");
        assert_eq!(g.role, Role::Rifler);
        assert_eq!(g.age, 20);
        // Tier2 区间：reaction 72..90、ak 72..90
        assert!((72..=90).contains(&g.base.reaction));
        assert!((72..=90).contains(&g.weapon.ak));
        // 潜力 ≥ 当前实力（成长只向上收敛）
        let power =
            PowerCalculator::player_power_components(g.role, &g.base, &g.skill, &g.pro, &g.weapon)
                .to_i32();
        let potential = g.potential;
        assert!(potential >= power, "潜力 {potential} 应 ≥ 当前实力 {power}");
    }

    #[test]
    fn same_seed_same_generation() {
        let mut a = Xoshiro256StarStar::seed(42);
        let mut b = Xoshiro256StarStar::seed(42);
        let ga = RandomPlayerGenerator::generate_attributes(Tier::Tier1, None, None, &mut a, None);
        let gb = RandomPlayerGenerator::generate_attributes(Tier::Tier1, None, None, &mut b, None);
        assert_eq!(ga, gb, "同种子生成必须完全一致");
    }

    #[test]
    fn generate_player_is_free_agent() {
        let mut rng = Xoshiro256StarStar::seed(1);
        let p = RandomPlayerGenerator::generate_player(
            Tier::Tier0,
            Some("ZywOo"),
            &mut rng,
            None,
            None,
        );
        assert_eq!(p.name, "ZywOo");
        assert!(p.is_player());
        assert_eq!(p.team, None, "初始自由身");
        let career = p.career.as_ref().unwrap();
        assert_eq!(career.salary, 0);
        assert_eq!(career.contract_years, 0);
    }

    /// 跨语言 golden：Kotlin `RandomPlayerGenerator.generateAttributes` 权威输出
    /// （seed=42、TIER2、RIFLER、"Golden"、age=20；tools/gen_entities_golden.kt）。
    #[test]
    fn golden_generate_attributes() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let g = RandomPlayerGenerator::generate_attributes(
            Tier::Tier2,
            Some(Role::Rifler),
            Some("Golden"),
            &mut rng,
            Some(20),
        );
        assert_eq!(g.player_name, "Golden");
        assert_eq!(g.role, Role::Rifler);
        assert_eq!(g.age, 20);
        assert_eq!(g.potential, 91);
        // GEN_BASE=83,86,82,79,81
        assert_eq!(g.base.reaction, 83);
        assert_eq!(g.base.stability, 86);
        assert_eq!(g.base.endurance, 82);
        assert_eq!(g.base.stamina, 79);
        assert_eq!(g.base.health, 81);
        // GEN_SKILL=87,60,59,76
        assert_eq!(g.skill.aim, 87);
        assert_eq!(g.skill.leader, 60);
        assert_eq!(g.skill.communication, 59);
        assert_eq!(g.skill.clutch, 76);
        // GEN_PRO=73,77,63,62,72
        assert_eq!(g.pro.mentality, 73);
        assert_eq!(g.pro.confidence, 77);
        assert_eq!(g.pro.team_spirit, 63);
        assert_eq!(g.pro.loyalty, 62);
        assert_eq!(g.pro.morale, 72);
        // GEN_WEAPON=RIFLER,86,53,75,53,56
        assert_eq!(g.weapon.position, Role::Rifler);
        assert_eq!(g.weapon.ak, 86);
        assert_eq!(g.weapon.awp, 53);
        assert_eq!(g.weapon.pistol, 75);
        assert_eq!(g.weapon.smoke, 53);
        assert_eq!(g.weapon.utility, 56);
    }

    /// 跨语言 golden：potentialFor 序列（Kotlin 权威：每轮先消耗一个 nextLong 再算）。
    #[test]
    fn golden_potential_for_sequence() {
        let mut rng = Xoshiro256StarStar::seed(42);
        let seq: Vec<i32> = (0..3)
            .map(|_| {
                rng.next_u64(); // 对齐 Kotlin golden 的 nextLong() 消耗
                RandomPlayerGenerator::potential_for(20, 60, &mut rng)
            })
            .collect();
        assert_eq!(seq, vec![76, 77, 95]);
    }

    /// 生态校准版潜力：命中长尾的年轻选手潜力被拉高，且不额外消耗 RNG 序列位置。
    #[test]
    fn calibrated_potential_elite_tail() {
        // 记录 baseline 的 RNG 轨迹，确认 calibrated 版不改变 RNG 序列推进
        let mut rng_a = Xoshiro256StarStar::seed(42);
        let mut rng_b = Xoshiro256StarStar::seed(42);
        let base = RandomPlayerGenerator::potential_for(20, 60, &mut rng_a);
        let hit = RandomPlayerGenerator::potential_for_calibrated(20, 60, &mut rng_b, 0.10, 0.19);
        assert_eq!(hit, (base + 25).min(100), "命中长尾 → 潜力拉高");
        assert_eq!(rng_a.snapshot(), rng_b.snapshot(), "不额外消费 RNG");

        // 未命中长尾（tail_seed > ratio）→ 与 baseline 一致
        let mut rng_c = Xoshiro256StarStar::seed(42);
        let miss = RandomPlayerGenerator::potential_for_calibrated(20, 60, &mut rng_c, 0.90, 0.19);
        assert_eq!(miss, base, "未命中长尾 → 与 baseline 一致");
    }

    const PROFILE: &str = r#"{
        "generated_at": "x",
        "source": "test",
        "sample_size": 672,
        "global": {"mean": 1.021, "sd": 0.103},
        "roles": {},
        "ages": [],
        "rookie": {"mean_offset": -0.02, "sd_scale": 1.0, "clamp": [0.5, 1.7]}
    }"#;

    #[test]
    fn newcomer_sampling_tracks_distribution() {
        let p = RatingProfile::from_json_str(PROFILE).unwrap();
        let mut rng = Xoshiro256StarStar::seed(42);
        let seq_a: Vec<f64> = (0..400)
            .map(|_| RandomPlayerGenerator::sample_newcomer_rating(&p, &mut rng))
            .collect();
        let mut rng2 = Xoshiro256StarStar::seed(42);
        let seq_b: Vec<f64> = (0..400)
            .map(|_| RandomPlayerGenerator::sample_newcomer_rating(&p, &mut rng2))
            .collect();
        assert_eq!(seq_a, seq_b, "同 seed 采样序列完全一致（确定性契约）");
        let mean = seq_a.iter().sum::<f64>() / seq_a.len() as f64;
        assert!(
            (1.001 - mean).abs() < 0.03,
            "400 次采样均值应接近期望 1.001：{mean}"
        );
        assert!(
            seq_a.iter().all(|r| (0.5..=1.7).contains(r)),
            "clamp 区间内"
        );
    }

    /// M5 收敛回归：新秀 Rating 采样路径与 `csc_util::gaussian` 口径一致——
    /// 同一 RNG 状态下先采 gaussian 再手算 clamp = 直接调 sample_newcomer_rating 的输出。
    #[test]
    fn newcomer_rating_uses_shared_gaussian() {
        let p = RatingProfile::from_json_str(PROFILE).unwrap();
        let (lo, hi) = p.rookie_clamp();
        let mut rng_direct = Xoshiro256StarStar::seed(42);
        let mut rng_manual = Xoshiro256StarStar::seed(42);
        let mut last_direct = 0.0f64;
        for _ in 0..64 {
            let direct = RandomPlayerGenerator::sample_newcomer_rating(&p, &mut rng_direct);
            let manual = (p.rookie_mean() + p.rookie_sd() * csc_util::gaussian(&mut rng_manual))
                .clamp(lo, hi);
            assert_eq!(direct, manual, "采样必须与 csc_util::gaussian 口径逐位一致");
            last_direct = direct;
        }
        // 输出仍在 [lo, hi] 且 RNG 序列长度不变（每轮恰 2 次 next_double）
        assert!(
            (lo..=hi).contains(&last_direct),
            "clamp 区间内: {last_direct}"
        );
    }

    #[test]
    fn newcomer_tiers_are_realistic_mix() {
        let p = RatingProfile::from_json_str(PROFILE).unwrap();
        let mut rng = Xoshiro256StarStar::seed(7);
        let mut counts = [0u32; 5];
        for _ in 0..400 {
            match RandomPlayerGenerator::newcomer_tier(Some(&p), &mut rng) {
                Tier::Tier0 => counts[0] += 1,
                Tier::Tier1 => counts[1] += 1,
                Tier::Tier2 => counts[2] += 1,
                Tier::Tier3 => counts[3] += 1,
                Tier::Tier4 => counts[4] += 1,
            }
        }
        // 真实分布：Tier4 ~24%、Tier3 ~31%、Tier2 ~28%、Tier1 ~15%、Tier0 ~1.6%
        assert!(counts[4] > 60, "Tier4 应占多数（青训主体）：{counts:?}");
        assert!(counts[0] < 30, "Tier0 应稀有（donk 级天才）：{counts:?}");
        assert!(
            counts[1] + counts[2] > 100,
            "一线/中游新秀应有相当比例：{counts:?}"
        );
        // None → 固定 Tier4（旧逻辑回退）
        let mut rng2 = Xoshiro256StarStar::seed(1);
        assert_eq!(
            RandomPlayerGenerator::newcomer_tier(None, &mut rng2),
            Tier::Tier4
        );
    }
}
