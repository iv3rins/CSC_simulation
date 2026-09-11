//! 公历日期算术（Howard Hinnant `days_from_civil` / `civil_from_days`）。
//!
//! 与 Java `LocalDate` 的语义完全一致：proleptic Gregorian 历法，
//! 支持负数年份（`0` = 公元前 1 年），`days_from_civil` 返回自
//! 1970-01-01（epoch）起的天数。

/// 是否为闰年（格里高利规则）。
#[inline]
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 某年某月的天数（2 月按闰年判定）。
#[inline]
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => panic!("非法月份: {month}"),
    }
}

/// 取 `"YYYY-MM-DD"` 所在月的最后一天（返回 `"YYYY-MM-<月末>"`）。
///
/// 解析失败回退为日期原串（调用方仅作展示提示，不影响确定性）。
/// **唯一事实源**——服务端 `transfer_market_from_pending` 与历史
/// `csc-server/game/mod.rs::month_end` 曾各自实现一份，提取到本 crate 后共享。
#[must_use]
pub fn month_end_label(date: &str) -> String {
    let mut parts = date.split('-');
    let (Some(y), Some(m)) = (parts.next(), parts.next()) else {
        return date.to_string();
    };
    let (Ok(year), Ok(mon)) = (y.parse::<i32>(), m.parse::<u32>()) else {
        return date.to_string();
    };
    if !(1..=12).contains(&mon) {
        return date.to_string();
    }
    let last = days_in_month(year, mon);
    format!("{y}-{m}-{last:02}")
}

/// 公历日期 → epoch 起的天数（1970-01-01 = 0）。
///
/// Howard Hinnant 算法：`days_from_civil(y, m, d)`。
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    debug_assert!((1..=12).contains(&month), "非法月份: {month}");
    debug_assert!(
        (1..=days_in_month(year, month)).contains(&day),
        "非法日期: {year}-{month}-{day}"
    );

    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i64 + 9) % 12; // [0, 11] March=0
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// epoch 起的天数 → 公历日期（`days_from_civil` 的逆）。
///
/// Howard Hinnant 算法：`civil_from_days(z)`。返回 `(year, month, day)`。
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = y + if m <= 2 { 1 } else { 0 };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_known_values() {
        // Java LocalDate 对照：1970-01-01 = 0
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // 2000-01-01 = 10957（Java: LocalDate.of(2000,1,1).toEpochDay()）
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
        // 2026-06-08（模拟常用日期）
        assert_eq!(days_from_civil(2026, 6, 8), 20612);
    }

    #[test]
    fn roundtrip_over_broad_range() {
        // 1970..2100 全量往返
        for year in 1970..=2100 {
            for month in 1..=12u32 {
                for day in 1..=days_in_month(year, month) {
                    let z = days_from_civil(year, month, day);
                    assert_eq!(civil_from_days(z), (year, month, day));
                }
            }
        }
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(2026, 2), 28);
    }

    #[test]
    fn negative_years_ok() {
        // 公元前（year=0 表示公元前 1 年）：Java LocalDate 同样支持
        let z = days_from_civil(0, 1, 1);
        assert_eq!(civil_from_days(z), (0, 1, 1));
    }

    #[test]
    fn month_end_label_computes_last_day_of_month() {
        // 常规月 → 月末日。
        assert_eq!(month_end_label("2026-06-08"), "2026-06-30");
        // 大月 / 闰年 2 月 / 平年 2 月。
        assert_eq!(month_end_label("2026-01-15"), "2026-01-31");
        assert_eq!(month_end_label("2024-02-01"), "2024-02-29", "2024 闰年");
        assert_eq!(month_end_label("2025-02-01"), "2025-02-28", "2025 平年");
        // 非法输入 → 原样回退（不 panic，调用方仅作展示）。
        assert_eq!(month_end_label("not-a-date"), "not-a-date");
        assert_eq!(month_end_label("2026-13-01"), "2026-13-01", "非法月份回退");
    }
}
