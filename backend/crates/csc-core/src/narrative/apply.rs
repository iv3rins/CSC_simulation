//! Validated narrative transaction: every fallible precondition is checked before mutation.
use super::content::{ChoiceDef, Effect, SceneDef};
use csc_entities::narrative::{NarrativeChoiceRecord, NarrativeProgress, PromiseStatus};
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_util::{SimError, id::PlayerId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppliedOutcome {
    pub immediate: Vec<String>,
    pub tracking: Vec<String>,
}

/// Inputs captured from the active scene by its owning engine.
pub struct ChoiceContext<'a> {
    pub scene: &'a SceneDef,
    pub choice: &'a ChoiceDef,
    pub player_id: PlayerId,
    pub actor_id: Option<PlayerId>,
    pub month_index: i32,
    pub date: &'a str,
}

pub fn apply_choice(
    world: &mut World,
    journal: &mut WorldJournal,
    progress: &mut NarrativeProgress,
    context: ChoiceContext<'_>,
) -> Result<AppliedOutcome, SimError> {
    let ChoiceContext {
        scene,
        choice,
        player_id,
        actor_id,
        month_index,
        date,
    } = context;
    let invalid = |reason: &str| {
        SimError::protocol(format!("场景 {} / 选项 {}：{reason}", scene.id, choice.id))
    };
    let active = progress
        .active_scene
        .as_ref()
        .filter(|a| a.scene_id == scene.id)
        .ok_or_else(|| invalid("场景已失效"))?
        .clone();
    if progress.has_seen(&active.instance_id) {
        return Err(invalid("场景已提交"));
    }
    let player = world
        .player(player_id)
        .ok_or_else(|| invalid("主体已不存在"))?;
    let team_id = player.team;
    let player_name = player.name.clone();
    let actor_name = actor_id
        .and_then(|id| world.player(id))
        .filter(|a| a.id != player_id && a.team == team_id && team_id.is_some())
        .map(|a| a.name.clone());
    let mut promise_plan = progress.clone();
    let mut relation_delta = 0.0;
    let mut cohesion_delta = 0.0;
    for effect in &choice.effects {
        match effect {
            Effect::CreatePromise {
                promise,
                due_months,
            } => {
                if *due_months <= 0
                    || month_index.checked_add(*due_months).is_none()
                    || promise_plan
                        .pending_promises()
                        .iter()
                        .any(|p| p.key == *promise)
                {
                    return Err(invalid("承诺重复或到期时间非法"));
                }
                promise_plan.add_promise(promise, month_index + due_months, &scene.id);
            }
            Effect::FulfillPromise { promise } | Effect::BreakPromise { promise } => {
                if !promise_plan
                    .promises
                    .iter()
                    .any(|p| p.key == *promise && p.status == PromiseStatus::Pending)
                {
                    return Err(invalid("待兑现承诺不存在"));
                }
                promise_plan.fulfill_promise(promise);
            }
            Effect::RelationDelta { amount } => {
                if !amount.is_finite() || actor_name.is_none() {
                    return Err(invalid("关系对象已失效或增量非法"));
                }
                relation_delta += amount;
            }
            Effect::TeamCohesionDelta { delta } => {
                if !delta.is_finite() || team_id.and_then(|id| world.team(id)).is_none() {
                    return Err(invalid("战队已失效或凝聚力增量非法"));
                }
                cohesion_delta += delta;
            }
            Effect::ReputationDelta { .. } if player.career.is_none() => {
                return Err(invalid("主体没有生涯档案"));
            }
            Effect::StayTeam if team_id.and_then(|id| world.team(id)).is_none() => {
                return Err(invalid("当前没有可留队的战队"));
            }
            Effect::AcceptTransfer => {
                return Err(invalid("签约必须通过真实转会报价系统，剧情不能代签"));
            }
            _ => {}
        }
    }
    if !relation_delta.is_finite() || !cohesion_delta.is_finite() {
        return Err(invalid("效果合计溢出"));
    }
    // From here on, all operations are infallible under the single owner contract.
    let mut outcome = AppliedOutcome::default();
    for effect in &choice.effects {
        match effect {
            Effect::CreatePromise {
                promise,
                due_months,
            } => {
                progress.add_promise(promise, month_index + due_months, &scene.id);
                outcome.immediate.push(format!(
                    "已约定：{}；{due_months}个月后回访。",
                    promise_label(promise)
                ));
                outcome.tracking.push(promise.clone());
            }
            Effect::FulfillPromise { promise } => {
                progress.fulfill_promise(promise);
                outcome
                    .immediate
                    .push(format!("兑现承诺：{}", promise_label(promise)));
            }
            Effect::BreakPromise { promise } => {
                progress.break_promise(promise);
                outcome
                    .immediate
                    .push(format!("未兑现承诺：{}", promise_label(promise)));
            }
            Effect::SetStoryFlag { flag } => {
                progress.set_flag(flag);
                outcome.immediate.push("这次选择已记入生涯档案。".into());
            }
            Effect::MoraleDelta { delta } => {
                let player = world.player_mut(player_id).expect("validated player");
                let before = player.pro.morale;
                player.pro.morale = before.saturating_add(*delta).clamp(0, 100);
                outcome
                    .immediate
                    .push(format!("你的士气 {:+}", player.pro.morale - before));
            }
            Effect::ReputationDelta { delta } => {
                let career = world
                    .player_mut(player_id)
                    .and_then(|p| p.career_mut())
                    .expect("validated career");
                let before = career.reputation;
                career.reputation = before.saturating_add(*delta).clamp(0, 100);
                outcome
                    .immediate
                    .push(format!("你的声誉 {:+}", career.reputation - before));
            }
            Effect::MarkTerminal { state } => {
                progress.mark_terminal(state);
            }
            Effect::StayTeam => {
                progress.set_flag("committed_to_current_team");
                outcome
                    .immediate
                    .push("已记下留队立场；合同没有变化。".into());
            }
            Effect::RelationDelta { .. }
            | Effect::TeamCohesionDelta { .. }
            | Effect::AcceptTransfer => {}
        }
    }
    if relation_delta != 0.0 || cohesion_delta != 0.0 {
        let tid = team_id.expect("validated team");
        let roster: Vec<String> = world.roster(tid).iter().map(|p| p.name.clone()).collect();
        let team = world.team_mut(tid).expect("validated team");
        let before_cohesion = team.chemistry.cohesion;
        if relation_delta != 0.0 {
            let name = actor_name.expect("validated actor");
            let before = team.chemistry.relation_of(&player_name, &name);
            team.chemistry.adjust(&player_name, &name, relation_delta);
            outcome.immediate.push(format!(
                "与{name}的关系 {:+.1}",
                team.chemistry.relation_of(&player_name, &name) - before
            ));
            team.chemistry.cohesion = csc_simulation::chemistry_model::ChemistryModel::cohesion_of(
                &team.chemistry,
                &roster,
            );
        }
        team.chemistry.cohesion = (team.chemistry.cohesion + cohesion_delta).clamp(0.0, 100.0);
        outcome.immediate.push(format!(
            "队伍凝聚力 {:+.1}",
            team.chemistry.cohesion - before_cohesion
        ));
    }
    journal.record(WorldEvent::LiveUpdate {
        date: date.into(),
        seq: -1,
        headline: format!("剧情推进：{}", scene.title),
        detail: format!("你在「{}」中选择了：{}", scene.title, choice.label),
    });
    progress.mark_seen(&active.instance_id);
    progress.active_scene = None;
    *progress.arc_stage.entry(scene.arc.clone()).or_insert(0) += 1;
    if let Some(next) = &choice.next_scene {
        progress.next_scenes.insert(scene.arc.clone(), next.clone());
    } else {
        progress.next_scenes.remove(&scene.arc);
        if !progress.closed_arcs.contains(&scene.arc) {
            progress.closed_arcs.push(scene.arc.clone());
        }
    }
    progress.set_cooldown(&format!("scene:{}", scene.id), month_index);
    progress.set_cooldown(&format!("after:{}", scene.id), month_index);
    progress.choice_history.push(NarrativeChoiceRecord {
        scene_id: scene.id.clone(),
        instance_id: active.instance_id,
        choice_id: choice.id.clone(),
        date: date.into(),
        actor_id,
        immediate: outcome.immediate.clone(),
        tracking: outcome.tracking.clone(),
    });
    Ok(outcome)
}

/// Player-facing labels are presentation only; tracking keeps the stable key.
fn promise_label(key: &str) -> &'static str {
    match key {
        "complete_team_review" => "一起完成团队复盘",
        "review_before_next_match" => "一起复盘比赛",
        "step_up_next_match" => "主动承担比赛责任",
        "public_show_result" => "用表现回应期待",
        "trust_conversation" => "再谈谈彼此的信任",
        _ => "约定",
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::dummy_world;
    use super::*;
    use csc_entities::narrative::PromiseStatus;

    #[test]
    fn promise_effects_are_recorded() {
        let mut world = dummy_world();
        let pid = world.all_players_only()[0].id;
        let mut journal = WorldJournal::default();
        let mut progress = NarrativeProgress::default();
        let scene = SceneDef {
            id: "s".into(),
            arc: "a".into(),
            chapter_title: String::new(),
            title: "t".into(),
            location: String::new(),
            priority: super::super::content::ScenePriority::Normal,
            actor_role: "captain".into(),
            trigger: super::super::content::Trigger::FirstLoss,
            requires_any_flag: vec![],
            paragraphs: vec![],
            choices: vec![],
        };
        progress.active_scene = Some(csc_entities::narrative::ActiveSceneRun {
            scene_id: "s".into(),
            instance_id: "s#d".into(),
            date: "d".into(),
            actor_role: "captain".into(),
            actor_id: None,
            actor_name: None,
        });
        let choice = ChoiceDef {
            id: "C".into(),
            label: "l".into(),
            impact_certain: "".into(),
            impact_possible: "".into(),
            effects: vec![
                Effect::CreatePromise {
                    promise: "p".into(),
                    due_months: 2,
                },
                Effect::MarkTerminal {
                    state: "term".into(),
                },
            ],
            next_scene: None,
        };
        let out = apply_choice(
            &mut world,
            &mut journal,
            &mut progress,
            ChoiceContext {
                scene: &scene,
                choice: &choice,
                player_id: pid,
                actor_id: None,
                month_index: 100,
                date: "2026-02-01",
            },
        )
        .unwrap();
        assert_eq!(progress.promises.len(), 1);
        assert_eq!(progress.promises[0].status, PromiseStatus::Pending);
        assert!(progress.contains_terminal("term"));
        assert!(out.tracking.contains(&"p".to_string()));
        assert!(!journal.all().is_empty());
        assert_eq!(progress.arc_stage.get("a"), Some(&1));
    }
    #[test]
    fn invalid_actor_rejects_entire_choice_without_consuming_scene() {
        let mut world = dummy_world();
        let pid = world.all_players_only()[0].id;
        let content = super::super::content::NarrativeContent::from_json_str(include_str!(
            "../../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap();
        let scene = content.scene("rookie.welcome").unwrap();
        let mut progress = NarrativeProgress {
            active_scene: Some(super::super::instantiate(
                scene,
                &super::super::NarrativeFacts {
                    date: "2026-02-01".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let mut journal = WorldJournal::default();
        let before = serde_json::to_string(&world).unwrap();
        let old_progress = progress.clone();
        let choice = &scene.choices[0];
        let result = apply_choice(
            &mut world,
            &mut journal,
            &mut progress,
            ChoiceContext {
                scene,
                choice,
                player_id: pid,
                actor_id: None,
                month_index: 100,
                date: "2026-02-01",
            },
        );
        assert!(result.is_err());
        assert_eq!(serde_json::to_string(&world).unwrap(), before);
        assert_eq!(progress, old_progress);
        assert!(journal.all().is_empty());
    }

    #[test]
    fn combined_relation_cohesion_uses_actual_clamped_deltas_and_records_path() {
        let mut world = dummy_world();
        let pid = world.all_players_only()[0].id;
        let tid = world.player(pid).unwrap().team.unwrap();
        let aid = world.create_player(
            csc_domain::tier::Tier::Tier4,
            Some("Mate"),
            &mut csc_util::rng::Xoshiro256StarStar::seed(2),
            None,
            None,
        );
        world.assign_player_to_team(aid, tid).unwrap();
        world
            .team_mut(tid)
            .unwrap()
            .chemistry
            .set_relation("MyPlayer", "Mate", 99.0);
        world
            .team_mut(tid)
            .unwrap()
            .chemistry
            .set_relation("Mate", "MyPlayer", 99.0);
        world.team_mut(tid).unwrap().chemistry.cohesion = 99.0;
        let content = super::super::content::NarrativeContent::from_json_str(include_str!(
            "../../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap();
        let scene = content.scene("rookie.welcome").unwrap();
        let choice = ChoiceDef {
            effects: vec![
                Effect::RelationDelta { amount: 8.0 },
                Effect::TeamCohesionDelta { delta: 2.0 },
            ],
            ..scene.choices[0].clone()
        };
        let mut progress = NarrativeProgress {
            active_scene: Some(super::super::instantiate(
                scene,
                &super::super::NarrativeFacts {
                    date: "d".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let mut journal = WorldJournal::default();
        let out = apply_choice(
            &mut world,
            &mut journal,
            &mut progress,
            ChoiceContext {
                scene,
                choice: &choice,
                player_id: pid,
                actor_id: Some(aid),
                month_index: 100,
                date: "d",
            },
        )
        .unwrap();
        assert_eq!(world.team(tid).unwrap().chemistry.cohesion, 100.0);
        assert!(out.immediate.iter().any(|s| s == "与Mate的关系 +1.0"));
        assert!(out.immediate.iter().any(|s| s == "队伍凝聚力 +1.0"));
        assert_eq!(
            progress
                .next_scenes
                .get("rookie_belonging")
                .map(String::as_str),
            Some("rookie.first_talk")
        );
        assert_eq!(progress.choice_history.len(), 1);
        assert!(
            apply_choice(
                &mut world,
                &mut journal,
                &mut progress,
                ChoiceContext {
                    scene,
                    choice: &choice,
                    player_id: pid,
                    actor_id: Some(aid),
                    month_index: 100,
                    date: "d"
                }
            )
            .is_err()
        );
        assert_eq!(progress.choice_history.len(), 1);
    }
}

/// 测试辅助：最小可玩世界（仅测试使用）。
#[cfg(test)]
mod test_support {
    use csc_entities::world::World;
    use csc_util::rng::Xoshiro256StarStar;

    pub fn dummy_world() -> World {
        let mut w = World::new();
        let tid = w.create_team("Alpha", 1, 2000);
        let pid = w.create_player(
            csc_domain::tier::Tier::Tier4,
            Some("MyPlayer"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        w.assign_player_to_team(pid, tid).expect("入队");
        w
    }
}
