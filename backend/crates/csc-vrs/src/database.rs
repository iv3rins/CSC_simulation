//! VRS 排名内存数据库（Kotlin `VrsDatabase.kt` 转写）。
//!
//! 转写差异：
//! - **零 IO**：`load(File)` → `from_json_files(&[(file_name, content)])`（解析用 serde）；
//! - **matchHistory 窗口剪枝**（docs/09 B2）：`reseed` 时按窗口裁剪旧记录，
//!   20 年生涯内存不再无界增长；
//! - 未知签名告警 → `eprintln!`（不 fail-fast，部分库降级语义保留）。

use std::collections::HashMap;

use csc_domain::match_result::MatchResult;
use csc_domain::tourney_tier::TourneyTier;
use csc_time::window::TimeWindow;
use csc_util::SimError;
use serde::Deserialize;

use crate::entry::VrsEntry;
use crate::modifiers::{MatchRecord, VrsModifiers};
use crate::scoring::VrsScoring;

/// 队伍当前模拟状态（= Kotlin `VrsDatabase.VrsTeamState`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VrsTeamState {
    /// 队伍名称
    pub team_name: String,
    /// 当前 VRS 动态 ELO 分
    pub points: i32,
    /// 当前排名（按积分排序后刷新）
    pub ranking: i32,
    /// 当前阵容（选手名）
    pub roster: Vec<String>,
    /// 上次结算时的阵容（用于 3 人离队判定）
    pub last_settled_roster: Vec<String>,
    /// VRS 静态 Seed 分（官方两层模型的第 1 层；由 `reseed` 重算）
    pub seed_points: i32,
}

impl VrsTeamState {
    /// 距离上次结算，出走的选手数。
    pub fn departed_count(&self) -> usize {
        let settled: std::collections::HashSet<&String> = self.last_settled_roster.iter().collect();
        self.roster.iter().filter(|p| !settled.contains(p)).count()
    }
}

/// VRS 排名内存数据库（模拟推进用）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VrsDatabase {
    /// 按时间排序的月份标识（如 "2026_01_05"）
    pub months: Vec<String>,
    /// month → 当月全部排名条目（按排名升序）——初始基准，只读
    by_month: HashMap<String, Vec<VrsEntry>>,
    /// signature → (month → ranking)，跨月历史——初始基准，只读
    history: HashMap<String, HashMap<String, i32>>,
    /// signature → 当前模拟状态（可变，随比赛推进更新）
    teams: HashMap<String, VrsTeamState>,
    /// 模拟推进中产生的比赛历史（供 Seed 重算；**reseed 时按窗口剪枝**）
    match_history: Vec<MatchRecord>,
}

impl VrsDatabase {
    /// 测试/调试用：直接覆写一支队伍的当前状态（排名/积分/阵容）。
    pub fn upsert_state(&mut self, signature: String, state: VrsTeamState) {
        self.teams.insert(signature, state);
    }

    /// 队伍阵容签名（跨月稳定、同名不同队可区分；= Kotlin `signatureOf`）。
    pub fn signature_of(team_name: &str, roster: &[String]) -> String {
        let mut names = roster.to_vec();
        names.sort();
        csc_util::signatures::team_signature(team_name, &names)
    }

    /// 由排名条目求阵容签名。
    pub fn signature_of_entry(entry: &VrsEntry) -> String {
        Self::signature_of(&entry.team_name, &entry.roster)
    }

    // —— 初始基准加载 ——

    /// 从 (文件名, 内容) 列表加载全部 standings JSON 作为初始基准
    /// （= Kotlin `load(files)`；文件读取由调用方负责——零 IO）。
    ///
    /// **外部输入边界（D6/D2）**：解析失败 / 语义校验失败（空队名、负积分、
    /// 空阵容、非法排名）返回 [`SimError::Asset`]——坏资产显式报错，
    /// 不再静默丢弃（Kotlin 时代的宽容解析是格式漂移静默化的历史债）。
    pub fn from_json_files(files: &[(String, String)]) -> Result<Self, SimError> {
        let mut by_month: HashMap<String, Vec<VrsEntry>> = HashMap::new();
        for (file_name, content) in files {
            by_month.insert(
                Self::month_of(file_name),
                Self::parse_json_checked(content, file_name)?,
            );
        }
        let mut history: HashMap<String, HashMap<String, i32>> = HashMap::new();
        for (month, entries) in &by_month {
            for e in entries {
                history
                    .entry(Self::signature_of_entry(e))
                    .or_default()
                    .insert(month.clone(), e.ranking);
            }
        }

        let months: Vec<String> = {
            let mut v: Vec<String> = by_month.keys().cloned().collect();
            v.sort();
            v
        };

        // 用最新一个月的数据初始化「当前模拟状态」（空输入 → 空状态）
        let Some(latest_month) = months.last() else {
            return Ok(Self {
                months,
                by_month,
                history,
                teams: HashMap::new(),
                match_history: Vec::new(),
            });
        };
        let latest = by_month.get(latest_month).cloned().unwrap_or_default();
        let mut teams: HashMap<String, VrsTeamState> = HashMap::new();
        let mut sorted = latest;
        sorted.sort_by_key(|e| std::cmp::Reverse(e.points));
        for (idx, e) in sorted.into_iter().enumerate() {
            teams.insert(
                Self::signature_of_entry(&e),
                VrsTeamState {
                    team_name: e.team_name,
                    points: e.points,
                    ranking: idx as i32 + 1,
                    roster: e.roster.clone(),
                    last_settled_roster: e.roster,
                    seed_points: 0,
                },
            );
        }

        Ok(Self {
            months,
            by_month,
            history,
            teams,
            match_history: Vec::new(),
        })
    }

    /// 从文件名提取月份标识："standings_global_2026_05_04.json" → "2026_05_04"。
    pub fn month_of(file_name: &str) -> String {
        file_name
            .strip_prefix("standings_global_")
            .and_then(|s| s.strip_suffix(".json"))
            .unwrap_or(file_name)
            .to_string()
    }

    /// 解析并**校验** standings JSON（外部输入边界）。
    ///
    /// 结构：serde 结构解析（结构错误 = 资产损坏，直接报错——不再静默空表）；
    /// 语义：空队名 / 负积分 / 空阵容 / 非正排名 = 资产损坏（这些会破坏
    /// 排程/映射不变量）。**允许**同名不同队（青训队与主队共用 org 名，
    /// 真实数据存在，按阵容签名区分）与重复排名（历史数据无，但排序语义
    /// 容忍重排）。
    pub fn parse_json_checked(json: &str, file_name: &str) -> Result<Vec<VrsEntry>, SimError> {
        #[derive(Deserialize)]
        struct StandingsFile {
            #[serde(default)]
            rankings: Vec<RawEntry>,
        }
        #[derive(Deserialize)]
        struct RawEntry {
            ranking: i32,
            #[serde(default)]
            points: i32,
            #[serde(rename = "teamName")]
            team_name: String,
            #[serde(default)]
            roster: Vec<String>,
        }
        let parsed: StandingsFile = serde_json::from_str(json)
            .map_err(|e| SimError::asset(file_name, format!("JSON 解析失败：{e}")))?;
        for (i, e) in parsed.rankings.iter().enumerate() {
            let idx = format!("第 {i} 条");
            if e.team_name.trim().is_empty() {
                return Err(SimError::asset(file_name, format!("{idx}：队名为空")));
            }
            if e.points < 0 {
                return Err(SimError::asset(
                    file_name,
                    format!("{idx}（{}）：积分为负（{}）", e.team_name, e.points),
                ));
            }
            if e.roster.is_empty() {
                return Err(SimError::asset(
                    file_name,
                    format!("{idx}（{}）：阵容为空（映射将无法生成选手）", e.team_name),
                ));
            }
            if e.ranking < 1 {
                return Err(SimError::asset(
                    file_name,
                    format!("{idx}（{}）：排名非法（{}）", e.team_name, e.ranking),
                ));
            }
        }
        Ok(parsed
            .rankings
            .into_iter()
            .map(|e| VrsEntry::new(e.ranking, e.points, e.team_name, e.roster))
            .collect())
    }

    /// 解析 standings JSON（**宽容语义**，仅测试/调试用：解析失败返回空表、
    /// 缺字段走 serde 默认值；= Kotlin `parseJson` 历史语义）。
    ///
    /// 生产路径一律走 [`Self::parse_json_checked`]（外部输入必须显式报错）。
    pub fn parse_json(json: &str) -> Vec<VrsEntry> {
        #[derive(Deserialize)]
        struct StandingsFile {
            #[serde(default)]
            rankings: Vec<RawEntry>,
        }
        #[derive(Deserialize)]
        struct RawEntry {
            ranking: i32,
            #[serde(default)]
            points: i32,
            #[serde(rename = "teamName")]
            team_name: String,
            #[serde(default)]
            roster: Vec<String>,
        }
        let parsed: Result<StandingsFile, _> = serde_json::from_str(json);
        match parsed {
            Ok(f) => f
                .rankings
                .into_iter()
                .map(|e| VrsEntry::new(e.ranking, e.points, e.team_name, e.roster))
                .collect(),
            Err(_) => Vec::new(), // 与 Kotlin 正则解析的宽容语义对齐：解析失败 → 空表
        }
    }

    // —— 初始基准查询 ——

    /// 某月全部排名条目（按排名升序）——初始基准。
    pub fn entries_of(&self, month: &str) -> Vec<VrsEntry> {
        self.by_month.get(month).cloned().unwrap_or_default()
    }

    /// 队伍在指定月份的历史排名；查不到返回 None——初始基准。
    pub fn ranking_of(&self, entry: &VrsEntry, month: &str) -> Option<i32> {
        self.history
            .get(&Self::signature_of_entry(entry))?
            .get(month)
            .copied()
    }

    /// 该队伍在指定月份及之前 count 个月里，是否都进入前 topN——初始基准判定。
    pub fn consecutive_top_n(
        &self,
        entry: &VrsEntry,
        month: &str,
        top_n: i32,
        count: usize,
    ) -> bool {
        let Some(idx) = self.months.iter().position(|m| m == month) else {
            return false;
        };
        if idx < count - 1 {
            return false;
        }
        let sig = Self::signature_of_entry(entry);
        for m in &self.months[idx + 1 - count..=idx] {
            let Some(rank) = self.history.get(&sig).and_then(|h| h.get(m)) else {
                return false;
            };
            if *rank > top_n {
                return false;
            }
        }
        true
    }

    // —— 模拟推进 API ——

    /// 当前参与模拟的队伍状态列表（按积分降序，即最新排名）。
    ///
    /// **确定性（2026 修订）**：同分队伍按签名升序 tiebreak——此前同分时顺序
    /// 由 `HashMap` 迭代序（RandomState 按进程随机）决定，邀请名单/排程在
    /// 跨进程运行时不同，破坏「seed + 决策日志 = 一致世界」。
    pub fn all_teams(&self) -> Vec<&VrsTeamState> {
        let mut v: Vec<(&String, &VrsTeamState)> = self.teams.iter().collect();
        v.sort_by(|(ka, a), (kb, b)| b.points.cmp(&a.points).then_with(|| ka.cmp(kb)));
        v.into_iter().map(|(_, t)| t).collect()
    }

    /// 按签名取队伍状态；不存在返回 None。
    pub fn team_of(&self, signature: &str) -> Option<&VrsTeamState> {
        self.teams.get(signature)
    }

    /// 队伍当前的 VRS 静态 Seed 分；未 reseed 过或查不到时返回 None。
    pub fn seed_points_of(&self, signature: &str) -> Option<i32> {
        self.teams.get(signature).map(|t| t.seed_points)
    }

    /// 比赛历史条数（观察/测试用；= Kotlin matchHistory.size）。
    pub fn match_history_len(&self) -> usize {
        self.match_history.len()
    }

    /// 为赛事中首次出现的阵容注册运行时 VRS 状态。
    /// 扩展世界中的队伍没有初始 standings 历史，但仍必须参与积分计算。
    fn ensure_runtime_team(&mut self, signature: &str) {
        if self.teams.contains_key(signature) {
            return;
        }
        let (team_name, roster) = signature
            .split_once('|')
            .map(|(name, roster)| {
                (
                    name.to_string(),
                    roster
                        .split(',')
                        .map(str::to_string)
                        .filter(|p| !p.is_empty())
                        .collect(),
                )
            })
            .unwrap_or_else(|| (signature.to_string(), Vec::new()));
        self.teams.insert(
            signature.to_string(),
            VrsTeamState {
                team_name,
                points: 0,
                ranking: self.teams.len() as i32 + 1,
                roster: roster.clone(),
                last_settled_roster: roster,
                seed_points: 0,
            },
        );
    }

    /// 结算一场比赛（按签名；= Kotlin `applyMatch(winnerSig, loserSig, ...)`）。
    /// 未进入初始 standings 的队伍按 0 分注册后正常结算。
    pub fn apply_match(
        &mut self,
        winner_sig: &str,
        loser_sig: &str,
        tier: TourneyTier,
        timestamp: Option<i64>,
        window: Option<&TimeWindow>,
    ) -> bool {
        self.ensure_runtime_team(winner_sig);
        self.ensure_runtime_team(loser_sig);
        // 同时可变借出双方：从 map 取出修改后再插回（HashMap 不允许双可变借用）
        let Some(mut winner) = self.teams.remove(winner_sig) else {
            Self::warn_unknown_signature(winner_sig, "applyMatch(winner)");
            return false;
        };
        let Some(mut loser) = self.teams.remove(loser_sig) else {
            self.teams.insert(winner_sig.to_string(), winner); // 恢复
            Self::warn_unknown_signature(loser_sig, "applyMatch(loser)");
            return false;
        };
        let prize_pool = csc_domain::tier_profile::tier_prize_pool(tier);
        let lan = csc_domain::tier_profile::tier_lan(tier);
        VrsScoring::apply_match(&mut winner, &mut loser, tier, timestamp, window);
        let ts = timestamp.unwrap_or_else(|| window.map_or(0, |w| w.end()));
        self.match_history.push(MatchRecord {
            winner_sig: winner_sig.to_string(),
            loser_sig: loser_sig.to_string(),
            timestamp: ts,
            prize_pool,
            lan,
        });
        self.teams.insert(winner_sig.to_string(), winner);
        self.teams.insert(loser_sig.to_string(), loser);
        self.refresh_rankings();
        true
    }

    /// 按 MatchResult 结算（保留 venue 信息；= Kotlin `applyMatch(result, ...)`）。
    pub fn apply_match_result(
        &mut self,
        result: &MatchResult,
        timestamp: Option<i64>,
        window: Option<&TimeWindow>,
    ) -> bool {
        self.ensure_runtime_team(&result.winner_sig);
        self.ensure_runtime_team(&result.loser_sig);
        let Some(mut winner) = self.teams.remove(&result.winner_sig) else {
            Self::warn_unknown_signature(&result.winner_sig, "applyMatch(winner)");
            return false;
        };
        let Some(mut loser) = self.teams.remove(&result.loser_sig) else {
            self.teams.insert(result.winner_sig.clone(), winner);
            Self::warn_unknown_signature(&result.loser_sig, "applyMatch(loser)");
            return false;
        };
        let prize_pool = csc_domain::tier_profile::tier_prize_pool(result.tier);
        let lan = csc_domain::match_result::venue_lan(result.venue);
        VrsScoring::apply_match(&mut winner, &mut loser, result.tier, timestamp, window);
        let ts = timestamp.unwrap_or_else(|| window.map_or(0, |w| w.end()));
        self.match_history.push(MatchRecord {
            winner_sig: result.winner_sig.clone(),
            loser_sig: result.loser_sig.clone(),
            timestamp: ts,
            prize_pool,
            lan,
        });
        self.teams.insert(result.winner_sig.clone(), winner);
        self.teams.insert(result.loser_sig.clone(), loser);
        self.refresh_rankings();
        true
    }

    /// 基于比赛历史重算全部队伍的 VRS 静态 Seed 分（= Kotlin `reseed`；
    /// **同时按窗口剪枝比赛历史**——docs/09 B2，旧比赛随窗口滚出内存）。
    pub fn reseed(&mut self, window: &TimeWindow) {
        // 剪枝：窗口起点之前的记录对当前/未来窗口的衰减恒为 0，删除（内存有界）
        self.match_history.retain(|m| m.timestamp >= window.start());
        let seeds = VrsModifiers::compute_seed_values(&self.match_history, window);
        let elo = VrsModifiers::seed_to_elo(&seeds);
        for (sig, seed_elo) in elo {
            if let Some(t) = self.teams.get_mut(&sig) {
                t.seed_points = seed_elo as i32;
            }
        }
    }

    /// 更新队伍阵容；出走 ≥ ROSTER_CLEAR_DEPARTURES 人则积分清零
    /// （= Kotlin `applyRosterChange`；签名迁移：旧 key → 新 key）。
    pub fn apply_roster_change(&mut self, signature: &str, new_roster: &[String]) -> bool {
        let Some(team) = self.teams.get_mut(signature) else {
            Self::warn_unknown_signature(signature, "applyRosterChange");
            return false;
        };
        let new_sig = Self::signature_of(&team.team_name, new_roster);
        if new_sig != signature {
            let mut moved = self.teams.remove(signature).expect("已借出");
            moved.roster = new_roster.to_vec();
            let cleared = moved.departed_count() >= Self::ROSTER_CLEAR_DEPARTURES as usize;
            if cleared {
                moved.points = 0;
                moved.last_settled_roster = new_roster.to_vec();
            }
            self.teams.insert(new_sig, moved);
            if cleared {
                self.refresh_rankings();
            }
            cleared
        } else {
            team.roster = new_roster.to_vec();
            let cleared = team.departed_count() >= Self::ROSTER_CLEAR_DEPARTURES as usize;
            if cleared {
                team.points = 0;
                team.last_settled_roster = new_roster.to_vec();
                self.refresh_rankings();
            }
            cleared
        }
    }

    /// 转会窗结算：把全部队伍的「上次结算阵容」重置为当前阵容。
    ///
    /// `departed_count` 因此只统计**一个转会窗内**的累计离队人数——王朝队
    /// 每个窗口换 1 名替补、连续换 3 个窗是正常的阵容迭代，不应在第三个窗
    /// 突然触发「3 人离队积分清零」而从榜首掉进残废区。真正的 3 人核心拆队
    /// 仍会在同一窗内被 `apply_roster_change` 抓到并清零。
    pub fn settle_rosters(&mut self) {
        for team in self.teams.values_mut() {
            team.last_settled_roster = team.roster.clone();
        }
    }

    /// 打印未知队伍签名告警（= Kotlin `warnUnknownSignature` → System.err）。
    fn warn_unknown_signature(signature: &str, at: &str) {
        eprintln!("[VrsDatabase] {at}: 未知队伍签名「{signature}」——VRS 状态未更新");
    }

    /// 按积分降序刷新排名（1 起）。
    ///
    /// **确定性（2026 修订）**：同分按签名升序 tiebreak——此前同分队伍的名次
    /// 赋值顺序由 `HashMap` 迭代序决定，跨进程排名可能互换。
    fn refresh_rankings(&mut self) {
        let mut pairs: Vec<(String, i32)> = self
            .teams
            .iter()
            .map(|(k, t)| (k.clone(), t.points))
            .collect();
        pairs.sort_by(|(ka, a), (kb, b)| b.cmp(a).then_with(|| ka.cmp(kb)));
        for (idx, (sig, _)) in pairs.into_iter().enumerate() {
            if let Some(t) = self.teams.get_mut(&sig) {
                t.ranking = idx as i32 + 1;
            }
        }
    }
}

impl VrsDatabase {
    /// 阵容出走达到该人数即触发积分清零。
    pub const ROSTER_CLEAR_DEPARTURES: i32 = 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "rankings": [
            { "ranking": 1, "points": 2081, "teamName": "Vitality", "roster": ["apEX", "ZywOo", "ropz", "mezii", "flameZ"] },
            { "ranking": 2, "points": 1900, "teamName": "FaZe", "roster": ["karrigan", "broky", "rain", "frozen", "Twistzz"] }
        ]
    }"#;

    fn db() -> VrsDatabase {
        VrsDatabase::from_json_files(&[(
            "standings_global_2026_05_04.json".to_string(),
            SAMPLE.to_string(),
        )])
        .expect("样例资产合法")
    }

    #[test]
    fn month_of_parses() {
        assert_eq!(
            VrsDatabase::month_of("standings_global_2026_05_04.json"),
            "2026_05_04"
        );
    }

    #[test]
    fn parse_json_entries() {
        let entries = VrsDatabase::parse_json(SAMPLE);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].team_name, "Vitality");
        assert_eq!(entries[0].ranking, 1);
        assert_eq!(entries[0].roster.len(), 5);
    }

    #[test]
    fn checked_parse_rejects_corrupt_assets() {
        let base = |team: &str, points: i32, ranking: i32, roster: &str| {
            format!(
                r#"{{"rankings":[{{"ranking":{ranking},"points":{points},"teamName":"{team}","roster":{roster}}}]}}"#
            )
        };
        // 合法（含真实数据特征：同名不同队，如青训队与主队共用 org 名）
        let ok = r#"{"rankings":[
            {"ranking":1,"points":2000,"teamName":"BIG","roster":["a","b"]},
            {"ranking":2,"points":1500,"teamName":"BIG","roster":["c","d"]}
        ]}"#;
        assert_eq!(
            VrsDatabase::parse_json_checked(ok, "ok.json")
                .unwrap()
                .len(),
            2,
            "同名不同队合法"
        );

        let cases: Vec<(String, &str)> = vec![
            (base("", 100, 1, r#"["a"]"#), "空队名"),
            (base("T", -1, 1, r#"["a"]"#), "负积分"),
            (base("T", 100, 1, "[]"), "空阵容"),
            (base("T", 100, 0, r#"["a"]"#), "非正排名"),
        ];
        for (json, what) in cases {
            assert!(
                VrsDatabase::parse_json_checked(&json, "bad.json").is_err(),
                "应拒绝：{what}"
            );
        }
        // 结构错误（非 JSON）
        assert!(
            VrsDatabase::parse_json_checked("not json", "bad.json").is_err(),
            "应拒绝：结构错误"
        );
        // 错误信息可读（含文件与原因）
        let err =
            VrsDatabase::parse_json_checked(&base("T", -1, 1, r#"["a"]"#), "s.json").unwrap_err();
        assert!(
            err.to_string().contains("s.json") && err.to_string().contains("负"),
            "错误信息含文件与原因: {err}"
        );
    }

    #[test]
    fn from_json_files_propagates_validation_error() {
        let bad = r#"{"rankings":[{"ranking":1,"points":-5,"teamName":"T","roster":["a"]}]}"#;
        let result = VrsDatabase::from_json_files(&[(
            "standings_global_2026_05_04.json".to_string(),
            bad.to_string(),
        )]);
        assert!(result.is_err(), "校验失败必须显式传播（不静默空表）");
    }

    #[test]
    fn signature_matches_kotlin_format() {
        // Kotlin `sorted()` 为字典序（ASCII）：'Z'(90) < 'a'(97) → "ZywOo" 在前
        assert_eq!(
            VrsDatabase::signature_of("Vitality", &["apEX".to_string(), "ZywOo".to_string()]),
            "Vitality|ZywOo,apEX"
        );
        // 同队跨月稳定：roster 顺序变化不影响签名
        assert_eq!(
            VrsDatabase::signature_of("Vitality", &["ZywOo".to_string(), "apEX".to_string()]),
            "Vitality|ZywOo,apEX"
        );
    }

    #[test]
    fn load_initializes_teams_by_latest_month() {
        let d = db();
        assert_eq!(d.months, vec!["2026_05_04"]);
        assert_eq!(d.all_teams().len(), 2);
        // 按积分降序
        assert_eq!(d.all_teams()[0].team_name, "Vitality");
        assert_eq!(d.all_teams()[0].ranking, 1);
        assert_eq!(d.all_teams()[1].ranking, 2);
    }

    #[test]
    fn apply_match_updates_points_and_rankings() {
        let mut d = db();
        let winner_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        let loser_sig = VrsDatabase::signature_of(
            "FaZe",
            &[
                "karrigan".to_string(),
                "broky".to_string(),
                "rain".to_string(),
                "frozen".to_string(),
                "Twistzz".to_string(),
            ],
        );
        let before_w = d.team_of(&winner_sig).unwrap().points;
        let before_l = d.team_of(&loser_sig).unwrap().points;
        assert!(d.apply_match(&winner_sig, &loser_sig, TourneyTier::T1, None, None));
        assert!(d.team_of(&winner_sig).unwrap().points > before_w);
        assert!(d.team_of(&loser_sig).unwrap().points < before_l);
        assert_eq!(d.match_history_len(), 1);
        // 扩展世界中的新队伍按 0 分注册后正常结算。
        assert!(d.apply_match("Ghost|a,b", &loser_sig, TourneyTier::T1, None, None));
        assert!(d.team_of("Ghost|a,b").is_some());
        assert_eq!(d.match_history_len(), 2);
    }

    #[test]
    fn roster_change_clears_points_on_3_departures() {
        let mut d = db();
        let old_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        // 3 人出走 → 清零 + 签名迁移
        let new_roster = vec![
            "apEX".to_string(),
            "ZywOo".to_string(),
            "new1".to_string(),
            "new2".to_string(),
            "new3".to_string(),
        ];
        let cleared = d.apply_roster_change(&old_sig, &new_roster);
        assert!(cleared);
        let new_sig = VrsDatabase::signature_of("Vitality", &new_roster);
        let t = d.team_of(&new_sig).expect("签名应迁移到新 key");
        assert_eq!(t.points, 0);
        assert!(d.team_of(&old_sig).is_none(), "旧 key 应移除");
    }

    #[test]
    fn roster_change_no_clear_below_3() {
        let mut d = db();
        let old_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        let before = d.team_of(&old_sig).unwrap().points;
        let new_roster = vec![
            "apEX".to_string(),
            "ZywOo".to_string(),
            "ropz".to_string(),
            "new1".to_string(),
            "new2".to_string(),
        ];
        let cleared = d.apply_roster_change(&old_sig, &new_roster);
        assert!(!cleared);
        let new_sig = VrsDatabase::signature_of("Vitality", &new_roster);
        assert_eq!(
            d.team_of(&new_sig).unwrap().points,
            before,
            "2 人出走不清零"
        );
    }

    #[test]
    fn settle_rosters_makes_departures_window_scoped() {
        // 王朝队连续三个转会窗每窗换 1 人：不应在第三窗触发「3 人离队清零」。
        let mut d = db();
        let mut roster = vec![
            "apEX".to_string(),
            "ZywOo".to_string(),
            "ropz".to_string(),
            "mezii".to_string(),
            "flameZ".to_string(),
        ];
        let mut sig = VrsDatabase::signature_of("Vitality", &roster);
        let before = d.team_of(&sig).unwrap().points;
        for k in 0..3 {
            let old_sig = sig.clone();
            roster[2 + k] = format!("new{k}");
            sig = VrsDatabase::signature_of("Vitality", &roster);
            assert!(
                !d.apply_roster_change(&old_sig, &roster),
                "每窗只走 1 人，不应清零"
            );
            d.settle_rosters();
        }
        let t = d.team_of(&sig).expect("签名应逐窗迁移");
        assert_eq!(t.points, before, "三次 1 人换血不应触发积分清零");
        assert_eq!(t.roster, roster);
    }

    #[test]
    fn reseed_computes_seed_points_and_prunes_history() {
        let mut d = db();
        let winner_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        let loser_sig = VrsDatabase::signature_of(
            "FaZe",
            &[
                "karrigan".to_string(),
                "broky".to_string(),
                "rain".to_string(),
                "frozen".to_string(),
                "Twistzz".to_string(),
            ],
        );
        // 旧比赛（窗口外）与窗口内比赛
        let w = TimeWindow::new(500_000, 1_000_000);
        d.apply_match(
            &winner_sig,
            &loser_sig,
            TourneyTier::T1,
            Some(100_000),
            Some(&w),
        );
        d.apply_match(
            &winner_sig,
            &loser_sig,
            TourneyTier::T1,
            Some(900_000),
            Some(&w),
        );
        assert_eq!(d.match_history_len(), 2);
        d.reseed(&w);
        // 窗口外记录被剪枝
        assert_eq!(d.match_history_len(), 1, "旧比赛应在 reseed 时剪枝");
        // seed 分在 [400, 2000]
        let s = d.seed_points_of(&winner_sig).unwrap();
        assert!((400..=2000).contains(&s), "seed 分应在 [400,2000]: {s}");
    }

    #[test]
    fn consecutive_top_n_baseline() {
        let mut files = Vec::new();
        let month1 = r#"{"rankings":[{"ranking":1,"points":100,"teamName":"A","roster":["a1","a2"]},{"ranking":2,"points":90,"teamName":"B","roster":["b1","b2"]}]}"#;
        let month2 = r#"{"rankings":[{"ranking":2,"points":100,"teamName":"A","roster":["a1","a2"]},{"ranking":1,"points":110,"teamName":"B","roster":["b1","b2"]}]}"#;
        files.push((
            "standings_global_2026_01_05.json".to_string(),
            month1.to_string(),
        ));
        files.push((
            "standings_global_2026_02_05.json".to_string(),
            month2.to_string(),
        ));
        let d = VrsDatabase::from_json_files(&files).expect("样例资产合法");
        let a = VrsEntry::new(1, 100, "A", vec!["a1".to_string(), "a2".to_string()]);
        // 1 月前 2 名：连续 1 个月成立
        assert!(d.consecutive_top_n(&a, "2026_01_05", 2, 1));
        // A 2 月排名 2（前 2）→ 连续 2 个月成立
        assert!(d.consecutive_top_n(&a, "2026_02_05", 2, 2));
        // 前 1 名：2 月不成立
        assert!(!d.consecutive_top_n(&a, "2026_02_05", 1, 2));
        // 未知月份 → false
        assert!(!d.consecutive_top_n(&a, "2026_03_05", 2, 1));
    }

    // —— P2-4：VRS 动态注册边界 ——

    #[test]
    fn runtime_register_empty_roster_signature() {
        // 空 roster 签名（split 后无选手，如 "Org|"）：按 0 分注册、roster 为空、不 panic。
        let mut d = db();
        let empty_sig = "Newcomers|";
        assert!(d.team_of(empty_sig).is_none());
        // 直接触发动态注册，验证初始状态（0 分、空 roster、队名取 '|' 左侧）。
        d.ensure_runtime_team(empty_sig);
        let t = d.team_of(empty_sig).expect("空 roster 签名应被动态注册");
        assert_eq!(t.team_name, "Newcomers", "队名取 '|' 左侧");
        assert!(t.roster.is_empty(), "空 roster 应保留为空");
        assert_eq!(t.points, 0, "动态注册初始 0 分");
        // apply_match 路径：已注册的空 roster 队伍参与结算（积分增长），不 panic。
        let known_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        assert!(d.apply_match(empty_sig, &known_sig, TourneyTier::T1, None, None));
        assert!(d.team_of(empty_sig).unwrap().points > 0, "连胜后积分应增长");
        // 重复注册幂等：已存在时不覆盖现有状态。
        let pts_before = d.team_of(empty_sig).unwrap().points;
        d.ensure_runtime_team(empty_sig);
        assert_eq!(d.team_of(empty_sig).unwrap().points, pts_before);
    }

    #[test]
    fn runtime_register_malformed_signature_no_pipe() {
        // 畸形签名（无 '|' 分隔符）：不 panic，整串作为队名、roster 为空、0 分注册。
        let mut d = db();
        let malformed = "SoloOrg";
        assert!(d.team_of(malformed).is_none());
        d.ensure_runtime_team(malformed);
        let t = d.team_of(malformed).expect("无 '|' 签名应被动态注册");
        assert_eq!(t.team_name, "SoloOrg", "无分隔符时整串作为队名");
        assert!(t.roster.is_empty());
        assert_eq!(t.points, 0);
        // 排名参与：无分隔符队伍注册后也进入 all_teams 排序。
        assert!(d.all_teams().iter().any(|x| x.team_name == "SoloOrg"));
        // apply_match 路径：畸形签名队伍参与结算，不 panic。
        let known_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        assert!(d.apply_match(malformed, &known_sig, TourneyTier::T1, None, None));
        assert!(d.team_of(malformed).unwrap().points > 0);
    }

    #[test]
    fn runtime_team_rank_persists_across_restore() {
        // 动态注册队伍参与比赛获得积分后，序列化持久化 → 反序列化恢复后
        // 排名/积分/roster 保持一致（确定性可复现）。
        let mut d = db();
        let new_sig = "Rising|a,b,c";
        let known_sig = VrsDatabase::signature_of(
            "Vitality",
            &[
                "apEX".to_string(),
                "ZywOo".to_string(),
                "ropz".to_string(),
                "mezii".to_string(),
                "flameZ".to_string(),
            ],
        );
        for _ in 0..3 {
            d.apply_match(new_sig, &known_sig, TourneyTier::T1, None, None);
        }
        // 结算后动态队伍已有正积分与排名。
        let before = d.team_of(new_sig).expect("动态队伍已注册");
        let before_points = before.points;
        let before_rank = before.ranking;
        let before_roster = before.roster.clone();
        assert!(before_points > 0);

        // 序列化（持久化）→ 反序列化（恢复）。
        let json = serde_json::to_string(&d).expect("序列化应成功");
        let restored: VrsDatabase = serde_json::from_str(&json).expect("反序列化应成功");
        let after = restored.team_of(new_sig).expect("恢复后动态队伍仍存在");
        assert_eq!(after.points, before_points, "积分应跨恢复保留");
        assert_eq!(after.ranking, before_rank, "排名应跨恢复保留");
        assert_eq!(after.roster, before_roster, "roster 应跨恢复保留");
        // 完整 VrsDatabase 相等（round-trip 无损）。
        assert_eq!(restored, d, "序列化往返应无损");
    }
}
