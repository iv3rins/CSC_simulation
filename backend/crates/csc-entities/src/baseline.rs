//! 真实选手位置基准表（Kotlin `RoleBaseline.kt` 的转写）。

use std::collections::HashMap;

use serde::Deserialize;

use crate::role::Role;

/// roles_baseline.json 的文件结构（字段名 camelCase，与 Kotlin 解析的 JSON 一致）。
#[derive(Debug, Deserialize)]
struct BaselineFile {
    players: Vec<BaselineEntry>,
}

#[derive(Debug, Deserialize)]
struct BaselineEntry {
    /// 选手名（映射键；`player` 字段唯一被消费的字段）
    player: String,
    role: String,
    /// 真实年龄（可选；缺省 = 引擎随机 18~30）
    #[serde(default)]
    age: Option<i32>,
    // 注：JSON 资产中另有 team/ctRole/tRole 等展示性字段——serde 默认忽略
    // 未声明字段，**不**在结构里保留占位（历史上有 #[allow(dead_code)] 死字段
    // 声称"防格式漂移"，但未加 deny_unknown_fields 时该主张不成立，已删除）。
}

/// 真实选手位置基准表（只读）。
///
/// 数据来源：`assets/roles_baseline.json`（HLTV 真实位置 + 年龄）。
/// 用途：映射时若 JSON 中存在该选手的 role/age，则用真实值；不存在则回退到 slot 分配。
///
/// 转写差异：Kotlin 手写逐行 JSON 解析 → Rust 用 `serde_json`（结构定死，防格式漂移）；
/// 保持**零 IO**——`from_json_str` 只做反序列化，文件读取由调用方（server/CLI）负责。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleBaseline {
    /// playerName → 主角色（HLTV 原始字符串，如 "AWPer" / "IGL-Opener"）
    roles: HashMap<String, String>,
    /// playerName → 真实年龄（缺省条目不在此表中）
    ages: HashMap<String, i32>,
}

impl RoleBaseline {
    /// 解析 JSON 文本（= Kotlin `parse`；加载失败返回 [`SimError::Asset`] 而非
    /// 静默空表——外部输入边界 D6）。
    pub fn from_json_str(json: &str) -> Result<Self, csc_util::SimError> {
        let file: BaselineFile = serde_json::from_str(json).map_err(|e| {
            csc_util::SimError::asset("roles_baseline.json", format!("JSON 解析失败：{e}"))
        })?;
        let roles = file
            .players
            .iter()
            .map(|e| (e.player.clone(), e.role.clone()))
            .collect();
        let ages = file
            .players
            .iter()
            .filter_map(|e| e.age.map(|a| (e.player.clone(), a)))
            .collect();
        Ok(Self { roles, ages })
    }

    /// 该选手是否有基准 role。
    pub fn has_role(&self, player_name: &str) -> bool {
        self.roles.contains_key(player_name)
    }

    /// 选手的 [`Role`] 枚举位置；无基准或映射失败返回 None（调用方 fallback 到 slot）。
    pub fn role_of(&self, player_name: &str) -> Option<Role> {
        self.roles.get(player_name).map(|r| Self::hltv_to_role(r))
    }

    /// 选手真实年龄（None = 无基准，调用方按引擎默认随机）。
    pub fn age_of(&self, player_name: &str) -> Option<i32> {
        self.ages.get(player_name).copied()
    }

    /// 覆盖的选手总数。
    pub fn size(&self) -> usize {
        self.roles.len()
    }

    /// HLTV 主角色 → 现有 [`Role`] 枚举。
    /// IGL 的三种细分（Opener/AWPer/Closer）都归为 IGL；未知角色兜底为 RIFLER。
    pub fn hltv_to_role(hltv_role: &str) -> Role {
        match hltv_role {
            "AWPer" => Role::Awp,                                   // 狙击手
            "Opener" => Role::Entry,                                // 突破手
            "Closer" => Role::Lurker,                               // 残局/自由人
            "IGL-Opener" | "IGL-AWPer" | "IGL-Closer" => Role::Igl, // 指挥（细分归 IGL）
            "Support" => Role::Support,                             // 辅助
            _ => Role::Rifler,                                      // 未知兜底
        }
    }
}

/// 真实选手评级先验表（`player_ratings.json`）——**用真实 HLTV Rating 校准虚拟
/// 选手的实力档位**，修复「真实选手名只是皮肤，属性纯随机」的失真
/// （试玩发现 zont1x 打出 1.43 评分登上 TOP1——现实里他约 1.05~1.10）。
///
/// 数据为 2024-2025 大赛评级的**近似值**（手编 v1；后续可由爬取的
/// HLTV 数据集替换——见 player_ratings.json 注释与爬取路线图）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RatingBaseline {
    /// playerName → 真实 Rating 先验（生涯/大赛近似）
    ratings: std::collections::HashMap<String, f64>,
}

impl RatingBaseline {
    /// 解析 JSON 文本（结构 `{ "players": [{ "player", "rating" }] }`）。
    /// 加载失败返回 `Err`（外部输入边界 D6——坏资产显式报错，不静默空表）。
    pub fn from_json_str(json: &str) -> Result<Self, csc_util::SimError> {
        #[derive(serde::Deserialize)]
        struct File {
            players: Vec<Entry>,
        }
        #[derive(serde::Deserialize)]
        struct Entry {
            player: String,
            rating: f64,
        }
        let file: File = serde_json::from_str(json).map_err(|e| {
            csc_util::SimError::asset("player_ratings.json", format!("JSON 解析失败：{e}"))
        })?;
        let ratings = file
            .players
            .into_iter()
            .map(|e| (e.player, e.rating))
            .collect();
        Ok(Self { ratings })
    }

    /// 选手的真实 Rating 先验（None = 无数据，回退槽位档位逻辑）。
    pub fn rating_of(&self, player_name: &str) -> Option<f64> {
        self.ratings.get(player_name).copied()
    }

    /// 覆盖的选手总数。
    pub fn size(&self) -> usize {
        self.ratings.len()
    }

    /// 真实 Rating → 虚拟实力档位（校准曲线：T1+ 选手 1.00~1.35 映射到
    /// Tier0~Tier4；与 `TierRanges` 属性区间对齐）：
    /// - ≥1.22 → Tier0（世界顶级：donk/ZywOo/m0NESY）
    /// - ≥1.12 → Tier1（一线核心）
    /// - ≥1.04 → Tier2（中游主力）
    /// - ≥0.96 → Tier3（下游）
    /// - <0.96 → Tier4（边缘/青训）
    pub fn tier_for_rating(rating: f64) -> csc_domain::tier::Tier {
        if rating >= 1.22 {
            csc_domain::tier::Tier::Tier0
        } else if rating >= 1.12 {
            csc_domain::tier::Tier::Tier1
        } else if rating >= 1.04 {
            csc_domain::tier::Tier::Tier2
        } else if rating >= 0.96 {
            csc_domain::tier::Tier::Tier3
        } else {
            csc_domain::tier::Tier::Tier4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;

    const SAMPLE: &str = r#"{
        "players": [
            { "player": "ZywOo", "team": "Vitality", "role": "AWPer", "ctRole": "AWPer", "tRole": "AWPer" },
            { "player": "apEX", "team": "Vitality", "role": "IGL-Opener", "ctRole": "IGL", "tRole": "Opener" },
            { "player": "UnknownGuy", "team": "Nova", "role": "StrangeRole", "ctRole": "X", "tRole": "Y" }
        ]
    }"#;

    #[test]
    fn parses_sample_json() {
        let b = RoleBaseline::from_json_str(SAMPLE).unwrap();
        assert_eq!(b.size(), 3);
        assert!(b.has_role("ZywOo"));
        assert!(!b.has_role("Niko"));
    }

    #[test]
    fn hltv_role_mapping() {
        assert_eq!(RoleBaseline::hltv_to_role("AWPer"), Role::Awp);
        assert_eq!(RoleBaseline::hltv_to_role("Opener"), Role::Entry);
        assert_eq!(RoleBaseline::hltv_to_role("Closer"), Role::Lurker);
        assert_eq!(RoleBaseline::hltv_to_role("IGL-Opener"), Role::Igl);
        assert_eq!(RoleBaseline::hltv_to_role("IGL-AWPer"), Role::Igl);
        assert_eq!(RoleBaseline::hltv_to_role("IGL-Closer"), Role::Igl);
        assert_eq!(RoleBaseline::hltv_to_role("Support"), Role::Support);
        assert_eq!(RoleBaseline::hltv_to_role("AnythingElse"), Role::Rifler);
    }

    #[test]
    fn role_of_falls_back_to_none() {
        let b = RoleBaseline::from_json_str(SAMPLE).unwrap();
        assert_eq!(b.role_of("ZywOo"), Some(Role::Awp));
        assert_eq!(b.role_of("apEX"), Some(Role::Igl));
        assert_eq!(
            b.role_of("UnknownGuy"),
            Some(Role::Rifler),
            "未知角色兜底 RIFLER"
        );
        assert_eq!(b.role_of("Niko"), None);
    }

    #[test]
    fn invalid_json_returns_err() {
        assert!(RoleBaseline::from_json_str("not json").is_err());
    }

    #[test]
    fn rating_baseline_parses_and_maps_tier() {
        let r = RatingBaseline::from_json_str(
            r#"{"generated_at":"x","source":"test","players":[
                {"player":"donk","rating":1.30},
                {"player":"zont1x","rating":1.08},
                {"player":"karrigan","rating":0.95}
            ]}"#,
        )
        .unwrap();
        assert_eq!(r.size(), 3);
        assert_eq!(r.rating_of("donk"), Some(1.30));
        assert_eq!(r.rating_of("nobody"), None);
        assert_eq!(
            RatingBaseline::tier_for_rating(1.30),
            Tier::Tier0,
            "1.22+ 顶级"
        );
        assert_eq!(RatingBaseline::tier_for_rating(1.15), Tier::Tier1);
        assert_eq!(RatingBaseline::tier_for_rating(1.08), Tier::Tier2);
        assert_eq!(RatingBaseline::tier_for_rating(1.00), Tier::Tier3);
        assert_eq!(RatingBaseline::tier_for_rating(0.95), Tier::Tier4);
        assert!(RatingBaseline::from_json_str("{ bad").is_err());
    }
}

// ─────────────────────────── 新选手实力分布模型 ───────────────────────────

/// 一个 Rating 分布（均值 ± 标准差；正态近似）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatingDist {
    pub mean: f64,
    pub sd: f64,
}

/// 一个年龄段分布（含 `min..=max` 与样本量，供检验）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgeBand {
    pub min: i32,
    pub max: i32,
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
}

/// **新选手实力分布模型**（`rating_profile.json`）——用真实职业分布校准
/// 虚拟新秀/新选手的档位采样（2026 修订：新秀不再固定 Tier4 起步，而是按
/// 全职业 Rating 分布采样——多数青训进 Tier3/4，少量一线天才，~1.6%
/// 直接 Tier0（donk 级））。
///
/// 数据来源：`scripts/calibrate_profile.mjs` 从 csapi.de 全职业聚合拟合
/// （`assets/rating_profile.json`）。`roles`/`ages` 为按角色/年龄段的真实
/// 分布（供检验与未来自由市场生成扩展），采样模型只消费 `global + rookie`
/// 参数（新秀角色未定，年龄偏差被精英采样偏差污染，故不直接参与采样）。
///
/// **零 IO**：`from_json_str` 只做反序列化，文件读取由调用方（server/CLI）负责。
/// **确定性**：采样只消费 2 次 `next_double`（Box-Muller）——同一 seed 同一序列。
#[derive(Debug, Clone, PartialEq)]
pub struct RatingProfile {
    /// 全职业 Rating 分布（采样总体）
    global: RatingDist,
    /// 按角色分布（缺省回退 global；供检验/未来扩展）
    roles: HashMap<Role, RatingDist>,
    /// 按年龄段分布（供检验；采样不直接使用）
    ages: Vec<AgeBand>,
    /// 新秀采样：均值偏移（青训起步略低于职业均值，潜力未兑现）
    rookie_mean_offset: f64,
    /// 新秀采样：标准差缩放
    rookie_sd_scale: f64,
    /// 新秀采样：clamp 区间（防极端值溢出档位边界）
    rookie_clamp: (f64, f64),
}

/// rating_profile.json 的文件结构（只声明被消费的字段；generated_at/source/
/// sample_size 等元数据字段由 serde 默认忽略——历史 #[allow(dead_code)] 占位
/// 声称"防格式漂移"，但未加 deny_unknown_fields 时该主张不成立，已删除）。
#[derive(Debug, serde::Deserialize)]
struct ProfileFile {
    global: DistFile,
    roles: HashMap<String, DistFile>,
    ages: Vec<AgeBandFile>,
    rookie: RookieFile,
}

#[derive(Debug, serde::Deserialize)]
struct DistFile {
    mean: f64,
    sd: f64,
}

#[derive(Debug, serde::Deserialize)]
struct AgeBandFile {
    min: i32,
    max: i32,
    n: usize,
    mean: f64,
    sd: f64,
}

#[derive(Debug, serde::Deserialize)]
struct RookieFile {
    mean_offset: f64,
    sd_scale: f64,
    clamp: [f64; 2],
}

impl RatingProfile {
    /// 解析 JSON 文本（坏资产返回 `Err(SimError::Asset)`——外部输入边界 D6）。
    pub fn from_json_str(json: &str) -> Result<Self, csc_util::SimError> {
        let file: ProfileFile = serde_json::from_str(json).map_err(|e| {
            csc_util::SimError::asset("rating_profile.json", format!("JSON 解析失败：{e}"))
        })?;
        // 角色 key 为 SCREAMING_SNAKE_CASE（与 Role 的 serde 名一致）；未知 key 忽略。
        // 注意：key 是裸字符串（非 JSON 文档），用 from_value 而不是 from_str。
        let roles = file
            .roles
            .into_iter()
            .filter_map(|(k, d)| {
                serde_json::from_value::<Role>(serde_json::Value::String(k))
                    .ok()
                    .map(|r| {
                        (
                            r,
                            RatingDist {
                                mean: d.mean,
                                sd: d.sd,
                            },
                        )
                    })
            })
            .collect();
        let ages = file
            .ages
            .into_iter()
            .map(|a| AgeBand {
                min: a.min,
                max: a.max,
                n: a.n,
                mean: a.mean,
                sd: a.sd,
            })
            .collect();
        Ok(Self {
            global: RatingDist {
                mean: file.global.mean,
                sd: file.global.sd,
            },
            roles,
            ages,
            rookie_mean_offset: file.rookie.mean_offset,
            rookie_sd_scale: file.rookie.sd_scale,
            rookie_clamp: (file.rookie.clamp[0], file.rookie.clamp[1]),
        })
    }

    /// 全职业分布（采样总体；供检验）。
    pub fn global(&self) -> RatingDist {
        self.global
    }

    /// 按角色分布（缺省回退全职业；供检验/未来自由市场生成）。
    pub fn role_dist(&self, role: Role) -> RatingDist {
        self.roles.get(&role).copied().unwrap_or(self.global)
    }

    /// 按年龄段分布（None = 年龄超出全部区间；供检验）。
    pub fn age_band(&self, age: i32) -> Option<AgeBand> {
        self.ages
            .iter()
            .copied()
            .find(|b| age >= b.min && age <= b.max)
    }

    /// 新秀采样的均值（全职业均值 + 青训折扣）；采样本身由
    /// [`crate::generator::RandomPlayerGenerator`] 负责——本表只提供分布参数。
    pub fn rookie_mean(&self) -> f64 {
        self.global.mean + self.rookie_mean_offset
    }

    /// 新秀采样的标准差（全职业标准差 × 缩放）。
    pub fn rookie_sd(&self) -> f64 {
        self.global.sd * self.rookie_sd_scale
    }

    /// 新秀采样的 clamp 区间（防极端值溢出档位边界）。
    pub fn rookie_clamp(&self) -> (f64, f64) {
        self.rookie_clamp
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    const PROFILE: &str = r#"{
        "generated_at": "x",
        "source": "test",
        "sample_size": 672,
        "global": {"mean": 1.021, "sd": 0.103},
        "roles": {
            "IGL": {"mean": 0.964, "sd": 0.079},
            "AWP": {"mean": 1.077, "sd": 0.071},
            "RIFLER": {"mean": 1.021, "sd": 0.103},
            "ENTRY": {"mean": 1.073, "sd": 0.122},
            "SUPPORT": {"mean": 1.033, "sd": 0.072},
            "LURKER": {"mean": 1.036, "sd": 0.102}
        },
        "ages": [
            {"min": 17, "max": 19, "n": 6, "mean": 1.083, "sd": 0.195},
            {"min": 31, "max": 60, "n": 3, "mean": 0.910, "sd": 0.091}
        ],
        "rookie": {"mean_offset": -0.02, "sd_scale": 1.0, "clamp": [0.5, 1.7]}
    }"#;

    #[test]
    fn parses_profile_and_distributions() {
        let p = RatingProfile::from_json_str(PROFILE).unwrap();
        assert_eq!(
            p.global(),
            RatingDist {
                mean: 1.021,
                sd: 0.103
            }
        );
        // 角色分布：AWP > IGL（真实认知：狙击手 > 指挥）
        assert!(p.role_dist(Role::Awp).mean > p.role_dist(Role::Igl).mean);
        // 未知角色回退 global
        assert_eq!(p.role_dist(Role::Rifler).mean, 1.021);
        // 年龄段：老将均值低于年轻（31+ 0.910 < 17-19 1.083）
        let young = p.age_band(18).unwrap();
        let old = p.age_band(35).unwrap();
        assert!(young.mean > old.mean);
        assert_eq!(p.age_band(99), None);
    }

    #[test]
    fn invalid_json_returns_err() {
        assert!(RatingProfile::from_json_str("not json").is_err());
        assert!(
            RatingProfile::from_json_str(r#"{"global":{"mean":1}}"#).is_err(),
            "缺字段应报错（防格式漂移）"
        );
    }
}
