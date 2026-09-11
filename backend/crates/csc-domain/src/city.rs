//! 赛事举办城市 + 区域（Kotlin `City.kt`）。

use serde::{Deserialize, Serialize};

/// 区域（VRS 全球积分按区域划分直邀名额）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Region {
    /// 欧洲
    Europe,
    /// 北美
    NorthAmerica,
    /// 南美
    SouthAmerica,
    /// 亚洲
    Asia,
    /// 大洋洲
    Oceania,
    /// 其他 / 未分类
    Other,
}

/// 赛事举办城市。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct City {
    /// 城市名称，如「上海」「卡托维兹」。
    pub name: String,
    /// 国家，如「中国」「波兰」。
    pub country: String,
    /// 所属区域（用于 VRS 区域积分体系）。
    pub region: Region,
}

impl City {
    /// 构造城市。
    pub fn new(name: impl Into<String>, country: impl Into<String>, region: Region) -> Self {
        Self {
            name: name.into(),
            country: country.into(),
            region,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_and_copy() {
        let c = City::new("上海", "中国", Region::Asia);
        let c2 = c.clone(); // Clone；City 非 Copy（含 String）
        assert_eq!(c.name, c2.name);
        assert_eq!(c.region, Region::Asia);
        assert_ne!(c.region, Region::Europe);
    }

    #[test]
    fn region_serde_names_match_kotlin() {
        assert_eq!(
            serde_json::to_string(&Region::NorthAmerica).unwrap(),
            r#""NORTH_AMERICA""#
        );
        assert_eq!(
            serde_json::to_string(&Region::SouthAmerica).unwrap(),
            r#""SOUTH_AMERICA""#
        );
        assert_eq!(serde_json::to_string(&Region::Other).unwrap(), r#""OTHER""#);
    }
}
