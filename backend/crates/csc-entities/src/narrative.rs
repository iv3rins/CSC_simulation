//! 剧情进度与承诺（纯值；23 号方案 §5）——**只存无法从既有事实可靠派生的内容**。
//!
//! 设计边界：
//! - 现有声望/关系/荣誉/档案保持原事实源，本模块**不重复存储**它们；
//! - 本模块只保存剧情专用状态：弧进度、未兑现承诺、剧情 flags、按事件/对象的
//!   冷却、已提交场景 occurrence IDs、已达成终态；
//! - 全部 `serde(default)`——旧档读入补最小 `Idle` 进度，**不给旧档追加未发生的剧情**；
//! - 旧档未知日期/人物不猜测（缺失即 `None`/空），不按当前队伍反推历史归属。

use serde::{Deserialize, Serialize};

use csc_util::id::PlayerId;

/// 当前活动场景实例（选择前持久；重启后由同 `content_version` 的正文重新水合）。
///
/// 只存**稳定引用**（scene_id/instance_id/date/actor 稳定 ID），不存正文——
/// 正文由内容资产提供，内容版本独立演进。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveSceneRun {
    /// 场景定义 id（内容资产键）。
    pub scene_id: String,
    /// 场景实例 id（去重键；同场景不同触发时机不同实例）。
    pub instance_id: String,
    /// 呈现日期（SimClock 派生）。
    pub date: String,
    /// 演员角色（captain/teammate/coach/manager/media/self）。
    pub actor_role: String,
    /// 演员稳定 ID（真实人物才绑定；None = 概括角色）。
    #[serde(default)]
    pub actor_id: Option<PlayerId>,
    /// 演员展示名（快照；真实人物名为准，None = 角色名）。
    #[serde(default)]
    pub actor_name: Option<String>,
}

/// 一条「承诺」——剧情选择产生的、可在以后检查是否兑现的约定。
///
/// 不是一句存起来的话：必须解析成具体 `due_month_index`（模拟月序）与可检查目标 key。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promise {
    /// 承诺 key（稳定；如 `review_before_next_match`）。
    pub key: String,
    /// 到期模拟月序（`year*12+month`）；推进到此月序仍未兑现即视为逾期。
    pub due_month_index: i32,
    /// 产生该承诺的场景实例 id（可追溯来源）。
    pub from_scene: String,
    /// 状态。
    pub status: PromiseStatus,
}

/// 承诺状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseStatus {
    /// 待兑现。
    Pending,
    /// 已兑现。
    Fulfilled,
    /// 已失信（逾期或显式放弃）。
    Broken,
}

/// 已提交选择的结构化历史；中文仅是结果投影。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrativeChoiceRecord {
    pub scene_id: String,
    pub instance_id: String,
    pub choice_id: String,
    pub date: String,
    pub actor_id: Option<PlayerId>,
    pub immediate: Vec<String>,
    pub tracking: Vec<String>,
}

/// 剧情进度聚合（`GameState.narrative`，v10 新增）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeProgress {
    /// 叙事内容版本（与 `GameState` 格式版本、`WORLD_SIM_VERSION` 分开管理）。
    pub content_version: u32,
    /// 各剧情弧的进度计数（arc_id → 已推进场景数；驱动弧内顺序）。
    #[serde(default)]
    pub arc_stage: std::collections::HashMap<String, i32>,
    /// 未兑现/历史承诺。
    #[serde(default)]
    pub promises: Vec<Promise>,
    /// 剧情专用 flags（如 `team_first`）；与既有 CareerMark 等不重复。
    #[serde(default)]
    pub flags: Vec<String>,
    /// 已提交场景的 occurrence id（去重；同一场景实例不重复触发）。
    #[serde(default)]
    pub seen_scenes: Vec<String>,
    /// 按事件/对象的冷却：key → 最近触发的模拟月序（含对象稳定 ID，如 `arc:rookie:x`）。
    #[serde(default)]
    pub cooldowns: std::collections::HashMap<String, i32>,
    /// 已达成终态（弧的多终态记录；用于验收「每条弧至少两个不同终态」）。
    #[serde(default)]
    pub terminal_states: Vec<String>,
    /// 已完成的场景数（诊断/进度展示；= seen_scenes.len() 的派生缓存）。
    #[serde(default)]
    pub scenes_completed: i32,
    /// 当前活动场景（等待玩家选择时持久；None = 无进行中场景）。
    #[serde(default)]
    pub active_scene: Option<ActiveSceneRun>,
    /// 已选后继；只有该场景满足自身条件后才可进入。
    #[serde(default)]
    pub next_scenes: std::collections::HashMap<String, String>,
    /// 无后继的已闭合弧；显式尾声边完成后才关闭。
    #[serde(default)]
    pub closed_arcs: Vec<String>,
    #[serde(default)]
    pub choice_history: Vec<NarrativeChoiceRecord>,
}

impl Default for NarrativeProgress {
    fn default() -> Self {
        Self {
            content_version: 1,
            arc_stage: std::collections::HashMap::new(),
            promises: Vec::new(),
            flags: Vec::new(),
            seen_scenes: Vec::new(),
            cooldowns: std::collections::HashMap::new(),
            terminal_states: Vec::new(),
            scenes_completed: 0,
            active_scene: None,
            next_scenes: std::collections::HashMap::new(),
            closed_arcs: Vec::new(),
            choice_history: Vec::new(),
        }
    }
}

impl NarrativeProgress {
    /// 是否已见过某场景实例（去重判定）。
    pub fn has_seen(&self, occurrence_id: &str) -> bool {
        self.seen_scenes.iter().any(|s| s == occurrence_id)
    }

    /// 标记某场景实例已见（幂等）。
    pub fn mark_seen(&mut self, occurrence_id: &str) {
        if !self.has_seen(occurrence_id) {
            self.seen_scenes.push(occurrence_id.to_string());
            self.scenes_completed = self.seen_scenes.len() as i32;
        }
    }

    /// 是否含某剧情 flag。
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// 设置剧情 flag（幂等）。
    pub fn set_flag(&mut self, flag: &str) {
        if !self.has_flag(flag) {
            self.flags.push(flag.to_string());
        }
    }

    /// 查询某 key 的冷却月序（None = 从未触发）。
    pub fn cooldown_of(&self, key: &str) -> Option<i32> {
        self.cooldowns.get(key).copied()
    }

    /// 设置冷却（覆盖为更近的月序）。
    pub fn set_cooldown(&mut self, key: &str, month_index: i32) {
        self.cooldowns.insert(key.to_string(), month_index);
    }

    /// 该 key 是否仍在冷却窗口内（`now - last < cooldown_months`）。
    pub fn in_cooldown(&self, key: &str, now_month_index: i32, cooldown_months: i32) -> bool {
        self.cooldown_of(key)
            .is_some_and(|last| now_month_index - last < cooldown_months)
    }

    /// 写入一条承诺（幂等：同 key 未结清的旧承诺先置 Broken，避免并行悬挂）。
    pub fn add_promise(&mut self, key: &str, due_month_index: i32, from_scene: &str) {
        for p in self.promises.iter_mut() {
            if p.key == key && p.status == PromiseStatus::Pending {
                p.status = PromiseStatus::Broken;
            }
        }
        self.promises.push(Promise {
            key: key.to_string(),
            due_month_index,
            from_scene: from_scene.to_string(),
            status: PromiseStatus::Pending,
        });
    }

    /// 兑现一条承诺（返回是否命中待兑现项）。
    pub fn fulfill_promise(&mut self, key: &str) -> bool {
        let mut hit = false;
        for p in self.promises.iter_mut() {
            if p.key == key && p.status == PromiseStatus::Pending {
                p.status = PromiseStatus::Fulfilled;
                hit = true;
            }
        }
        hit
    }

    /// 标记一条承诺失信（返回是否命中待兑现项）。
    pub fn break_promise(&mut self, key: &str) -> bool {
        let mut hit = false;
        for p in self.promises.iter_mut() {
            if p.key == key && p.status == PromiseStatus::Pending {
                p.status = PromiseStatus::Broken;
                hit = true;
            }
        }
        hit
    }

    /// 当前所有待兑现承诺（供 CareerView「未兑现承诺」展示）。
    pub fn pending_promises(&self) -> Vec<&Promise> {
        self.promises
            .iter()
            .filter(|p| p.status == PromiseStatus::Pending)
            .collect()
    }

    /// 已达成的终态集合。
    pub fn contains_terminal(&self, state: &str) -> bool {
        self.terminal_states.iter().any(|s| s == state)
    }

    /// 记录一个终态（幂等）。
    pub fn mark_terminal(&mut self, state: &str) {
        if !self.contains_terminal(state) {
            self.terminal_states.push(state.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promise_lifecycle() {
        let mut p = NarrativeProgress::default();
        p.add_promise("review_before_next_match", 2026 * 12 + 3, "scene-1");
        assert_eq!(p.pending_promises().len(), 1);
        assert!(p.fulfill_promise("review_before_next_match"));
        assert!(p.pending_promises().is_empty());
        assert!(
            !p.fulfill_promise("review_before_next_match"),
            "已兑现不重复"
        );
    }

    #[test]
    fn promise_dedup_breaks_old_pending() {
        let mut p = NarrativeProgress::default();
        p.add_promise("k", 100, "s1");
        p.add_promise("k", 200, "s2");
        // 旧 Pending 被置 Broken，只保留一条 Pending。
        assert_eq!(p.pending_promises().len(), 1);
        assert_eq!(p.pending_promises()[0].due_month_index, 200);
    }

    #[test]
    fn cooldown_window() {
        let mut p = NarrativeProgress::default();
        p.set_cooldown("arc:rookie", 12);
        assert!(p.in_cooldown("arc:rookie", 13, 3));
        assert!(!p.in_cooldown("arc:rookie", 15, 3));
        assert!(!p.in_cooldown("arc:other", 13, 3));
    }

    #[test]
    fn seen_scenes_dedup() {
        let mut p = NarrativeProgress::default();
        p.mark_seen("rookie.welcome#1");
        p.mark_seen("rookie.welcome#1");
        assert_eq!(p.scenes_completed, 1);
        assert!(p.has_seen("rookie.welcome#1"));
    }

    #[test]
    fn serde_roundtrip_backfills_defaults() {
        let p = NarrativeProgress::default();
        let json = serde_json::to_string(&p).unwrap();
        let back: NarrativeProgress = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        // 缺字段的旧形状可读入（serde default）。
        let minimal = r#"{"content_version":1}"#;
        let parsed: NarrativeProgress = serde_json::from_str(minimal).unwrap();
        assert!(parsed.seen_scenes.is_empty());
    }
}
