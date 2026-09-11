//! 叙事导演（`NarrativeDirector`，23 号方案 §5）——**纯函数**按优先级选一个场景。
//!
//! 输入只读世界事实（系列赛计数/失利/转会接触/承诺/进度/日期）；输出
//! `Option<SceneDef>`（稳定 ID 排序，不依赖 HashMap 顺序）。选择顺序：
//! 1. 已到期的承诺（`PromiseDue`）——先处理因果后果；
//! 2. 关键职业节点（Critical > Important：弧内的 `AfterScene`/`AfterLoss` 等）；
//! 3. 普通互动（弧入口/延迟回访）。
//!
//! 触发判定只读既有事实，不消费 RNG（场景自身不掷点；状态变更在 [`crate::narrative::apply`]）。

use csc_entities::narrative::{ActiveSceneRun, NarrativeProgress};

use super::content::{NarrativeContent, SceneDef, Trigger};

/// 只读世界事实（Director 决策输入）。
#[derive(Debug, Clone, Default)]
pub struct NarrativeFacts {
    /// 已完成正式系列赛场数（主角参与）。
    pub series_played: i32,
    /// 已失利场数（主角参与且落败）。
    pub series_lost: i32,
    /// 当前模拟月序（`year*12+month`）。
    pub month_index: i32,
    /// 当前日期标签（`YYYY-MM-DD`）。
    pub date: String,
    /// 是否存在真实招募接触（`transfer_contacts` 非空）。
    pub has_transfer_contact: bool,
}

/// 已见场景/实例的判定上下文。
pub struct Director;

impl Director {
    /// 选择一个可触发场景。`None` = 当前无剧情可用（继续正常推进）。
    pub fn select(
        content: &NarrativeContent,
        progress: &NarrativeProgress,
        facts: &NarrativeFacts,
    ) -> Option<SceneDef> {
        // 已有活动场景未完成 → 不选新场景（同时刻只处理一个主场景）。
        if progress.active_scene.is_some() {
            return None;
        }
        // 稳定排序：先 priority（Critical 高），再 order 字段缺失时按 id 字典序。
        let mut candidates: Vec<&SceneDef> = content
            .scenes
            .iter()
            .filter(|s| {
                if progress.closed_arcs.contains(&s.arc) {
                    return false;
                }
                let incoming = content
                    .scenes
                    .iter()
                    .flat_map(|s| &s.choices)
                    .any(|c| c.next_scene.as_deref() == Some(s.id.as_str()));
                let path_allowed = match progress.next_scenes.get(&s.arc) {
                    Some(next) => next == &s.id,
                    None => !incoming && progress.arc_stage.get(&s.arc).copied().unwrap_or(0) == 0,
                };
                path_allowed && Self::is_eligible(s, progress, facts)
            })
            .collect();
        // priority 降序（Critical 最高），同优先级按 id 字典序（稳定，不依赖插入序）。
        candidates.sort_by(|a, b| {
            matches!(b.trigger, Trigger::PromiseDue { .. })
                .cmp(&matches!(a.trigger, Trigger::PromiseDue { .. }))
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.id.cmp(&b.id))
        });
        candidates.into_iter().next().cloned()
    }

    /// 场景是否可触发（触发事实满足 + 未见过 + flag 前置 + 无冷却）。
    fn is_eligible(scene: &SceneDef, progress: &NarrativeProgress, facts: &NarrativeFacts) -> bool {
        // 场景实例去重：同场景已 seen（用 scene_id 前缀判定；实例 id 由 date 组成）。
        if progress
            .seen_scenes
            .iter()
            .any(|s| s == &scene.id || s.starts_with(&format!("{}#", scene.id)))
        {
            return false;
        }
        // flag 前置：requires_any_flag 非空时任一满足。
        if !scene.requires_any_flag.is_empty()
            && !scene.requires_any_flag.iter().any(|f| progress.has_flag(f))
        {
            return false;
        }
        // 冷却：按 `arc:{arc}` 与 `scene:{id}` 双键。
        if progress.in_cooldown(&format!("scene:{}", scene.id), facts.month_index, 1) {
            return false;
        }
        Self::trigger_satisfied(&scene.trigger, progress, facts)
    }

    /// 触发条件判定（纯值）。
    fn trigger_satisfied(
        trigger: &Trigger,
        progress: &NarrativeProgress,
        facts: &NarrativeFacts,
    ) -> bool {
        match trigger {
            Trigger::ArcEntry { arc } => {
                // 该弧尚无任何已见场景 → 弧入口。
                let arc_seen = progress.arc_stage.get(arc).is_some_and(|stage| *stage > 0);
                !arc_seen
            }
            Trigger::AfterSeries { min_series } => facts.series_played >= *min_series,
            Trigger::AfterLoss {
                min_losses,
                min_series,
            } => facts.series_lost >= *min_losses && facts.series_played >= *min_series,
            Trigger::AfterScene { after_scene } => {
                progress.has_seen(after_scene)
                    || progress
                        .seen_scenes
                        .iter()
                        .any(|s| s.starts_with(&format!("{after_scene}#")))
            }
            Trigger::PromiseDue { promise } => progress.promises.iter().any(|p| {
                &p.key == promise
                    && p.status == csc_entities::narrative::PromiseStatus::Pending
                    && p.due_month_index <= facts.month_index
            }),
            Trigger::DelayMonths {
                after_scene,
                months,
            } => {
                let seen_after = progress.has_seen(after_scene)
                    || progress
                        .seen_scenes
                        .iter()
                        .any(|s| s.starts_with(&format!("{after_scene}#")));
                let due = progress
                    .cooldown_of(&format!("after:{after_scene}"))
                    .is_some_and(|at| facts.month_index - at >= *months);
                seen_after && due
            }
            Trigger::HasTransferContact { min_series } => {
                facts.has_transfer_contact && facts.series_played >= *min_series
            }
            Trigger::FirstLoss => facts.series_lost >= 1,
        }
    }
}

/// 构造某场景当前实例（instance_id 含日期，保证同场景不同时机不同实例）。
pub fn instantiate(scene: &SceneDef, facts: &NarrativeFacts) -> ActiveSceneRun {
    ActiveSceneRun {
        scene_id: scene.id.clone(),
        instance_id: format!("{}#{}", scene.id, facts.date),
        date: facts.date.clone(),
        actor_role: scene.actor_role.clone(),
        actor_id: None,
        actor_name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::content::{ChoiceDef, Effect, NarrativeContent, ScenePriority};
    use super::*;

    fn content() -> NarrativeContent {
        NarrativeContent {
            content_version: 1,
            arcs: vec![super::super::content::ArcDef {
                id: "rookie_belonging".into(),
                title: "轮到你证明自己".into(),
                order: 1,
            }],
            scenes: vec![
                SceneDef {
                    id: "rookie.welcome".into(),
                    arc: "rookie_belonging".into(),
                    chapter_title: "第一章".into(),
                    title: "welcome".into(),
                    location: "l".into(),
                    priority: ScenePriority::Important,
                    actor_role: "captain".into(),
                    trigger: Trigger::ArcEntry {
                        arc: "rookie_belonging".into(),
                    },
                    requires_any_flag: vec![],
                    paragraphs: vec!["p".into()],
                    choices: vec![ChoiceDef {
                        id: "A".into(),
                        label: "a".into(),
                        impact_certain: String::new(),
                        impact_possible: String::new(),
                        effects: vec![Effect::SetStoryFlag {
                            flag: "team_first".into(),
                        }],
                        next_scene: None,
                    }],
                },
                SceneDef {
                    id: "rookie.first_talk".into(),
                    arc: "rookie_belonging".into(),
                    chapter_title: "第一章".into(),
                    title: "talk".into(),
                    location: "l".into(),
                    priority: ScenePriority::Important,
                    actor_role: "captain".into(),
                    trigger: Trigger::AfterSeries { min_series: 1 },
                    requires_any_flag: vec!["team_first".into()],
                    paragraphs: vec!["p".into()],
                    choices: vec![ChoiceDef {
                        id: "B".into(),
                        label: "b".into(),
                        impact_certain: String::new(),
                        impact_possible: String::new(),
                        effects: vec![],
                        next_scene: None,
                    }],
                },
            ],
        }
    }

    #[test]
    fn arc_entry_triggers_when_unseen() {
        let c = content();
        let p = NarrativeProgress::default();
        let facts = NarrativeFacts {
            date: "2026-02-01".into(),
            month_index: 2026 * 12 + 2,
            ..Default::default()
        };
        let sel = Director::select(&c, &p, &facts).expect("应有弧入口");
        assert_eq!(sel.id, "rookie.welcome");
    }

    #[test]
    fn requires_flag_blocks_until_set() {
        let c = content();
        let mut p = NarrativeProgress::default();
        p.mark_seen("rookie.welcome");
        p.arc_stage.insert("rookie_belonging".into(), 1);
        let facts = NarrativeFacts {
            series_played: 2,
            date: "2026-03-01".into(),
            month_index: 2026 * 12 + 3,
            ..Default::default()
        };
        // 无 flag → first_talk 不可触发。
        assert!(Director::select(&c, &p, &facts).is_none());
        // 设置 flag → 可触发。
        p.set_flag("team_first");
        p.next_scenes
            .insert("rookie_belonging".into(), "rookie.first_talk".into());
        let sel = Director::select(&c, &p, &facts).expect("应有续接场景");
        assert_eq!(sel.id, "rookie.first_talk");
    }

    #[test]
    fn seen_scene_not_reselected() {
        let c = content();
        let mut p = NarrativeProgress::default();
        p.mark_seen("rookie.welcome");
        p.arc_stage.insert("rookie_belonging".into(), 1);
        let facts = NarrativeFacts {
            date: "2026-02-01".into(),
            month_index: 2026 * 12 + 2,
            ..Default::default()
        };
        // 弧入口已见；first_talk 无 flag → 无候选。
        assert!(Director::select(&c, &p, &facts).is_none());
    }

    #[test]
    fn active_scene_blocks_new_selection() {
        let c = content();
        let p = NarrativeProgress {
            active_scene: Some(ActiveSceneRun {
                scene_id: "rookie.welcome".into(),
                instance_id: "rookie.welcome#2026-02-01".into(),
                date: "2026-02-01".into(),
                actor_role: "captain".into(),
                actor_id: None,
                actor_name: None,
            }),
            ..Default::default()
        };
        let facts = NarrativeFacts {
            date: "2026-02-01".into(),
            month_index: 2026 * 12 + 2,
            ..Default::default()
        };
        assert!(
            Director::select(&c, &p, &facts).is_none(),
            "有活动场景时不选新场景"
        );
    }
    #[test]
    fn selected_path_waits_for_due_date_and_excludes_other_branch() {
        let c = NarrativeContent::from_json_str(include_str!(
            "../../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap();
        let mut p = NarrativeProgress::default();
        p.mark_seen("rookie.welcome#d");
        p.mark_seen("rookie.first_talk#d");
        p.arc_stage.insert("rookie_belonging".into(), 2);
        p.next_scenes
            .insert("rookie_belonging".into(), "rookie.review_followup".into());
        p.set_cooldown("after:rookie.first_talk", 0);
        p.add_promise("review_before_next_match", 999, "rookie.first_talk");
        let mut facts = NarrativeFacts {
            month_index: 10,
            ..Default::default()
        };
        assert!(
            Director::select(&c, &p, &facts).is_none(),
            "future promise and unchosen solo branch must both wait"
        );
        facts.month_index = 999;
        assert_eq!(
            Director::select(&c, &p, &facts).unwrap().id,
            "rookie.review_followup"
        );
        p.closed_arcs.push("rookie_belonging".into());
        assert!(
            Director::select(&c, &p, &facts).is_none(),
            "closed arc cannot reenter"
        );
    }

    #[test]
    fn explicit_terminal_epilogue_remains_reachable_after_restore() {
        let c = NarrativeContent::from_json_str(include_str!(
            "../../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap();
        let mut p = NarrativeProgress::default();
        p.closed_arcs.push("rookie_belonging".into());
        p.mark_seen("transfer.letter#d");
        p.mark_terminal("stayed_loyal");
        p.arc_stage.insert("stay_or_leave".into(), 1);
        p.next_scenes
            .insert("stay_or_leave".into(), "transfer.loyal_followup".into());
        p.set_cooldown("after:transfer.letter", 10);
        let restored: NarrativeProgress =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        let facts = NarrativeFacts {
            month_index: 11,
            ..Default::default()
        };
        assert_eq!(
            Director::select(&c, &restored, &facts).unwrap().id,
            "transfer.loyal_followup"
        );
        assert_eq!(p, restored);
    }
}
