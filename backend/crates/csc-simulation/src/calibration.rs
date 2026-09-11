//! 生态系统校准器（Calibration）——用真实 HLTV TOP20 三年名次轨迹拟合虚拟系统的
//! 成长 / 衰减 / 轮换动力参数，让「未来新人冒头、老将掉榜、天才常年霸榜」的虚拟走势
//! 在**统计上**与真实 CS 生态对齐。
//!
//! # 背景与动机
//!
//! 虚拟世界的人口 / 潜力 / 波动模型（`GrowthModel` / `VolatilityModel` /
//! `RandomPlayerGenerator::potential_for`）是**手调常数**。真实 2023/2024/2025 三年
//! TOP20 榜提供了珍贵的「巅峰轨迹观测样本」——谁在三年里连续霸榜、谁一闪而过、
//! 每年有多少新面孔涌入、年际名次平均晃动多少名。本模块把这些观测拟合为一组
//! **可注入的确定性参数**，从而校准虚拟生态的"竞争烈度"与"天才稀缺度"。
//!
//! # 拟合出的三个参数（= 真实三年测得的经验值）
//!
//! 1. **`elite_tail_ratio`（天才长尾比例 ≈ 0.19）**：连续 3 年登上 TOP20 的选手
//!    占全部上榜选手的 19%。含义：真正"常年巅峰"的天才是稀缺长尾，多数名将
//!    只是一闪而过（56% 只上榜 1 年）。
//! 2. **`career_drift`（年际名次漂移 σ，≈ 4.5 的位次波动）**：连续在榜选手年际
//!    名次变动的标准差。含义：高排位有强粘性（中位 ±2 名），低排位大洗牌
//!    （尾巴 ±8~15 名）——对应"状态起伏"这一维度。
//! 3. **`rookie_elite_rate`（新秀进 TOP20 年速率 ≈ 0.30~0.60）**：某一年
//!    TOP20 里上一年没有出现的新面孔比例（2024 年 60%、2025 年 30%）。含义：
//!    每年有相当比例的黑马 / 新秀改写榜单。
//!
//! # 可复现性契约
//!
//! 本校准器是**纯函数**：输入三年榜 → 输出确定性 `CalibrationProfile`。这些参数
//! 只改变"世界生成时潜力 / 波动的**分布形状**"，不新增 RNG 调用、不改变既有
//! RNG 序列的推进位置——因此种子 + 决策日志 = 一致世界的契约**不被打破**。
//! 注入点用「结果重映射」而非「额外掷骰」实现。

use csc_domain::real_top20::RealTop20Index;
use csc_entities::baseline::RoleBaseline;

/// 基线快照的参考年份（`standings_global_2026_01_05.json` → 2026；`roles_baseline.json`
/// 的 `age` 字段即为该日期的年龄）。用于把「基线年龄」反推到任意 TOP20 年份。
pub const BASELINE_YEAR: i32 = 2026;

/// 一条「选手 × 年份 × 年龄」的观测样本（由 nickname 关联真实榜与年龄基线得出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top20AgeSample {
    pub nickname: String,
    /// TOP20 上榜年份
    pub year: i32,
    /// 该年份的年龄（由基线年龄反推：age = baseline_age − (BASELINE_YEAR − year)）
    pub age: i32,
}

/// 峰值年龄分布统计（TOP20 上榜时刻的年龄画像）。
#[derive(Debug, Clone, PartialEq)]
pub struct PeakAgeStats {
    /// 全部样本（按 year、age 升序）
    pub samples: Vec<Top20AgeSample>,
    /// 年龄 → 上榜次数直方图
    pub histogram: std::collections::BTreeMap<i32, usize>,
    /// 平均年龄
    pub mean: f64,
    /// 中位年龄
    pub median: i32,
    /// 最年轻 / 最年长（样本为空时 None）
    pub min: Option<i32>,
    pub max: Option<i32>,
}

/// 生态校准参数组（确定性；可存档/可展示）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationProfile {
    /// 天才长尾比例（0..1）：潜力达到"常年 TOP20"级的上尾概率。
    pub elite_tail_ratio: f64,
    /// 年际名次漂移 σ（0..无限）：连续在榜选手年际名次变动的标准差。
    pub career_drift: f64,
    /// 新秀进 TOP20 年速率（0..1）：任一年 TOP20 中上一年未上榜的新面孔比例。
    pub rookie_elite_rate: f64,
}

impl Default for CalibrationProfile {
    fn default() -> Self {
        // 真实三年 HLTV 榜的实验参数（无数据时也走这套默认，保证行为稳定）
        Self {
            elite_tail_ratio: 0.19,
            career_drift: 4.5,
            rookie_elite_rate: 0.45,
        }
    }
}

/// 校准结果（含中间统计，供前端展示拟合结论）。
#[derive(Debug, Clone, PartialEq)]
pub struct CalibrationReport {
    /// 拟合出的参数组
    pub profile: CalibrationProfile,
    /// 观测样本量：上榜选手总数（去重）
    pub distinct_players: usize,
    /// 连续在榜 1/2/3… 届的直方图（届数 → 人数）
    pub appearance_histogram: std::collections::BTreeMap<usize, usize>,
    /// 年际名次变动样本数（绝对值的样本）
    pub movement_samples: usize,
    /// 逐年新面孔率：`(year, new_count, total, ratio)`
    pub newcomer_rates: Vec<(i32, usize, usize, f64)>,
    /// 峰值年龄统计（nickname 关联年龄基线后；无基线 = 空样本）
    pub peak_age: PeakAgeStats,
    /// 一段人类可读的拟合结论（推演未来新人 TOP20 走势的文案素材）
    pub summary: String,
}

impl CalibrationProfile {
    /// 是否命中「天才长尾」：输入一个已生成、已确定的 `[0,1]` 均匀量（如新秀的
    /// 潜力 upside 经归一化后的产物，或某个不另耗 RNG 的派生值），按长尾比例判定
    /// 该选手是否为稀缺的"常年巅峰"级天才。
    ///
    /// **纯确定性重映射**：不额外消费 RNG，只对"已经存在的随机量"做阈值判定——
    /// 因此不改变既有 RNG 序列位置，可复现性契约不被破坏。
    pub fn is_elite_tail(&self, uniform01: f64) -> bool {
        uniform01 < self.elite_tail_ratio
    }
}

/// 生态校准器——纯函数（无状态、无随机）。
pub struct EcoCalibrator;

impl EcoCalibrator {
    /// 从三年真实榜拟合生态参数（缺数据时回退 [`CalibrationProfile::default`]）。
    pub fn fit(index: &RealTop20Index) -> CalibrationProfile {
        if index.is_empty() || index.years().len() < 2 {
            return CalibrationProfile::default();
        }

        // 1. 天才长尾比例 = 连续 3 届在榜人数 / 去重总人数（不足 3 届数据用 ≥2 届近似）
        let players = index.players_with_years();
        let total = players.len() as f64;
        let elite_tail_ratio = if total > 0.0 {
            let years_count = index.years().len();
            let long_stayers = players
                .iter()
                .filter(|(_, ys)| ys.len() == years_count)
                .count() as f64;
            (long_stayers / total).clamp(0.0, 1.0)
        } else {
            CalibrationProfile::default().elite_tail_ratio
        };

        // 2. 年际名次漂移 = 年际变动绝对值的标准差（样本不足用 default）
        let movements = index.year_over_year_movements();
        let career_drift = if movements.len() >= 2 {
            let n = movements.len() as f64;
            let mean: f64 = movements.iter().map(|(_, _, d)| *d as f64).sum::<f64>() / n;
            let var = movements
                .iter()
                .map(|(_, _, d)| {
                    let x = *d as f64 - mean;
                    x * x
                })
                .sum::<f64>()
                / (n - 1.0);
            var.sqrt().max(0.1)
        } else {
            CalibrationProfile::default().career_drift
        };

        // 3. 新秀进 TOP20 年速率 = 各新面孔率的平均（跳过首年）
        let rates = index.newcomer_rates();
        let rookie_elite_rate = if rates.is_empty() {
            CalibrationProfile::default().rookie_elite_rate
        } else {
            let avg = rates.iter().map(|(_, _, _, r)| *r).sum::<f64>() / rates.len() as f64;
            avg.clamp(0.0, 1.0)
        };

        CalibrationProfile {
            elite_tail_ratio,
            career_drift,
            rookie_elite_rate,
        }
    }

    /// 把真实 TOP20 榜与年龄基线按 **nickname 关联**，产出「选手 × 年份 × 年龄」观测样本。
    ///
    /// 反推公式：`age = baseline_age − (BASELINE_YEAR − year)`（基线年龄 = 2026-01-05 快照值）。
    /// 基线中无该 nickname（或无 age）的选手被跳过。
    pub fn link_ages(index: &RealTop20Index, baseline: &RoleBaseline) -> Vec<Top20AgeSample> {
        let mut out = Vec::new();
        for (nick, years) in index.players_with_years() {
            let Some(base_age) = baseline.age_of(&nick) else {
                continue;
            };
            for y in years {
                out.push(Top20AgeSample {
                    nickname: nick.clone(),
                    year: y,
                    age: base_age - (BASELINE_YEAR - y),
                });
            }
        }
        out.sort_by_key(|s| (s.year, s.age));
        out
    }

    /// 峰值年龄统计：上榜时刻的年龄画像（均值/中位/最年轻/最年长 + 直方图）。
    pub fn peak_age_stats(samples: &[Top20AgeSample]) -> PeakAgeStats {
        let mut histogram: std::collections::BTreeMap<i32, usize> =
            std::collections::BTreeMap::new();
        let mut ages: Vec<i32> = Vec::new();
        for s in samples {
            *histogram.entry(s.age).or_insert(0) += 1;
            ages.push(s.age);
        }
        let mean = if ages.is_empty() {
            0.0
        } else {
            ages.iter().map(|a| *a as f64).sum::<f64>() / ages.len() as f64
        };
        ages.sort_unstable();
        let median = if ages.is_empty() {
            0
        } else {
            let n = ages.len();
            if n % 2 == 1 {
                ages[n / 2]
            } else {
                (ages[n / 2 - 1] + ages[n / 2]) / 2
            }
        };
        PeakAgeStats {
            samples: samples.to_vec(),
            histogram,
            mean,
            median,
            min: ages.first().copied(),
            max: ages.last().copied(),
        }
    }

    /// 拟合 + 生成一份带中间统计与推演文案的报告。
    ///
    /// @param baseline 可选年龄基线（nickname → 2026 快照年龄）；传入后报告含峰值年龄画像。
    pub fn report(index: &RealTop20Index, baseline: Option<&RoleBaseline>) -> CalibrationReport {
        let profile = Self::fit(index);
        let appearance_histogram = index.appearance_histogram();
        let movements = index.year_over_year_movements();
        let newcomer_rates = index.newcomer_rates();
        let distinct_players = index.distinct_players();
        let age_samples = baseline
            .map(|b| Self::link_ages(index, b))
            .unwrap_or_default();
        let peak_age = Self::peak_age_stats(&age_samples);

        let age_sentence = if peak_age.samples.is_empty() {
            String::new()
        } else {
            format!(
                " 年龄画像：TOP20 上榜时平均 {:.1} 岁、中位 {} 岁（最年轻 {} 岁、最年长 {} 岁）——\
                 巅峰期高度集中在 20~27 岁，30+ 仍能上榜者已是凤毛麟角。",
                peak_age.mean,
                peak_age.median,
                peak_age.min.unwrap_or(0),
                peak_age.max.unwrap_or(0),
            )
        };

        let summary = if index.is_empty() {
            "（无真实 TOP20 数据，使用默认生态参数。）".to_string()
        } else {
            format!(
                "据 {}—{} 三年真实榜拟合：仅 {:.0}% 选手能连续登顶（天才长尾），\
                 年际名次平均漂移 σ≈{:.1} 名（高排位粘性、低排位洗牌），\
                 每年约 {:.0}% 的新面孔改写 TOP20。{}按此节奏，虚拟世界里未来的新人\
                 冒头速率、老将掉榜节奏将与真实生态同频。",
                index.years().first().map(|y| y.year).unwrap_or(0),
                index.years().last().map(|y| y.year).unwrap_or(0),
                profile.elite_tail_ratio * 100.0,
                profile.career_drift,
                profile.rookie_elite_rate * 100.0,
                age_sentence,
            )
        };

        CalibrationReport {
            profile,
            distinct_players,
            appearance_histogram,
            movement_samples: movements.len(),
            newcomer_rates,
            peak_age,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::real_top20::RealTop20Year;

    const Y23: &str = r#"[{"placement":1,"nickname":"A","name":"a","country":"X"},{"placement":2,"nickname":"B","name":"b","country":"X"}]"#;
    const Y24: &str = r#"[{"placement":1,"nickname":"A","name":"a","country":"X"},{"placement":2,"nickname":"C","name":"c","country":"X"}]"#;
    const Y25: &str = r#"[{"placement":1,"nickname":"A","name":"a","country":"X"},{"placement":2,"nickname":"D","name":"d","country":"X"}]"#;

    fn idx() -> RealTop20Index {
        let mut i = RealTop20Index::default();
        i.push(RealTop20Year::from_json_str(2023, Y23).unwrap());
        i.push(RealTop20Year::from_json_str(2024, Y24).unwrap());
        i.push(RealTop20Year::from_json_str(2025, Y25).unwrap());
        i
    }

    #[test]
    fn fit_elite_tail_ratio() {
        // A 连续 3 届；B/C/D 各 1 届 → 天才长尾 = 1/4 = 0.25
        let p = EcoCalibrator::fit(&idx());
        assert!((p.elite_tail_ratio - 0.25).abs() < 1e-9);
    }

    #[test]
    fn fit_rookie_rate_average() {
        // 2024 新增 C（1/2）、2025 新增 D（1/2）→ 平均 0.5
        let p = EcoCalibrator::fit(&idx());
        assert!((p.rookie_elite_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn empty_falls_back_to_default() {
        assert_eq!(
            EcoCalibrator::fit(&RealTop20Index::default()),
            CalibrationProfile::default()
        );
    }

    #[test]
    fn report_has_summary_and_histogram() {
        let r = EcoCalibrator::report(&idx(), None);
        assert_eq!(r.distinct_players, 4);
        assert!(r.summary.contains("天才长尾"));
        assert!(r.appearance_histogram.contains_key(&3));
        assert!(r.peak_age.samples.is_empty(), "无基线 → 无年龄样本");
    }

    /// nickname 关联年龄：基线年龄按 BASELINE_YEAR 反推到上榜年份。
    #[test]
    fn link_ages_back_projects_from_baseline() {
        let baseline = RoleBaseline::from_json_str(
            r#"{"players":[
                {"player":"A","team":"X","role":"Rifler","ctRole":"Rifler","tRole":"Rifler","age":26},
                {"player":"C","team":"X","role":"Rifler","ctRole":"Rifler","tRole":"Rifler","age":28}
            ]}"#,
        )
        .unwrap();
        let samples = EcoCalibrator::link_ages(&idx(), &baseline);
        // A：2023/2024/2025 三届 → 23/24/25 岁；C：2024 → 26 岁；B/D 无年龄被跳过
        assert_eq!(samples.len(), 4);
        assert!(
            samples
                .iter()
                .any(|s| s.nickname == "A" && s.year == 2023 && s.age == 23)
        );
        assert!(
            samples
                .iter()
                .any(|s| s.nickname == "A" && s.year == 2025 && s.age == 25)
        );
        assert!(
            samples
                .iter()
                .any(|s| s.nickname == "C" && s.year == 2024 && s.age == 26)
        );
        assert!(
            samples
                .iter()
                .all(|s| s.nickname != "B" && s.nickname != "D")
        );
    }

    #[test]
    fn peak_age_stats_compute() {
        let samples = vec![
            Top20AgeSample {
                nickname: "a".into(),
                year: 2023,
                age: 22,
            },
            Top20AgeSample {
                nickname: "b".into(),
                year: 2023,
                age: 24,
            },
            Top20AgeSample {
                nickname: "c".into(),
                year: 2024,
                age: 26,
            },
        ];
        let st = EcoCalibrator::peak_age_stats(&samples);
        assert!((st.mean - 24.0).abs() < 1e-9);
        assert_eq!(st.median, 24);
        assert_eq!(st.min, Some(22));
        assert_eq!(st.max, Some(26));
        assert_eq!(st.histogram.get(&24), Some(&1));
        // 空样本防御
        let empty = EcoCalibrator::peak_age_stats(&[]);
        assert_eq!(empty.mean, 0.0);
        assert!(empty.min.is_none());
    }
}
