//! 真实 HLTV TOP20 历史榜单（外置资产 `assets/top20-data/*.json` 的解析与查询）。
//!
//! **定位**：真实世界的「传奇榜」基准——2023/2024/2025 三届官方 HLTV TOP20
//! 选手及其真实名次。玩家在其**模拟世界**里的 TOP20 排名与这些真实选手
//! 同「精神」竞争（展示层同屏对照 + 评语引入真实传奇参照系），而非数值上
//! 直接混入模拟榜单（模拟世界的样本/荣誉口径与真实 HLTV 不可直接换算）。
//!
//! 数据形态（`hltv_top20_{year}.json`）：
//! ```json
//! [ { "placement": 1, "nickname": "ZywOo", "name": "Mathieu Herbaut", "country": "France" } ]
//! ```
//!
//! - `placement`：官方名次（1..=20，2025 届仅 19 人因历史因素）；
//! - `nickname`：赛场 ID（与 standings 阵容里的名字对齐，可跨表关联）；
//! - `name` / `country`：展示信息。
//!
//! 纯值、无状态、无随机、零 csc-crate 依赖（仅 serde）——解析失败显式
//! `Result<_, String>`（坏资产不静默；错误字符串化由调用方决定包装方式）。

use serde::{Deserialize, Serialize};

/// 真实 TOP20 选手（一届榜单中的一名选手）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealTop20Player {
    /// 官方名次（1 起）
    pub placement: i32,
    /// 赛场 ID（如 "ZywOo"、"donk"）
    pub nickname: String,
    /// 真名（如 "Mathieu Herbaut"）
    pub name: String,
    /// 国籍（如 "France"）
    pub country: String,
}

/// 一届真实 TOP20 榜单（某一年度的完整名次）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealTop20Year {
    /// 归属年份（如 2025）
    pub year: i32,
    /// 榜单（按 `placement` 升序）
    pub players: Vec<RealTop20Player>,
}

impl RealTop20Year {
    /// 从 JSON 文本解析一届榜单（`hltv_top20_{year}.json` 的原始内容）。
    ///
    /// @param year 归属年份（文件名决定，调用方传入以保证元数据一致）
    pub fn from_json_str(year: i32, json: &str) -> Result<Self, String> {
        let mut players: Vec<RealTop20Player> = serde_json::from_str(json)
            .map_err(|e| format!("真实 TOP20 {year} 资产解析失败：{e}"))?;
        // 候选内序不稳定（人工维护的 JSON），按 placement 升序稳定化
        players.sort_by_key(|p| p.placement);
        Ok(Self { year, players })
    }

    /// 榜单人数。
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// 榜单是否为空。
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// 指定名次的选手（1 起；越界 = None）。
    pub fn at(&self, placement: i32) -> Option<&RealTop20Player> {
        self.players.iter().find(|p| p.placement == placement)
    }
}

/// 真实 TOP20 三年索引（2023/2024/2025），按年份查询。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealTop20Index {
    /// 年份 → 榜单（保持年份升序）
    years: Vec<RealTop20Year>,
}

impl RealTop20Index {
    /// 新增一届（保持年份升序）。
    pub fn push(&mut self, year: RealTop20Year) {
        self.years.push(year);
        self.years.sort_by_key(|y| y.year);
    }

    /// 全部年份（升序）。
    pub fn years(&self) -> &[RealTop20Year] {
        &self.years
    }

    /// 指定年份的榜单。
    pub fn at(&self, year: i32) -> Option<&RealTop20Year> {
        self.years.iter().find(|y| y.year == year)
    }

    /// 某选手在全部年份中的**最佳**名次（1 起；无上榜 = None）。
    pub fn best_placement_of(&self, nickname: &str) -> Option<i32> {
        self.years
            .iter()
            .flat_map(|y| y.players.iter())
            .filter(|p| p.nickname.eq_ignore_ascii_case(nickname))
            .map(|p| p.placement)
            .min()
    }

    /// 某选手上榜的总届数。
    pub fn appearances_of(&self, nickname: &str) -> usize {
        self.years
            .iter()
            .filter(|y| {
                y.players
                    .iter()
                    .any(|p| p.nickname.eq_ignore_ascii_case(nickname))
            })
            .count()
    }

    /// 是否有任何数据。
    pub fn is_empty(&self) -> bool {
        self.years.is_empty()
    }

    // —— 轨迹统计（供"用真实名次变动拟合虚拟成长/衰减模型"的校准器消费）——

    /// 全部上榜选手的（nickname, 出现年份列表按升序）。（年份列表用于算连续在榜 / 年际变动。）
    pub fn players_with_years(&self) -> Vec<(String, Vec<i32>)> {
        let mut map: std::collections::BTreeMap<String, Vec<i32>> =
            std::collections::BTreeMap::new();
        for y in &self.years {
            for p in &y.players {
                map.entry(p.nickname.clone()).or_default().push(y.year);
            }
        }
        map.into_iter().collect()
    }

    /// 上榜选手总数（去重）。
    pub fn distinct_players(&self) -> usize {
        self.players_with_years().len()
    }

    /// 连续在榜 N 届的选手数（按"恰好 N 届在榜"分组）。
    pub fn appearance_histogram(&self) -> std::collections::BTreeMap<usize, usize> {
        let mut hist: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
        for (_, years) in self.players_with_years() {
            *hist.entry(years.len()).or_insert(0) += 1;
        }
        hist
    }

    /// 连续在榜选手的年际名次变动（绝对值的全部样本）。
    /// 返回形如 `[(year_from, year_to, abs_delta), ...]` 的列表。
    pub fn year_over_year_movements(&self) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        for (nick, years) in self.players_with_years() {
            for w in years.windows(2) {
                let (a, b) = (w[0], w[1]);
                let pl_a = self
                    .at(a)
                    .and_then(|y| y.players.iter().find(|p| p.nickname == nick))
                    .map(|p| p.placement);
                let pl_b = self
                    .at(b)
                    .and_then(|y| y.players.iter().find(|p| p.nickname == nick))
                    .map(|p| p.placement);
                if let (Some(x), Some(yy)) = (pl_a, pl_b) {
                    out.push((a, b, (x - yy).abs()));
                }
            }
        }
        out
    }

    /// 新面孔年轮换率（某年的"新上榜人数" / 该年榜单人数）。
    /// 返回 `(year, new_count, total, ratio)` 列表（按年份升序；跳过首年——无前史可比）。
    pub fn newcomer_rates(&self) -> Vec<(i32, usize, usize, f64)> {
        let mut out = Vec::new();
        for (i, y) in self.years.iter().enumerate() {
            if i == 0 {
                continue;
            }
            let prev = &self.years[i - 1];
            let prev_nicks: std::collections::HashSet<&str> =
                prev.players.iter().map(|p| p.nickname.as_str()).collect();
            let total = y.players.len();
            let new_count = y
                .players
                .iter()
                .filter(|p| !prev_nicks.contains(p.nickname.as_str()))
                .count();
            let ratio = if total > 0 {
                new_count as f64 / total as f64
            } else {
                0.0
            };
            out.push((y.year, new_count, total, ratio));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
        {"placement": 2, "nickname": "donk", "name": "Danil Kryshkovets", "country": "Russia"},
        {"placement": 1, "nickname": "ZywOo", "name": "Mathieu Herbaut", "country": "France"}
    ]"#;

    #[test]
    fn parse_and_sort_by_placement() {
        let y = RealTop20Year::from_json_str(2025, SAMPLE).expect("合法资产必成功");
        assert_eq!(y.len(), 2);
        assert_eq!(y.players[0].nickname, "ZywOo", "按 placement 升序稳定化");
        assert_eq!(y.players[1].nickname, "donk");
        assert_eq!(y.at(1).unwrap().nickname, "ZywOo");
        assert_eq!(y.at(2).unwrap().country, "Russia");
        assert!(y.at(3).is_none());
    }

    #[test]
    fn malformed_json_rejected() {
        assert!(RealTop20Year::from_json_str(2025, "{ oops").is_err());
    }

    #[test]
    fn index_queries() {
        let mut idx = RealTop20Index::default();
        idx.push(RealTop20Year::from_json_str(2025, SAMPLE).unwrap());
        idx.push(RealTop20Year::from_json_str(2024, SAMPLE).unwrap());
        assert_eq!(idx.years().len(), 2);
        assert_eq!(idx.at(2025).unwrap().len(), 2);
        assert!(idx.at(2026).is_none());
        assert_eq!(idx.best_placement_of("ZywOo"), Some(1));
        assert_eq!(idx.appearances_of("donk"), 2);
        assert_eq!(idx.appearances_of("nobody"), 0);
        assert!(!idx.is_empty());
    }

    const Y23: &str = r#"[{"placement":1,"nickname":"ZywOo","name":"a","country":"FR"},{"placement":2,"nickname":"NiKo","name":"b","country":"BA"}]"#;
    const Y24: &str = r#"[{"placement":1,"nickname":"donk","name":"c","country":"RU"},{"placement":2,"nickname":"ZywOo","name":"a","country":"FR"}]"#;
    const Y25: &str = r#"[{"placement":1,"nickname":"ZywOo","name":"a","country":"FR"},{"placement":2,"nickname":"donk","name":"c","country":"RU"}]"#;

    fn three_year_index() -> RealTop20Index {
        let mut idx = RealTop20Index::default();
        idx.push(RealTop20Year::from_json_str(2023, Y23).unwrap());
        idx.push(RealTop20Year::from_json_str(2024, Y24).unwrap());
        idx.push(RealTop20Year::from_json_str(2025, Y25).unwrap());
        idx
    }

    #[test]
    fn trajectory_appearance_histogram() {
        let idx = three_year_index();
        // ZywOo 3 届、NiKo 1 届、donk 2 届
        let hist = idx.appearance_histogram();
        assert_eq!(hist.get(&3), Some(&1), "连续 3 届 1 人");
        assert_eq!(hist.get(&2), Some(&1), "连续 2 届 1 人");
        assert_eq!(hist.get(&1), Some(&1), "1 届 1 人");
        assert_eq!(idx.distinct_players(), 3);
    }

    #[test]
    fn trajectory_movements_and_newcomers() {
        let idx = three_year_index();
        // ZywOo: 2023 #1 → 2024 #2 → 2025 #1: 变动 1 + 1；donk: 2024 #1 → 2025 #2: 1
        let moves = idx.year_over_year_movements();
        assert_eq!(moves.len(), 3);
        assert!(moves.iter().all(|(_, _, d)| *d == 1));
        // 新面孔：2024 新增 donk（1/2）；2025 新增 0（donk/ZywOo 都在）
        let rates = idx.newcomer_rates();
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0], (2024, 1, 2, 0.5));
        assert_eq!(rates[1], (2025, 0, 2, 0.0));
    }
}
