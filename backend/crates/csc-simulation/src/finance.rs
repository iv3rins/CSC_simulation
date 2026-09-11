//! 经济规则模型（Kotlin `FinanceModel.kt` 转写）——生涯经济系统的**纯函数层**。

/// 经济规则——奖金分成 / 代言价值 / 转会费 / 支付能力全部收敛于此；
/// 结算在 `csc-systems::economy` 与 `csc-systems::transfer`。
///
/// 注：原型用简化模型（奖金分成比例固定、代言按声誉线性、转会费 = 培养补偿），
/// 数值可在本文件单点调整。
pub struct FinanceModel;

impl FinanceModel {
    /// 奖池分成比例（placement 1=冠军, 2=亚军, 3-4=四强, 5-8=八强, 其余 0）。
    pub fn prize_share_ratio(placement: i32) -> f64 {
        match placement {
            1 => 0.15,
            2 => 0.08,
            3..=4 => 0.04,
            5..=8 => 0.015,
            _ => 0.0,
        }
    }

    /// 玩家奖金分成（四舍五入取整；= Kotlin `prizeShare`：`(pool * ratio).toLong()` 截断）。
    pub fn prize_share(pool: i64, placement: i32) -> i64 {
        (pool as f64 * Self::prize_share_ratio(placement)) as i64
    }

    // —— 代言 ——

    /// 代言评估门槛（声誉 ≥ 该值才会收到邀约）
    pub const SPONSOR_REPUTATION_THRESHOLD: i32 = 70;

    /// 代言品牌池（按声誉解锁；index 越低门槛越低）——全部为电竞外设/能量饮料品牌。
    pub const BRANDS: [&'static str; 10] = [
        "ZOWIE",
        "HyperX",
        "SteelSeries",
        "Razer",
        "Logitech G",
        "Secretlab",
        "Red Bull",
        "ASUS ROG",
        "Gamer Supps",
        "Monster Energy",
    ];

    /// 品牌所需声誉门槛（与 [`Self::BRANDS`] 对齐）。
    pub fn brand_requirement(index: usize) -> i32 {
        70 + index as i32 * 5
    }

    /// 代言年付价值（声誉 × 基数）。
    pub fn sponsor_annual_value(reputation: i32) -> i64 {
        reputation as i64 * 8_000
    }

    /// 按声誉可解锁的品牌（供决策点选项）。
    pub fn available_brands(reputation: i32) -> Vec<&'static str> {
        Self::BRANDS
            .iter()
            .enumerate()
            .filter(|(i, _)| reputation >= Self::brand_requirement(*i))
            .map(|(_, b)| *b)
            .collect()
    }

    /// 当前最优可解锁品牌（邀约用；无则 None；= Kotlin `topBrand`）。
    pub fn top_brand(reputation: i32) -> Option<&'static str> {
        Self::available_brands(reputation).last().copied()
    }

    // —— 转会费（培养补偿模型）——

    /// 到期/自由转会时的培养补偿费：实力 × 单价（= Kotlin `transferFee`）。
    ///
    /// 量级与初始预算（VRS 积分 × 1000）同阶：top 队（~2000 分 = 200 万预算）
    /// 签 power 85 玩家（费 85 万 + 薪 42.5 万 ≈ 128 万）仍有余量，低档队自然被门槛滤掉。
    pub fn transfer_fee(power: f64) -> i64 {
        (power * 10_000.0) as i64
    }

    /// 支付能力判定（候选过滤：预算 ≥ 薪资 + 转会费）。
    pub fn can_afford(budget: i64, salary: i64, fee: i64) -> bool {
        budget >= salary + fee
    }

    // —— 薪资 ——

    /// 月度发薪（年度薪资 / 12；原型按年发，保留月付规则备查）。
    pub fn monthly_salary(annual_salary: i64) -> i64 {
        annual_salary / 12
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prize_share_ratios_match_kotlin() {
        assert_eq!(FinanceModel::prize_share_ratio(1), 0.15);
        assert_eq!(FinanceModel::prize_share_ratio(2), 0.08);
        assert_eq!(FinanceModel::prize_share_ratio(3), 0.04);
        assert_eq!(FinanceModel::prize_share_ratio(4), 0.04);
        assert_eq!(FinanceModel::prize_share_ratio(8), 0.015);
        assert_eq!(FinanceModel::prize_share_ratio(9), 0.0);
        assert_eq!(FinanceModel::prize_share_ratio(16), 0.0);
    }

    #[test]
    fn prize_share_truncates() {
        // 1M 池冠军 15% = 150,000
        assert_eq!(FinanceModel::prize_share(1_000_000, 1), 150_000);
        assert_eq!(FinanceModel::prize_share(1_000_000, 2), 80_000);
        assert_eq!(FinanceModel::prize_share(1_000_000, 5), 15_000);
        assert_eq!(FinanceModel::prize_share(1_000_000, 16), 0);
    }

    #[test]
    fn brands_unlock_by_reputation() {
        assert_eq!(FinanceModel::brand_requirement(0), 70);
        assert_eq!(FinanceModel::brand_requirement(5), 95);
        // 声誉 70：只有第一个品牌
        assert_eq!(FinanceModel::available_brands(70), vec!["ZOWIE"]);
        // 声誉 75：前两个
        assert_eq!(FinanceModel::available_brands(75), vec!["ZOWIE", "HyperX"]);
        // 声誉 100：解锁至第 7 个（100 = 70 + 6×5）
        assert_eq!(FinanceModel::available_brands(100).len(), 7);
        assert_eq!(FinanceModel::top_brand(100), Some("Red Bull"));
        assert_eq!(FinanceModel::top_brand(69), None);
    }

    #[test]
    fn sponsor_value_linear() {
        assert_eq!(FinanceModel::sponsor_annual_value(70), 560_000);
        assert_eq!(FinanceModel::sponsor_annual_value(100), 800_000);
    }

    #[test]
    fn transfer_fee_and_affordability() {
        assert_eq!(FinanceModel::transfer_fee(85.0), 850_000);
        assert_eq!(FinanceModel::transfer_fee(60.0), 600_000);
        assert!(FinanceModel::can_afford(1_000_000, 400_000, 500_000));
        assert!(!FinanceModel::can_afford(800_000, 400_000, 500_000));
    }

    #[test]
    fn monthly_salary_divides() {
        assert_eq!(FinanceModel::monthly_salary(120_000), 10_000);
        assert_eq!(FinanceModel::monthly_salary(125_000), 10_416);
    }
}
