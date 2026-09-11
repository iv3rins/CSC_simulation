//! 生涯结局与复盘（Career Ending）——退役时把冷冰冰的生涯汇总翻译成**有温度的
//! 生涯定论**。
//!
//! 定位（补全「结局/复盘」产品缺口）：`CareerArchive::totals_of` 只给数字
//! （杀敌/胜场/收入/荣誉列表），本模块把这些数字+印记+荣誉综合为一篇
//! 「生涯定论」：**传奇等级 + 一句话定论 + 逐段复盘**，供退役页/通关页渲染。
//!
//! 纯函数、无状态、无随机——同输入同输出（复盘是表现层派生，不进入模拟状态）。
//!
//! 传奇等级（2026 阶梯化重构——消除荣誉通胀，用户规格）：
//! - `Goat`（历史唯一/传奇）：门槛① Major 冠军 ≥3 且 TOP20 榜首 ≥3
//!   （或榜首 ≥2 且 TOP20 前三 ≥5）；门槛② 累计 MVP ≥10 且 Major MVP ≥3。
//!   同历史时期最多一位（动态 GOAT 评分最高者——单主角模式自然唯一）。
//! - `EraIcon`（时代统治者）：Major ≥2 且 TOP1 ≥2 且 MVP ≥5 且 TOP20 上榜 ≥5
//!   （「维持 T1 级竞技状态 5 年」的代理口径）。
//! - `HallOfFame`（名流堂）：Major ≥1 且 TOP1 ≥1 且 MVP ≥2（原 GOAT 标准降级）。
//! - `Legend`（传奇）：Major 冠军，或 TOP20 榜首，或 ≥5 次上榜；
//! - `Star`（名将）：有年度 TOP20 或赛事 MVP；
//! - `Journeyman`（名宿）：有完整生涯但缺顶级荣誉；
//! - `Rookie`（新秀）：生涯尚短，未有定论。

use csc_domain::tourney_tier::TourneyTier;
use csc_entities::career::{CareerInfo, IndividualHonourType};
use csc_entities::mark::CareerMarkType;
use csc_util::id::PlayerId;

use crate::archive::SeasonTotals;

/// 生涯传奇等级。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LegacyTier {
    /// 生涯尚短，未有定论
    Rookie,
    /// 名宿：完整生涯但缺顶级荣誉
    Journeyman,
    /// 名将：有年度 TOP20 或精英赛 MVP
    Star,
    /// 传奇：Major 冠军或多次年度 TOP20
    Legend,
    /// 名流堂：1× Major + 1× TOP1 + 2× MVP（原 GOAT 标准降级）
    HallOfFame,
    /// 时代统治者：2× Major + 2× TOP1 + 5× MVP + 5 年 T1 竞技状态
    EraIcon,
    /// 历史唯一（GOAT）：3× Major + 3× TOP1 + 10 MVP（含 3 Major MVP）
    Goat,
}

impl LegacyTier {
    /// 等级中文名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Rookie => "新锐",
            Self::Journeyman => "职业名宿",
            Self::Star => "一线名将",
            Self::Legend => "传奇选手",
            Self::HallOfFame => "名流堂",
            Self::EraIcon => "时代统治者",
            Self::Goat => "历史第一人（GOAT）",
        }
    }
}

/// 生涯定论（一篇结构化复盘）。
#[derive(Debug, Clone, PartialEq)]
pub struct CareerEnding {
    /// 传奇等级
    pub tier: LegacyTier,
    /// 动态 GOAT 评分（跨选手比较用；同历史时期唯一 GOAT = 评分最高者）
    pub goat_score: f64,
    /// 一句话定论（标题）
    pub verdict: String,
    /// 逐段复盘（正文，\n 分段）
    pub review: String,
}

/// 生涯结局评估器——纯函数。
pub struct CareerEndingEvaluator;

impl CareerEndingEvaluator {
    /// 综合生涯汇总 + **结构化荣誉**（`CareerInfo.honours`）+ 印记，产出结局定论。
    ///
    /// @param player_id 玩家稳定 ID（归属，叙事用名字另传）
    /// @param player_name 玩家昵称（叙事用）
    /// @param totals 生涯汇总（`CareerArchive::totals_of`）
    /// @param career 结构化生涯（荣誉按 tier/rank 精确计数——字符串摘要会丢层级）
    pub fn evaluate(
        player_id: PlayerId,
        player_name: &str,
        totals: &SeasonTotals,
        career: &CareerInfo,
        text: &csc_text::TextBundle,
    ) -> CareerEnding {
        let _ = player_id;

        // —— 结构化荣誉计数（不依赖字符串解析）——
        let mut major_titles = 0usize;
        let mut top20_times = 0usize;
        let mut top1_times = 0usize;
        let mut top3_times = 0usize;
        let mut mvp_total = 0usize;
        let mut major_mvps = 0usize;
        for t in &career.honours.team {
            if t.tier == TourneyTier::Major {
                major_titles += 1;
            }
        }
        for h in &career.honours.individual {
            match h.r#type {
                IndividualHonourType::Top20 => {
                    top20_times += 1;
                    match h.rank {
                        Some(1) => top1_times += 1,
                        Some(r) if r <= 3 => top3_times += 1,
                        _ => {}
                    }
                    if h.rank == Some(1) {
                        top3_times += 1; // 榜首也算前三
                    }
                }
                IndividualHonourType::Mvp => {
                    mvp_total += 1;
                    if h.tier == Some(TourneyTier::Major) {
                        major_mvps += 1;
                    }
                }
                _ => {}
            }
        }
        let toxic = totals.marks.contains(&CareerMarkType::Toxic);
        let hard_worker = totals.marks.contains(&CareerMarkType::HardWorker);

        // —— 动态 GOAT 评分（跨选手/跨时代比较；单主角模式唯一 GOAT = 最高分者）——
        let goat_score = (top1_times as f64) * 100.0
            + (major_titles as f64) * 80.0
            + (major_mvps as f64) * 30.0
            + (mvp_total as f64) * 5.0
            + (top3_times as f64) * 20.0
            + (top20_times as f64) * 5.0;

        // —— 传奇等级判定（2026 阶梯化：消除荣誉通胀）——
        let goat_gate1 =
            (major_titles >= 3 && top1_times >= 3) || (top1_times >= 2 && top3_times >= 5);
        let goat_gate2 = mvp_total >= 10 && major_mvps >= 3;
        let era_icon = major_titles >= 2 && top1_times >= 2 && mvp_total >= 5 && top20_times >= 5;
        let hall_of_fame = major_titles >= 1 && top1_times >= 1 && mvp_total >= 2;
        let tier = if goat_gate1 && goat_gate2 {
            LegacyTier::Goat
        } else if era_icon {
            LegacyTier::EraIcon
        } else if hall_of_fame {
            LegacyTier::HallOfFame
        } else if major_titles >= 1 || top1_times >= 1 || top20_times >= 5 {
            LegacyTier::Legend
        } else if top20_times >= 1 || mvp_total >= 1 {
            LegacyTier::Star
        } else if totals.seasons >= 2 {
            LegacyTier::Journeyman
        } else {
            LegacyTier::Rookie
        };

        // —— 一句话定论 ——
        let verdict = match tier {
            LegacyTier::Goat => text.format("ending.verdict.goat", &[player_name]),
            LegacyTier::EraIcon => text.format("ending.verdict.era_icon", &[player_name]),
            LegacyTier::HallOfFame => text.format("ending.verdict.hall_of_fame", &[player_name]),
            LegacyTier::Legend => text.format("ending.verdict.legend", &[player_name]),
            LegacyTier::Star => {
                if toxic {
                    text.format("ending.verdict.star_toxic", &[player_name])
                } else {
                    text.format("ending.verdict.star", &[player_name])
                }
            }
            LegacyTier::Journeyman => text.format("ending.verdict.journeyman", &[player_name]),
            LegacyTier::Rookie => text.format("ending.verdict.rookie", &[player_name]),
        };

        // —— 逐段复盘 ——
        let mut lines: Vec<String> = vec![text.format(
            "ending.review.summary",
            &[
                &totals.seasons.to_string(),
                &totals.total_kills.to_string(),
                &totals.total_wins.to_string(),
                &money_text(totals.total_earnings),
            ],
        )];
        if totals.seasons > 0 {
            lines.push(text.format(
                "ending.review.peak",
                &[&format!("{:.2}", totals.peak_rating)],
            ));
        }

        // 荣誉段落（结构化计数：Major 冠军 / TOP20 榜首 / MVP 总量 / Major MVP）
        lines.push(text.format(
            "ending.review.honours",
            &[
                &major_titles.to_string(),
                &top1_times.to_string(),
                &mvp_total.to_string(),
                &major_mvps.to_string(),
            ],
        ));

        // 印记段落（长线选择的后果在此具象化）
        let mark_line = if toxic && hard_worker {
            text.get("ending.mark.toxic_worker").to_string()
        } else if toxic {
            text.get("ending.mark.toxic").to_string()
        } else if hard_worker {
            text.get("ending.mark.worker").to_string()
        } else if totals.marks.is_empty() {
            text.get("ending.mark.plain").to_string()
        } else {
            text.get("ending.mark.signed").to_string()
        };
        lines.push(mark_line);

        CareerEnding {
            tier,
            goat_score,
            verdict,
            review: lines.join("\n"),
        }
    }
}

/// 金额中文简写（万）。
fn money_text(v: i64) -> String {
    if v >= 10_000_000 {
        format!("{:.1} 千万", v as f64 / 10_000_000.0)
    } else if v >= 10_000 {
        format!("{:.0} 万", v as f64 / 10_000.0)
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_entities::career::{IndividualHonour, TeamHonour};

    fn totals(seasons: i32, peak: f64, marks: Vec<CareerMarkType>) -> SeasonTotals {
        SeasonTotals {
            seasons,
            total_kills: 5000,
            total_wins: 200,
            total_earnings: 12_000_000,
            peak_rating: peak,
            honours: Vec::new(),
            marks,
        }
    }

    /// 构造结构化生涯：`major_titles` 座 Major、`top1` 次榜首、`top3` 次前三、
    /// `mvp` 次 MVP（其中 `major_mvp` 次为 Major MVP）、总 TOP20 上榜 `top20_total` 次。
    fn career(
        major_titles: usize,
        top1: usize,
        top3: usize,
        mvp: usize,
        major_mvp: usize,
        top20_total: usize,
    ) -> CareerInfo {
        let mut c = CareerInfo::free_agent_default(2020);
        for _ in 0..major_titles {
            c.honours.team.push(TeamHonour {
                title: "Shanghai Major".into(),
                year: 2025,
                tier: TourneyTier::Major,
                detail: String::new(),
            });
        }
        for i in 0..top1 {
            c.honours.individual.push(IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year: 2024 + i as i32,
                detail: "TOP20 第 1 名".into(),
                tier: None,
                event_name: None,
                rank: Some(1),
            });
        }
        for _ in 0..top3.saturating_sub(top1) {
            c.honours.individual.push(IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year: 2024,
                detail: "TOP20 第 3 名".into(),
                tier: None,
                event_name: None,
                rank: Some(3),
            });
        }
        // 剩余上榜（第 10 名垫名次）补足 top20_total
        for _ in 0..top20_total.saturating_sub(top3.max(top1)) {
            c.honours.individual.push(IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year: 2024,
                detail: "TOP20 第 10 名".into(),
                tier: None,
                event_name: None,
                rank: Some(10),
            });
        }
        for i in 0..mvp {
            let major = i < major_mvp;
            c.honours.individual.push(IndividualHonour {
                r#type: IndividualHonourType::Mvp,
                year: 2024,
                detail: "X MVP".into(),
                tier: Some(if major {
                    TourneyTier::Major
                } else {
                    TourneyTier::T1
                }),
                event_name: Some("X".into()),
                rank: None,
            });
        }
        c
    }

    #[test]
    fn goat_requires_full_gate() {
        // 3 Major + 3 TOP1 + 10 MVP（含 3 Major MVP）→ GOAT
        let t = totals(8, 1.40, vec![CareerMarkType::HardWorker]);
        let c = career(3, 3, 3, 10, 3, 5);
        let e = CareerEndingEvaluator::evaluate(
            PlayerId(1),
            "ZywOo",
            &t,
            &c,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(e.tier, LegacyTier::Goat);
        assert!(e.goat_score > 0.0);
        assert!(e.verdict.contains("ZywOo"));
    }

    #[test]
    fn goat_alt_path_top3() {
        // 2 TOP1 + 5 TOP20 前三 + 10 MVP（含 3 Major MVP）→ GOAT（替代门槛）
        let t = totals(8, 1.35, vec![]);
        let c = career(0, 2, 5, 10, 3, 5);
        assert_eq!(
            CareerEndingEvaluator::evaluate(
                PlayerId(1),
                "P",
                &t,
                &c,
                &csc_text::TextBundle::default()
            )
            .tier,
            LegacyTier::Goat
        );
    }

    #[test]
    fn era_icon_below_goat_gate() {
        // 2 Major + 2 TOP1 + 5 MVP + 5 上榜，但 MVP 不足 10 → 时代统治者（非 GOAT）
        let t = totals(6, 1.30, vec![]);
        let c = career(2, 2, 2, 5, 1, 5);
        assert_eq!(
            CareerEndingEvaluator::evaluate(
                PlayerId(1),
                "P",
                &t,
                &c,
                &csc_text::TextBundle::default()
            )
            .tier,
            LegacyTier::EraIcon
        );
    }

    #[test]
    fn hall_of_fame_is_downgraded_old_goat() {
        // 1 Major + 1 TOP1 + 2 MVP：原 GOAT 标准 → 名流堂
        let t = totals(5, 1.25, vec![]);
        let c = career(1, 1, 1, 2, 1, 1);
        assert_eq!(
            CareerEndingEvaluator::evaluate(
                PlayerId(1),
                "P",
                &t,
                &c,
                &csc_text::TextBundle::default()
            )
            .tier,
            LegacyTier::HallOfFame
        );
    }

    #[test]
    fn legend_tier_via_major() {
        let t = totals(5, 1.25, vec![]);
        let c = career(1, 0, 0, 0, 0, 0);
        assert_eq!(
            CareerEndingEvaluator::evaluate(
                PlayerId(1),
                "P",
                &t,
                &c,
                &csc_text::TextBundle::default()
            )
            .tier,
            LegacyTier::Legend
        );
    }

    #[test]
    fn rookie_tier_short_career() {
        let t = totals(1, 1.0, vec![]);
        assert_eq!(
            CareerEndingEvaluator::evaluate(
                PlayerId(1),
                "P",
                &t,
                &career(0, 0, 0, 0, 0, 0),
                &csc_text::TextBundle::default()
            )
            .tier,
            LegacyTier::Rookie
        );
    }

    #[test]
    fn toxic_mark_surfaces_in_review() {
        let t = totals(3, 1.1, vec![CareerMarkType::Toxic]);
        let mut c = career(0, 0, 0, 0, 0, 1);
        c.honours.individual.push(IndividualHonour {
            r#type: IndividualHonourType::Top20,
            year: 2025,
            detail: "TOP20 第 5 名".into(),
            tier: None,
            event_name: None,
            rank: Some(5),
        });
        let e = CareerEndingEvaluator::evaluate(
            PlayerId(1),
            "P",
            &t,
            &c,
            &csc_text::TextBundle::default(),
        );
        assert_eq!(e.tier, LegacyTier::Star);
        assert!(
            e.review.contains("暗礁"),
            "toxic 印记在复盘具象化: {}",
            e.review
        );
    }
}
