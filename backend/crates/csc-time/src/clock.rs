//! 模拟时钟 —— 全系统共享的**日期线程**（Kotlin `SimClock` 的转写）。

use serde::{Deserialize, Serialize};

use crate::civil::{civil_from_days, days_from_civil, days_in_month};
use crate::window::TimeWindow;

/// 模拟时钟 —— 所有子系统（赛事排程、VRS 结算、Seed 重算、生涯推进）
/// 串在同一条时间线上。
///
/// 语义与 Kotlin `SimClock`（Java `LocalDate`）完全一致：
/// - 时间基准：UTC 天首（当天 00:00:00 UTC）；
/// - `plusMonths` 的月末截断语义（1-31 进 2 月 → 2-28/29）；
/// - 恢复（`restore_to`）是任意回退，正常推进不经过。
///
/// 内部状态 = 整数三字段 `(year, month, day)`，与 GameState 快照形态一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimClock {
    year: i32,
    month: u32,
    day: u32,
}

impl SimClock {
    /// 指定日期创建（模拟可从此任意起点开始）。
    pub fn of(year: i32, month: u32, day: u32) -> Self {
        let s = Self { year, month, day };
        debug_assert!(s.is_valid(), "非法日期: {year}-{month}-{day}");
        s
    }

    /// 从真实 standings 月份标签定位（如 `2026_05_04` → 2026-05-04），
    /// 作为「真实数据已到此处」的锚点；由 Engine 在 `next_month_start` 时推进到模拟首月。
    ///
    /// 外部输入边界（D6/M13）：坏标签（段数 ≠ 3 / 非数字 / 非法日期）返回 `Err`，
    /// 显式报错而非 panic——调用方（`Engine::load_from_standings`）把它包装为
    /// `SimError::Asset`。
    pub fn from_standings_month(label: &str) -> Result<Self, String> {
        let parts: Vec<&str> = label.split('_').collect();
        if parts.len() != 3 {
            return Err(format!("standings 月份标签格式应为 YYYY_MM_DD: {label}"));
        }
        let (Ok(year), Ok(month), Ok(day)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) else {
            return Err(format!("standings 月份标签含非数字字段: {label}"));
        };
        let clock = Self { year, month, day };
        if !clock.is_valid() {
            return Err(format!("standings 月份标签不是合法日期: {label}"));
        }
        Ok(clock)
    }

    #[inline]
    fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && (1..=days_in_month(self.year, self.month)).contains(&self.day)
    }

    /// 当前模拟日期 `(year, month, day)`。
    pub fn now(&self) -> (i32, u32, u32) {
        (self.year, self.month, self.day)
    }

    /// 当前模拟时间（Unix 秒，UTC 天首）。
    pub fn now_epoch_seconds(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
    }

    /// 月份标签：对齐 standings 文件名（如 `2026_05_04`），供跨月数据对齐。
    pub fn month_label(&self) -> String {
        format!("{:04}_{:02}_{:02}", self.year, self.month, self.day)
    }

    /// 日期标签：对齐赛事记录（如 `2026-06-08`）。
    pub fn date_label(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// 当前年份（荣誉 / 合同按年结算用）。
    pub fn year(&self) -> i32 {
        self.year
    }

    /// 推进 `days` 天（正数向前；负数回退，谨慎使用）。
    pub fn advance_days(&mut self, days: i64) {
        let z = days_from_civil(self.year, self.month, self.day) + days;
        (self.year, self.month, self.day) = civil_from_days(z);
    }

    /// 推进 `months` 个月（日历月；月末溢出自动截断，同 Java `LocalDate.plusMonths`）。
    ///
    /// 实现：先做「年×12+月」整数运算（floor 语义，支持负数），
    /// 再把日截断到新月的最大日（1-31 进 2 月 → 2-28/29）。
    pub fn advance_months(&mut self, months: i64) {
        // floor 除法（Java plusMonths 内部用 floorDiv/floorMod 语义）
        let total = self.year as i64 * 12 + (self.month as i64 - 1) + months;
        let y2 = total.div_euclid(12);
        let m2 = total.rem_euclid(12) + 1;
        let d2 = self.day.min(days_in_month(y2 as i32, m2 as u32));
        self.year = y2 as i32;
        self.month = m2 as u32;
        self.day = d2;
    }

    /// 跳到下一个月的 1 号（月份线程的标准推进：每月从月初开始办赛事）。
    pub fn next_month_start(&mut self) {
        self.advance_months(1);
        self.day = 1;
    }

    /// 恢复到指定日期（存档读档用；任意回退，正常模拟推进不经过这里）。
    pub fn restore_to(&mut self, year: i32, month: u32, day: u32) {
        *self = Self::of(year, month, day);
    }

    /// 滚动时间窗口：`months_back` 个月前的今天 → 今天（闭区间，UTC 天首）。
    ///
    /// 供 VRS 时间衰减与 Seed 重算使用——窗口随模拟日期滚动，旧比赛自然滑出。
    /// 建议在月初固定日调用，避免月末截断（同 Kotlin 注释）。
    pub fn rolling_window(&self, months_back: i64) -> TimeWindow {
        let end = days_from_civil(self.year, self.month, self.day) * 86_400;
        // Java minusMonths：与 plusMonths(-n) 相同（含月末截断）
        let mut start_clock = *self;
        start_clock.advance_months(-months_back);
        let start = days_from_civil(start_clock.year, start_clock.month, start_clock.day) * 86_400;
        TimeWindow::new(start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_seconds_known_values() {
        // Java: LocalDate.of(1970,1,1).atStartOfDay(UTC).toEpochSecond() = 0
        assert_eq!(SimClock::of(1970, 1, 1).now_epoch_seconds(), 0);
        // Java: 2000-01-01 = 946684800
        assert_eq!(SimClock::of(2000, 1, 1).now_epoch_seconds(), 946_684_800);
        // 2026-06-08 = 20612 * 86400 = 1780876800
        assert_eq!(SimClock::of(2026, 6, 8).now_epoch_seconds(), 1_780_876_800);
    }

    #[test]
    fn month_end_truncation_matches_local_date() {
        // Java: LocalDate.of(2026,1,31).plusMonths(1) = 2026-02-28
        let mut c = SimClock::of(2026, 1, 31);
        c.advance_months(1);
        assert_eq!(c.now(), (2026, 2, 28));
        // 闰年 2024-01-31 +1 = 2024-02-29
        let mut c = SimClock::of(2024, 1, 31);
        c.advance_months(1);
        assert_eq!(c.now(), (2024, 2, 29));
        // Java: 2024-02-29.plusMonths(12) = 2025-02-28（闰日进平年截断）
        let mut c = SimClock::of(2024, 2, 29);
        c.advance_months(12);
        assert_eq!(c.now(), (2025, 2, 28));
        // 跨年：2026-11-30 +3 月 = 2027-02-28
        let mut c = SimClock::of(2026, 11, 30);
        c.advance_months(3);
        assert_eq!(c.now(), (2027, 2, 28));
    }

    #[test]
    fn next_month_start_matches_local_date() {
        // Java: LocalDate.of(2026,12,15).plusMonths(1).withDayOfMonth(1) = 2027-01-01
        let mut c = SimClock::of(2026, 12, 15);
        c.next_month_start();
        assert_eq!(c.now(), (2027, 1, 1));
    }

    #[test]
    fn advance_days_known_values() {
        // 2026-01-01 + 31 天 = 2026-02-01；+365 天跨年
        let mut c = SimClock::of(2026, 1, 1);
        c.advance_days(31);
        assert_eq!(c.now(), (2026, 2, 1));
        c.advance_days(365);
        assert_eq!(c.now(), (2027, 2, 1));
        // 负数回退
        c.advance_days(-1);
        assert_eq!(c.now(), (2027, 1, 31));
    }

    #[test]
    fn labels_match_kotlin_formats() {
        let c = SimClock::of(2026, 5, 4);
        assert_eq!(c.month_label(), "2026_05_04");
        assert_eq!(c.date_label(), "2026-05-04");
        assert_eq!(c.year(), 2026);
    }

    #[test]
    fn from_standings_month_parses() {
        assert_eq!(
            SimClock::from_standings_month("2026_05_04").unwrap().now(),
            (2026, 5, 4)
        );
    }

    #[test]
    fn from_standings_month_rejects_bad_labels() {
        // M13：坏资产标签显式报错（D6 外部输入边界），不再 assert/panic。
        assert!(
            SimClock::from_standings_month("2026_05").is_err(),
            "段数 ≠ 3"
        );
        assert!(SimClock::from_standings_month("a_b_c").is_err(), "非数字");
        assert!(
            SimClock::from_standings_month("2026_13_01").is_err(),
            "月份越界"
        );
        assert!(
            SimClock::from_standings_month("2026_02_30").is_err(),
            "日期非法（2 月无 30 日）"
        );
        assert!(
            SimClock::from_standings_month("2026_05_04").is_ok(),
            "合法标签必须可解析"
        );
    }

    #[test]
    fn rolling_window_is_closed_interval() {
        // Kotlin: current.minusMonths(6)..current（闭区间，UTC 天首）
        let c = SimClock::of(2026, 6, 15);
        let w = c.rolling_window(6);
        assert_eq!(w.end(), c.now_epoch_seconds());
        assert_eq!(w.start(), SimClock::of(2025, 12, 15).now_epoch_seconds());
        assert_eq!(w.end() - w.start(), (182 * 86_400)); // 2025-12-15→2026-06-15 = 182 天
    }

    #[test]
    fn restore_to_is_arbitrary() {
        let mut c = SimClock::of(2026, 6, 8);
        c.restore_to(2025, 1, 1);
        assert_eq!(c.now(), (2025, 1, 1));
    }

    #[test]
    fn serde_roundtrip() {
        let c = SimClock::of(2026, 6, 8);
        let json = serde_json::to_string(&c).unwrap();
        let back: SimClock = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
