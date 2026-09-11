//! Audit-only counterexamples against the current public APIs; not production tests.
use csc_core::narrative::{NarrativeContent, NarrativeFacts, director::Director};
use csc_entities::narrative::NarrativeProgress;

fn main() {
    let future = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],"scenes":[{"id":"a.due","arc":"a","title":"due","trigger":{"kind":"promise_due","promise":"p"},"choices":[{"id":"OK","label":"ok"}]}]}"#;
    let content = NarrativeContent::from_json_str(future).unwrap();
    let mut progress = NarrativeProgress::default();
    progress.add_promise("p", 999, "a.start");
    let facts = NarrativeFacts { month_index: 10, ..Default::default() };
    let selected = Director::select(&content, &progress, &facts).map(|s| s.id);
    println!("future_promise: now=10 due=999 selected={selected:?}; expected=None");

    let cycle = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],"scenes":[{"id":"a.loop","arc":"a","title":"loop","trigger":{"kind":"arc_entry","arc":"a"},"choices":[{"id":"X","label":"x","next_scene":"a.loop"}]}]}"#;
    println!("no_exit_self_cycle_accepted={}; expected=false", NarrativeContent::from_json_str(cycle).is_ok());
    let duplicate = future.replace(r#"[{"id":"OK","label":"ok"}]"#, r#"[{"id":"OK","label":"ok"},{"id":"OK","label":"different"}]"#);
    println!("duplicate_choice_id_accepted={}; expected=false", NarrativeContent::from_json_str(&duplicate).is_ok());
    let dangling = future.replace(r#"{"kind":"promise_due","promise":"p"}"#, r#"{"kind":"after_scene","after_scene":"ghost"}"#);
    println!("dangling_after_scene_accepted={}; expected=false", NarrativeContent::from_json_str(&dangling).is_ok());

    let actual = NarrativeContent::from_json_str(include_str!("../../assets/narrative/zh-CN/career.json")).unwrap();
    let mut branch = NarrativeProgress::default();
    branch.mark_seen("rookie.first_talk#2026-01-01");
    branch.set_cooldown("after:rookie.first_talk", 1);
    branch.set_flag("team_first");
    for scene in &actual.scenes {
        if scene.id != "rookie.solo_followup" {
            branch.mark_seen(&scene.id);
        }
    }
    let selected = Director::select(&actual, &branch, &NarrativeFacts { month_index: 2, ..Default::default() }).map(|s| s.id);
    println!("solo_branch_without_solo_choice_selected={selected:?}; expected=None");
    println!("actual_assets: arcs={} scenes={}", actual.arcs.len(), actual.scenes.len());
}
