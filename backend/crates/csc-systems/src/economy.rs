//! 生涯经济子系统（Kotlin `EconomyEngine.kt` 转写）：玩家侧财务（奖金分成 + 代言合同）。
//!
//! 分层：
//! - 规则：`csc-simulation::FinanceModel`（纯函数：分成比例/代言价值/转会费）；
//! - 状态：`CareerInfo.finance`（玩家侧）+ `Team.budget`（队伍侧）；
//! - 决策：代言邀约 → `DecisionPoint::SponsorshipOffer`（接受/拒绝）。

use csc_decision::point::{DecisionPoint, SponsorOption};
use csc_entities::mark::{CareerMarkType, CareerMarks};
use csc_entities::world::World;
use csc_simulation::finance::FinanceModel;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;

/// 生涯经济子系统：无跨调用状态（journal 不持有——本引擎不写日志）。
pub struct EconomyEngine;

impl EconomyEngine {
    // —— 奖金分成（赛事引擎赛后调用）——

    /// 玩家奖金分成：按名次比例计入玩家现金与生涯收益（placement 1=冠军）。
    pub fn award_prize_share(
        world: &mut World,
        player_id: PlayerId,
        pool: i64,
        placement: i32,
        year: i32,
    ) {
        let share = FinanceModel::prize_share(pool, placement);
        if share <= 0 {
            return;
        }
        let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        else {
            return;
        };
        let finance = &mut career.finance;
        finance.cash += share;
        finance.career_earnings += share;
        *finance.earnings_by_year.entry(year).or_insert(0) += share;
    }

    // —— 月度代言收入 ——

    /// 月度 tick：代言月付 + 年度评估（跨年后的首次调用触发评估点）。
    pub fn monthly_tick(
        world: &mut World,
        player_id: PlayerId,
        date: &str,
        batch: &mut Vec<DecisionPoint>,
        rng: &mut Xoshiro256StarStar,
    ) {
        let Some(year) = date.get(..4).and_then(|s| s.parse::<i32>().ok()) else {
            return;
        };
        Self::monthly_income(world, player_id);
        Self::monthly_salary(world, player_id, year);
        let last = world
            .player(player_id)
            .expect("玩家不存在")
            .career
            .as_ref()
            .map(|c| c.finance.last_sponsor_check_year)
            .unwrap_or(0);
        if year > last {
            Self::annual_sponsor_check(world, player_id, year, date, batch, rng);
        }
    }

    /// 月度代言收入：生效中代言按月付收入。
    pub fn monthly_income(world: &mut World, player_id: PlayerId) {
        let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        else {
            return;
        };
        let finance = &mut career.finance;
        let monthly: i64 = finance.sponsors.iter().map(|s| s.annual_value / 12).sum();
        if monthly > 0 {
            finance.cash += monthly;
            finance.career_earnings += monthly;
        }
    }

    /// 月度工资：合同期内每月发放 `salary / 12`（2026 修复：此前薪资只在签约时
    /// 扣队伍预算，玩家从未收到工资——经济闭环只有支出没有收入端）。
    /// 强制赋闲（待业 ≥12 月，薪资归零）不发放。
    pub fn monthly_salary(world: &mut World, player_id: PlayerId, year: i32) {
        let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        else {
            return;
        };
        if career.contract_years <= 0 || career.months_unsigned >= 12 || career.salary <= 0 {
            return;
        }
        let monthly = csc_simulation::FinanceModel::monthly_salary(career.salary);
        let finance = &mut career.finance;
        finance.cash += monthly;
        finance.career_earnings += monthly;
        *finance.earnings_by_year.entry(year).or_insert(0) += monthly;
    }

    /// 年度代言评估（跨年结算调用）：合同结转、过期移除、
    /// 声誉达标且无活跃合同时生成 `DecisionPoint::SponsorshipOffer`。
    pub fn annual_sponsor_check(
        world: &mut World,
        player_id: PlayerId,
        year: i32,
        date: &str,
        batch: &mut Vec<DecisionPoint>,
        rng: &mut Xoshiro256StarStar,
    ) {
        // 合同年度结转（yearsLeft -1，到期移除）
        {
            let Some(career) = world
                .player_mut(player_id)
                .expect("玩家不存在")
                .career_mut()
            else {
                return;
            };
            career.finance.sponsors.retain(|s| !s.tick_year().expired());
            for s in &mut career.finance.sponsors {
                *s = s.tick_year();
            }
            if year <= career.finance.last_sponsor_check_year {
                return;
            }
            career.finance.last_sponsor_check_year = year;
        }
        let rep = world
            .player(player_id)
            .expect("玩家不存在")
            .career
            .as_ref()
            .map(|c| c.reputation)
            .unwrap_or(0);
        if rep < FinanceModel::SPONSOR_REPUTATION_THRESHOLD {
            return;
        }
        let sponsors_len = world
            .player(player_id)
            .expect("玩家不存在")
            .career
            .as_ref()
            .map(|c| c.finance.sponsors.len())
            .unwrap_or(0);
        if sponsors_len > 0 {
            return;
        }
        let Some(brand) = FinanceModel::top_brand(rep) else {
            return;
        };
        let value = FinanceModel::sponsor_annual_value(rep);
        let years = 2 + rng.next_i32_bound(2); // Kotlin 单参数 `random.nextInt(2)`（nextLong 族原语）
        let player_name = world.player(player_id).expect("玩家不存在").name.clone();
        batch.push(DecisionPoint::SponsorshipOffer {
            id: format!("{date}|sponsor|{player_id}"),
            date: date.to_string(),
            player_id,
            player_name,
            brand: brand.to_string(),
            annual_value: value,
            years,
            options: vec![
                SponsorOption {
                    id: "ACCEPT".into(),
                    label: "接受代言".into(),
                    description: format!("{brand} 每年 {}k × {years} 年", value / 1000),
                },
                SponsorOption {
                    id: "DECLINE".into(),
                    label: "拒绝".into(),
                    description: "保持专注，等待更好机会".into(),
                },
            ],
        });
    }

    /// 应用代言决策（ACCEPT → 签约；DECLINE → 无）。
    pub fn apply_sponsor_decision(
        world: &mut World,
        player_id: PlayerId,
        option_id: &str,
        year: i32,
        point: &DecisionPoint,
    ) {
        if option_id != "ACCEPT" {
            return;
        }
        let DecisionPoint::SponsorshipOffer {
            brand,
            annual_value,
            years,
            ..
        } = point
        else {
            return;
        };
        let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        else {
            return;
        };
        if career.finance.sponsors.iter().any(|s| s.brand == *brand) {
            return; // 防御：重复签约
        }
        career
            .finance
            .sponsors
            .push(csc_entities::career::Sponsorship {
                brand: brand.clone(),
                annual_value: *annual_value,
                years_left: *years,
                signed_year: year,
            });
        // 代言明星印记（长期高声誉 + 代言 → 未来更值钱）
        CareerMarks::apply(
            &mut career.marks,
            CareerMarkType::SponsorMagnet,
            year,
            point.id(),
            1,
        );
    }

    /// 决策选项 id 常量。
    pub const ACCEPT: &'static str = "ACCEPT";
    pub const DECLINE: &'static str = "DECLINE";
}

/// 生涯资金运用（2026 复审新增）：把可支配现金变成生涯杠杆——
/// **投资自己**（属性成长）/ **购买 CS 饰品**（藏品 + 声誉）。
/// 买断自己换队在 `TransferEngine::buyout`（转会语义，需要 VRS/RNG）。
///
/// **可复现性契约**：服务端外部操作不进决策日志，因此本引擎**零 RNG 消费**——
/// 加成量与选品由 `(PlayerId, year, salt)` 哈希派生（同世界同结果），
/// 不推进引擎 RNG 序列，不破坏「种子 + 决策日志 = 一致世界」。
pub struct InvestmentEngine;

impl InvestmentEngine {
    /// 投资自己成本（每年限一次）。
    pub const INVEST_COST: i64 = 50_000;
    /// 标准饰品成本。
    pub const SKIN_STANDARD_COST: i64 = 20_000;
    /// 稀有饰品成本。
    pub const SKIN_RARE_COST: i64 = 80_000;

    /// 普通饰品池（CS 经典款）。
    pub const SKIN_STANDARD_POOL: [&'static str; 12] = [
        "AK-47 | Redline",
        "AWP | Asiimov",
        "Desert Eagle | Printstream",
        "M4A1-S | Printstream",
        "USP-S | Kill Confirmed",
        "AK-47 | Vulcan",
        "M4A4 | Neo-Noir",
        "P250 | Asiimov",
        "AWP | Neo-Noir",
        "Glock-18 | Fade",
        "M4A4 | Desolate Space",
        "MP9 | Wild Lily",
    ];
    /// 稀有饰品池（CS 传世款）。
    pub const SKIN_RARE_POOL: [&'static str; 10] = [
        "AWP | Dragon Lore",
        "M4A4 | Howl",
        "Butterfly Knife | Marble Fade",
        "Karambit | Fade",
        "Flip Knife | Doppler",
        "AK-47 | Fire Serpent",
        "M4A4 | Poseidon",
        "AWP | Fade",
        "AK-47 | Wild Lotus",
        "Karambit | Case Hardened",
    ];

    /// 确定性混合哈希：(PlayerId, year, salt) → u64（与 Top20Evaluator 同风格，
    /// 纯函数、不消费 RNG）。
    fn mix(player_id: PlayerId, year: i32, salt: u64) -> u64 {
        (player_id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (year as u64).wrapping_mul(0x517C_C1B7_2722_0A95)
            ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93)
    }

    /// 投资自己（**每年限一次**）：选择方向 → 对应属性 +1..=3 点（确定性），
    /// 声誉 +2。方向：AIM（枪法）/ ENDURANCE（体能）/ MENTAL（心态）。
    /// @return 描述文本（写日志用）
    pub fn invest_self(
        world: &mut World,
        player_id: PlayerId,
        focus: &str,
        year: i32,
    ) -> Result<String, String> {
        // 预校验（借用不可变读，避免可变借用冲突）
        {
            let Some(career) = world.player(player_id).and_then(|p| p.career.as_ref()) else {
                return Err("尚无生涯数据".to_string());
            };
            if career.finance.last_invest_year == year {
                return Err("本年度已经投资过自己——每年限一次".to_string());
            }
            if career.finance.cash < Self::INVEST_COST {
                return Err(format!(
                    "现金不足：投资自己需要 {}（可先打比赛/接代言攒钱）",
                    Self::INVEST_COST
                ));
            }
        }
        let gain = 1 + (Self::mix(player_id, year, 11) % 3) as i32;
        let (label, attr_name) = match focus {
            "AIM" => ("枪法", "aim"),
            "ENDURANCE" => ("体能", "endurance"),
            "MENTAL" => ("心态", "mentality"),
            _ => return Err(format!("未知投资方向：{focus}（AIM/ENDURANCE/MENTAL）")),
        };
        // 应用属性（clamp 0..=100）
        let pc = world
            .player_mut(player_id)
            .ok_or("玩家不存在".to_string())?;
        match attr_name {
            "aim" => pc.skill.aim = (pc.skill.aim + gain).min(100),
            "endurance" => pc.base.endurance = (pc.base.endurance + gain).min(100),
            _ => pc.pro.mentality = (pc.pro.mentality + gain).min(100),
        }
        let career = pc.career_mut().ok_or("尚无生涯数据".to_string())?;
        career.finance.cash -= Self::INVEST_COST;
        career.finance.last_invest_year = year;
        career.reputation = (career.reputation + 2).min(100);
        Ok(format!(
            "你投入 {} 特训{label}：{attr_name} +{gain}，声誉 +2（年度限一次）",
            Self::INVEST_COST
        ))
    }

    /// 购买 CS 饰品：标准/稀有两档，确定性选品（同世界同结果），
    /// 标准声誉 +1、稀有 +3。@return 入手的饰品
    pub fn buy_skin(
        world: &mut World,
        player_id: PlayerId,
        rare: bool,
        year: i32,
    ) -> Result<csc_entities::career::SkinOwned, String> {
        let cost = if rare {
            Self::SKIN_RARE_COST
        } else {
            Self::SKIN_STANDARD_COST
        };
        // 预校验
        let Some(career) = world.player(player_id).and_then(|p| p.career.as_ref()) else {
            return Err("尚无生涯数据".to_string());
        };
        if career.finance.cash < cost {
            return Err(format!(
                "现金不足：{} 饰品需要 {cost}（标准 {}/ 稀有 {}）",
                if rare { "稀有" } else { "标准" },
                Self::SKIN_STANDARD_COST,
                Self::SKIN_RARE_COST
            ));
        }
        let owned_count = career.finance.skins.len() as u64;
        let pool = if rare {
            &Self::SKIN_RARE_POOL[..]
        } else {
            &Self::SKIN_STANDARD_POOL[..]
        };
        let name = pool[(Self::mix(player_id, year, 7 + owned_count) % pool.len() as u64) as usize]
            .to_string();
        let skin = csc_entities::career::SkinOwned {
            name,
            rarity: if rare {
                "rare".into()
            } else {
                "standard".into()
            },
            value: cost,
            year,
        };
        let pc = world
            .player_mut(player_id)
            .ok_or("玩家不存在".to_string())?;
        let career = pc.career_mut().ok_or("尚无生涯数据".to_string())?;
        career.finance.cash -= cost;
        career.finance.skins.push(skin.clone());
        career.reputation = (career.reputation + if rare { 3 } else { 1 }).min(100);
        Ok(skin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;

    fn player_with_reputation(w: &mut World, rep: i32) -> PlayerId {
        let p = w.create_player(
            Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(7),
            None,
            None,
        );
        w.player_mut(p).unwrap().career.as_mut().unwrap().reputation = rep;
        p
    }

    #[test]
    fn prize_share_accumulates() {
        let mut w = World::new();
        let p = player_with_reputation(&mut w, 50);
        EconomyEngine::award_prize_share(&mut w, p, 1_000_000, 1, 2026);
        let c = w.player(p).unwrap().career.as_ref().unwrap();
        // 冠军 15%
        assert_eq!(c.finance.cash, 150_000);
        assert_eq!(c.finance.career_earnings, 150_000);
        assert_eq!(c.finance.earnings_of(2026), 150_000);
    }

    #[test]
    fn monthly_income_from_sponsors() {
        let mut w = World::new();
        let p = player_with_reputation(&mut w, 50);
        w.player_mut(p)
            .unwrap()
            .career
            .as_mut()
            .unwrap()
            .finance
            .sponsors
            .push(csc_entities::career::Sponsorship {
                brand: "Nike".into(),
                annual_value: 120_000,
                years_left: 2,
                signed_year: 2026,
            });
        EconomyEngine::monthly_income(&mut w, p);
        assert_eq!(
            w.player(p).unwrap().career.as_ref().unwrap().finance.cash,
            10_000
        );
    }

    #[test]
    fn sponsor_check_generates_offer_only_when_eligible() {
        let mut w = World::new();
        let p = player_with_reputation(&mut w, 50); // 声誉 50 < 70 门槛
        let mut batch = Vec::new();
        EconomyEngine::annual_sponsor_check(
            &mut w,
            p,
            2026,
            "2026-06-08",
            &mut batch,
            &mut Xoshiro256StarStar::seed(1),
        );
        assert!(batch.is_empty(), "声誉不足不应生成代言邀约");

        let p2 = player_with_reputation(&mut w, 85);
        EconomyEngine::annual_sponsor_check(
            &mut w,
            p2,
            2026,
            "2026-06-08",
            &mut batch,
            &mut Xoshiro256StarStar::seed(1),
        );
        assert_eq!(batch.len(), 1);
        let DecisionPoint::SponsorshipOffer {
            brand,
            annual_value,
            years,
            options,
            ..
        } = &batch[0]
        else {
            panic!("应为代言点");
        };
        assert!(!brand.is_empty());
        assert!(annual_value > &0);
        assert!((2..=3).contains(years));
        assert_eq!(options.len(), 2);
    }

    #[test]
    fn apply_sponsor_decision_signs_contract() {
        let mut w = World::new();
        let p = player_with_reputation(&mut w, 85);
        let mut batch = Vec::new();
        let mut rng = Xoshiro256StarStar::seed(1);
        EconomyEngine::annual_sponsor_check(&mut w, p, 2026, "2026-06-08", &mut batch, &mut rng);
        let point = batch[0].clone();
        EconomyEngine::apply_sponsor_decision(&mut w, p, "ACCEPT", 2026, &point);
        let sponsors = w
            .player(p)
            .unwrap()
            .career
            .as_ref()
            .unwrap()
            .finance
            .sponsors
            .len();
        assert_eq!(sponsors, 1);
        let marks = w.player(p).unwrap().career.as_ref().unwrap().marks.clone();
        assert!(
            marks
                .iter()
                .any(|m| m.r#type == CareerMarkType::SponsorMagnet)
        );
        // 重复签约防御
        EconomyEngine::apply_sponsor_decision(&mut w, p, "ACCEPT", 2026, &point);
        let sponsors = w
            .player(p)
            .unwrap()
            .career
            .as_ref()
            .unwrap()
            .finance
            .sponsors
            .len();
        assert_eq!(sponsors, 1);
    }
}
