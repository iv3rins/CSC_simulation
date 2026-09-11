//! 属性取值区间（Kotlin `IntRange` 的转写）。
//!
//! 设计差异：Kotlin `IntRange`（`80..98`）序列化形态取决于平台，且语义隐式；
//! 这里用显式 `lo..=hi` 双端点结构（JSON 友好、可 `Copy`、含端语义明确）。
//! 与 Kotlin 的 `IntRange` 语义完全一致：**闭区间**（两端都包含）。

use serde::{Deserialize, Serialize};

/// 属性取值区间（闭区间：`lo <= value <= hi`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttrRange {
    /// 区间下界（含）。
    pub lo: i32,
    /// 区间上界（含）。
    pub hi: i32,
}

impl AttrRange {
    /// 构造闭区间（`const`，供静态表使用）。
    /// 编译期断言 `lo <= hi`（const panic 在编译期触发——非法区间在构建时即失败）。
    pub const fn new(lo: i32, hi: i32) -> Self {
        assert!(lo <= hi);
        Self { lo, hi }
    }

    /// 是否包含值 `v`（闭区间判定；`lo > hi` 时永不包含——非法区间由调用方保证）。
    #[inline]
    pub fn contains(&self, v: i32) -> bool {
        self.lo <= v && v <= self.hi
    }

    /// 区间长度（元素个数；`hi - lo + 1`，供随机生成/概率计算用）。
    #[inline]
    pub fn len(&self) -> i32 {
        self.hi - self.lo + 1
    }

    /// 区间是否为空（`lo > hi`）。
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lo > self.hi
    }

    /// 中心点（`(lo + hi) / 2`，供档位对比/统计断言用）。
    #[inline]
    pub fn mid(&self) -> i32 {
        (self.lo + self.hi) / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_interval_semantics() {
        let r = AttrRange::new(80, 98);
        assert!(r.contains(80));
        assert!(r.contains(98));
        assert!(r.contains(89));
        assert!(!r.contains(79));
        assert!(!r.contains(99));
        assert_eq!(r.len(), 19);
        assert!(!r.is_empty());
    }

    #[test]
    fn single_point_interval() {
        let r = AttrRange::new(5, 5);
        assert!(r.contains(5));
        assert_eq!(r.len(), 1);
        assert_eq!(r.mid(), 5);
    }

    #[test]
    fn serde_roundtrip() {
        let r = AttrRange::new(80, 98);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"lo":80,"hi":98}"#);
        let back: AttrRange = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
