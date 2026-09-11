//! 选手属性维度 + 角色画像（Kotlin `RoleProfile.kt` 的转写）。

use serde::{Deserialize, Serialize};

use csc_domain::attr_range::AttrRange;

use crate::role::Role;

/// 选手属性维度（19 项，与属性面板一一对应；索引 = [`Self::idx`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Attr {
    // —— 基础属性 ——
    /// 反应时间
    Reaction,
    /// 稳定性
    Stability,
    /// 体能
    Endurance,
    /// 精力
    Stamina,
    /// 健康
    Health,
    // —— 技能属性 ——
    /// 瞄准能力
    Aim,
    /// 领导力
    Leader,
    /// 沟通
    Communication,
    /// 残局能力
    Clutch,
    // —— 心理属性 ——
    /// 抗压能力
    Mentality,
    /// 自信
    Confidence,
    /// 团队配合
    TeamSpirit,
    /// 忠诚度
    Loyalty,
    /// 士气
    Morale,
    // —— 武器熟练度 ——
    /// AK-47 熟练度
    Ak,
    /// AWP 熟练度
    Awp,
    /// 手枪熟练度
    Pistol,
    /// 道具熟练度
    Smoke,
    /// 战术 / 地图理解
    Utility,
}

impl Attr {
    /// 属性在画像数组中的索引（0..19，按枚举声明顺序）。
    pub const fn idx(self) -> usize {
        match self {
            Self::Reaction => 0,
            Self::Stability => 1,
            Self::Endurance => 2,
            Self::Stamina => 3,
            Self::Health => 4,
            Self::Aim => 5,
            Self::Leader => 6,
            Self::Communication => 7,
            Self::Clutch => 8,
            Self::Mentality => 9,
            Self::Confidence => 10,
            Self::TeamSpirit => 11,
            Self::Loyalty => 12,
            Self::Morale => 13,
            Self::Ak => 14,
            Self::Awp => 15,
            Self::Pistol => 16,
            Self::Smoke => 17,
            Self::Utility => 18,
        }
    }
}

/// 角色画像：决定同一档位下该角色选手的属性轮廓与发挥波动。
///
/// 转写差异：Kotlin `prefs: Map<Attr, Double>`（缺省 0.5）→ **`[f64; 19]` 数组**
/// （Attr 索引直映，零散列开销；Kotlin 表全部 19 项均显式定义，语义一致）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RoleProfile {
    /// 各属性偏好权重 0..1（1=拉满，0=短板，0.5=平庸）
    pub prefs: [f64; 19],
    /// 单场发挥波动 σ（0~0.3；突破/自由人波动大、指挥稳），用于胜率计算时的个体发挥采样
    pub volatility: f64,
}

impl RoleProfile {
    /// 取某属性的偏好权重（数组直映；Kotlin 缺省 0.5 语义由表数据全填保证）。
    #[inline]
    pub fn pref(&self, attr: Attr) -> f64 {
        self.prefs[attr.idx()]
    }
}

/// 各角色画像表（Kotlin `RoleProfiles` object 的静态形态）。
///
/// 数学思想：每个角色是一个「偏好向量」pref∈[0,1]^19。
/// 生成属性时先采样风格纯度潜变量 z，再按
///     p_i = clamp( pref_i · (0.60 + 0.50·z) + u_i · 0.30 , 0, 1 )
/// 得到属性在档位区间内的分位（u_i 为个体噪声）。
/// 由于同一 z 同时调制所有属性，高偏好属性集体走高、低偏好属性集体走低，
/// 形成结构性耦合（AWP 高则 AK 必然低），而非独立均匀随机。
pub struct RoleProfiles;

impl RoleProfiles {
    /// 取某角色的画像。
    pub fn of(role: Role) -> &'static RoleProfile {
        match role {
            // IGL（指挥）：战术/沟通/道具优先，枪法显著偏软，最稳
            Role::Igl => &RoleProfile {
                prefs: [
                    0.45, 0.70, 0.60, 0.60,
                    0.60, // reaction, stability, endurance, stamina, health
                    0.18, 1.00, 0.95, 0.55, // aim, leader, communication, clutch
                    0.85, 0.65, 0.80, 0.65,
                    0.70, // mentality, confidence, team_spirit, loyalty, morale
                    0.18, 0.15, 0.50, 0.80, 0.85, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.05,
            },
            // AWP（狙击手）：AWP/手枪/反应拉满，步枪与道具是短板，稳定
            Role::Awp => &RoleProfile {
                prefs: [
                    0.90, 0.70, 0.55, 0.55,
                    0.65, // reaction, stability, endurance, stamina, health
                    0.85, 0.35, 0.45, 0.80, // aim, leader, communication, clutch
                    0.75, 0.75, 0.50, 0.50,
                    0.60, // mentality, confidence, team_spirit, loyalty, morale
                    0.25, 1.00, 0.92, 0.20, 0.35, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.08,
            },
            // RIFLER（步枪手）：AK/瞄准/反应拉满，狙击与道具偏弱，稳定
            Role::Rifler => &RoleProfile {
                prefs: [
                    0.85, 0.85, 0.65, 0.65,
                    0.70, // reaction, stability, endurance, stamina, health
                    0.95, 0.40, 0.45, 0.65, // aim, leader, communication, clutch
                    0.70, 0.70, 0.55, 0.50,
                    0.60, // mentality, confidence, team_spirit, loyalty, morale
                    1.00, 0.20, 0.75, 0.30, 0.40, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.08,
            },
            // ENTRY（突破手）：爆发力/反应/枪法极高，稳定性与心态差，波动巨大（神经刀）
            Role::Entry => &RoleProfile {
                prefs: [
                    0.90, 0.10, 0.55, 0.50,
                    0.65, // reaction, stability, endurance, stamina, health
                    0.95, 0.30, 0.40, 0.85, // aim, leader, communication, clutch
                    0.25, 0.90, 0.40, 0.30,
                    0.30, // mentality, confidence, team_spirit, loyalty, morale
                    0.70, 0.60, 0.75, 0.40, 0.50, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.28,
            },
            // SUPPORT（辅助）：道具/残局/心态极强，团队沟通偏弱，中等波动
            Role::Support => &RoleProfile {
                prefs: [
                    0.70, 0.65, 0.50, 0.50,
                    0.60, // reaction, stability, endurance, stamina, health
                    0.70, 0.30, 0.30, 1.00, // aim, leader, communication, clutch
                    0.90, 0.85, 0.45, 0.50,
                    0.60, // mentality, confidence, team_spirit, loyalty, morale
                    0.60, 0.45, 0.70, 0.55, 0.75, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.12,
            },
            // LURKER（自由人）：残局/单打/应变极强，枪法与道具突出，
            // 独行风格（指挥与团队精神低），波动偏大（神鬼莫测）
            Role::Lurker => &RoleProfile {
                prefs: [
                    0.80, 0.60, 0.55, 0.55,
                    0.65, // reaction, stability, endurance, stamina, health
                    0.85, 0.15, 0.30, 1.00, // aim, leader, communication, clutch
                    0.80, 0.90, 0.35, 0.30,
                    0.55, // mentality, confidence, team_spirit, loyalty, morale
                    0.75, 0.40, 0.70, 0.65, 0.80, // ak, awp, pistol, smoke, utility
                ],
                volatility: 0.16,
            },
        }
    }

    /// 按角色在档位区间内生成一个属性值（= Kotlin `RoleProfiles.statValue`）。
    ///
    /// 数学：先采样风格纯度 z∈[0,1]，再对每个属性按
    ///     p = clamp( pref · (0.60 + 0.50·z) + u · 0.30 , 0, 1 )
    /// 求分位 p，映射回档位区间。
    ///
    /// 转写差异：Kotlin `p.pow(2.0)`（libm pow）→ **`p * p`**（平方数学等价，
    /// 消除跨语言 libm 位级差异——可复现性口径 D2 的整数/确定性数学策略）。
    pub fn stat_value(range: AttrRange, pref: f64, z: f64, u: f64) -> i32 {
        // 分位 = 偏好 × 风格纯度调制 + 个体噪声；clamp 到 [0,1]
        let p = (pref * (0.60 + 0.50 * z) + u * 0.30).clamp(0.0, 1.0);
        // 非线性拉伸：中间值被压低，突出两极（避免六边形）
        let stretched = p * p;
        // 把分位映射回档位区间并 clamp 到 0..100（toInt 截断语义 = Rust as i32）
        let v = range.lo as f64 + (range.hi - range.lo) as f64 * stretched;
        (v as i32).clamp(0, 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_value_extremes() {
        let r = AttrRange::new(0, 100);
        // 满分画像：pref=1, z=1, u=1 → p=1 → 100
        assert_eq!(RoleProfiles::stat_value(r, 1.0, 1.0, 1.0), 100);
        // 无偏好：pref=0, z=0, u=0 → p=0 → 0
        assert_eq!(RoleProfiles::stat_value(r, 0.0, 0.0, 0.0), 0);
        // 平庸：pref=0.5, z=0.5, u=0.5 → p=0.5*0.85+0.15=0.575 → 0.575²≈0.33 → 33
        assert_eq!(RoleProfiles::stat_value(r, 0.5, 0.5, 0.5), 33);
    }

    #[test]
    fn stat_value_maps_into_range() {
        let r = AttrRange::new(80, 98);
        // 全满 → 上界 98
        assert_eq!(RoleProfiles::stat_value(r, 1.0, 1.0, 1.0), 98);
        // 全零 → 下界 80
        assert_eq!(RoleProfiles::stat_value(r, 0.0, 0.0, 0.0), 80);
    }

    #[test]
    fn awp_high_pref_ak_low_pref_coupled() {
        // 结构性耦合：同一 z 下 AWP 偏好高 → 生成值高、AK 偏好低 → 生成值低
        let r = AttrRange::new(55, 95); // awp 区间
        let rk = AttrRange::new(55, 88); // ak 区间（Tier0 表）
        let z = 0.8;
        let u = 0.5;
        let profile = RoleProfiles::of(Role::Awp);
        let awp_v = RoleProfiles::stat_value(r, profile.pref(Attr::Awp), z, u);
        let ak_v = RoleProfiles::stat_value(rk, profile.pref(Attr::Ak), z, u);
        assert!(
            awp_v > ak_v,
            "AWP 生成值应显著高于 AK（awp={awp_v}, ak={ak_v}）"
        );
    }

    #[test]
    fn volatility_ranges_match_kotlin() {
        assert_eq!(RoleProfiles::of(Role::Igl).volatility, 0.05);
        assert_eq!(RoleProfiles::of(Role::Entry).volatility, 0.28);
        assert_eq!(RoleProfiles::of(Role::Support).volatility, 0.12);
        assert_eq!(RoleProfiles::of(Role::Awp).volatility, 0.08);
    }

    /// 六角色画像互不相同：防止画像重复（2026-08-18 修复 SUPPORT=LURKER 逐字节相同）。
    #[test]
    fn all_six_role_profiles_are_distinct() {
        let roles = [
            Role::Igl,
            Role::Awp,
            Role::Rifler,
            Role::Entry,
            Role::Support,
            Role::Lurker,
        ];
        for (i, a) in roles.iter().enumerate() {
            for b in roles.iter().skip(i + 1) {
                let pa = RoleProfiles::of(*a);
                let pb = RoleProfiles::of(*b);
                assert_ne!(
                    (pa.prefs, pa.volatility),
                    (pb.prefs, pb.volatility),
                    "角色画像重复：{:?} == {:?}",
                    a,
                    b
                );
            }
        }
    }

    /// 跨语言 golden：Kotlin `RoleProfiles` 权威输出（tools/gen_entities_golden.kt）。
    #[test]
    fn golden_stat_value_and_prefs() {
        assert_eq!(
            RoleProfiles::stat_value(AttrRange::new(80, 98), 0.85, 0.5, 0.3),
            91
        );
        assert_eq!(
            RoleProfiles::stat_value(AttrRange::new(0, 100), 0.5, 0.5, 0.5),
            33
        );
        let awp = RoleProfiles::of(Role::Awp);
        assert_eq!(awp.pref(Attr::Awp), 1.0);
        assert_eq!(awp.pref(Attr::Ak), 0.25);
        assert_eq!(awp.volatility, 0.08);
    }
}
