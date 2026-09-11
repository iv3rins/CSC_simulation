//! 时间窗口（时间系统的值对象；Kotlin `TimeWindow` 的转写）。

use serde::{Deserialize, Serialize};

/// 时间窗口（官方 RankingContext.setTimeWindow）。
///
/// 官方用窗口把时间映射到 `[0,1]`：窗口起点 = 0（最旧）、终点 = 1（最新），
/// `timeDecayFactor = 1` 时为线性衰减——越久远的结果权重越低。
/// 模拟中窗口 = 最近 6 个月，由 [`crate::clock::SimClock::rolling_window`] 随模拟日期滚动生成。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    start: i64,
    end: i64,
}

impl TimeWindow {
    /// 构造窗口（`end >= start` 契约，与 Kotlin `init require` 等价）。
    pub fn new(start: i64, end: i64) -> Self {
        assert!(
            end >= start,
            "TimeWindow.end 必须 >= start（{start} > {end}）"
        );
        Self { start, end }
    }

    /// 窗口起点（epoch 秒）。
    pub fn start(&self) -> i64 {
        self.start
    }

    /// 窗口终点（epoch 秒）。
    pub fn end(&self) -> i64 {
        self.end
    }

    /// 时间衰减因子：窗口内线性映射到 `[0,1]`，窗口外 clamp 为 0 / 1
    /// （= Kotlin `windowMod`）。
    pub fn window_mod(&self, timestamp: i64) -> f64 {
        remap_value_clamped(
            timestamp as f64,
            self.start as f64,
            self.end as f64,
            0.0,
            1.0,
        )
    }
}

/// 线性重映射并 clamp；输入区间退化（`in_start == in_end`）时取中点
/// （= Kotlin `TimeWindow.remapValueClamped` 静态方法）。
pub fn remap_value_clamped(
    value: f64,
    in_start: f64,
    in_end: f64,
    out_start: f64,
    out_end: f64,
) -> f64 {
    let interp = if in_start == in_end {
        0.5
    } else {
        (value - in_start) / (in_end - in_start)
    };
    let clamped = interp.clamp(0.0, 1.0);
    out_start + (out_end - out_start) * clamped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_mod_boundaries_and_midpoint() {
        // Kotlin 对照：windowMod(start)=0、windowMod(end)=1、中点=0.5
        let w = TimeWindow::new(1_000, 2_000);
        assert_eq!(w.window_mod(1_000), 0.0);
        assert_eq!(w.window_mod(2_000), 1.0);
        assert_eq!(w.window_mod(1_500), 0.5);
    }

    #[test]
    fn window_mod_clamps_outside() {
        let w = TimeWindow::new(1_000, 2_000);
        assert_eq!(w.window_mod(999), 0.0); // 窗口外（旧）→ 0
        assert_eq!(w.window_mod(2_001), 1.0); // 窗口外（新）→ 1
        assert_eq!(w.window_mod(-5_000), 0.0);
    }

    #[test]
    fn degenerate_window_takes_midpoint() {
        // Kotlin：inStart == inEnd → interp = 0.5 → 输出 0.5
        let w = TimeWindow::new(500, 500);
        assert_eq!(w.window_mod(500), 0.5);
    }

    #[test]
    fn remap_custom_output_range() {
        // Kotlin 静态方法可换输出区间
        assert_eq!(remap_value_clamped(5.0, 0.0, 10.0, 100.0, 200.0), 150.0);
        assert_eq!(remap_value_clamped(-1.0, 0.0, 10.0, 100.0, 200.0), 100.0); // clamp 下界
        assert_eq!(remap_value_clamped(11.0, 0.0, 10.0, 100.0, 200.0), 200.0); // clamp 上界
    }

    #[test]
    #[should_panic(expected = "end 必须 >= start")]
    fn invalid_window_panics_like_require() {
        let _ = TimeWindow::new(2_000, 1_000);
    }

    #[test]
    fn serde_roundtrip() {
        let w = TimeWindow::new(1_000, 2_000);
        let json = serde_json::to_string(&w).unwrap();
        let back: TimeWindow = serde_json::from_str(&json).unwrap();
        assert_eq!(w, back);
    }
}
