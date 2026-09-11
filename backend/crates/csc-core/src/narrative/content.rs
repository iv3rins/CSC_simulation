//! 叙事内容包 schema 与加载（23 号方案 §5）——`assets/narrative/zh-CN/*.json`。
//!
//! 边界（23 号方案 §5）：
//! - 效果是**受限 Rust enum**（[`Effect`]），不执行脚本字符串/eval/任意 JSON path；
//! - 首次加载即校验：schema、重复场景 ID、不存在的 `next_scene`、非法效果、无出口循环；
//! - 文本采用内联 `paragraphs`（中文），不依赖前端 describe 引擎。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use csc_util::SimError;

/// 剧情包（一个语言/内容文件）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeContent {
    /// 内容版本（与 GameState 格式版本、WORLD_SIM_VERSION 分开管理）。
    pub content_version: u32,
    /// 弧定义（id/title/order）。
    #[serde(default)]
    pub arcs: Vec<ArcDef>,
    /// 场景定义。
    #[serde(default)]
    pub scenes: Vec<SceneDef>,
}

/// 剧情弧定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub order: i32,
}

/// 场景优先级（Director 选择顺序：Critical > Important > Normal）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePriority {
    Normal,
    Important,
    Critical,
}

/// 场景触发条件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    /// 弧入口（该弧尚无任何已见场景时触发）。
    ArcEntry { arc: String },
    /// 完成至少 `min_series` 场正式系列赛后。
    AfterSeries {
        #[serde(default)]
        min_series: i32,
    },
    /// 至少有 `min_losses` 场失利且完成 `min_series` 场后。
    AfterLoss {
        #[serde(default)]
        min_losses: i32,
        #[serde(default)]
        min_series: i32,
    },
    /// 某场景已完成后（直接续接）。
    AfterScene { after_scene: String },
    /// 指定承诺到期（待兑现）。
    PromiseDue { promise: String },
    /// 某场景完成后延迟 N 个月。
    DelayMonths { after_scene: String, months: i32 },
    /// 存在真实招募接触且完成 `min_series` 场后。
    HasTransferContact {
        #[serde(default)]
        min_series: i32,
    },
    /// 首次明显失利（首次出现失利时）。
    FirstLoss,
}

/// 一个场景定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDef {
    pub id: String,
    pub arc: String,
    #[serde(default)]
    pub chapter_title: String,
    pub title: String,
    #[serde(default)]
    pub location: String,
    #[serde(default = "default_priority")]
    pub priority: ScenePriority,
    /// 演员角色（captain/teammate/coach/manager/media/self）。
    #[serde(default)]
    pub actor_role: String,
    pub trigger: Trigger,
    /// 任一 flag 满足才可触发（空 = 无条件）。
    #[serde(default)]
    pub requires_any_flag: Vec<String>,
    /// 正文段落（中文，2–4 段）。
    #[serde(default)]
    pub paragraphs: Vec<String>,
    /// 选项（1–3 个）。
    pub choices: Vec<ChoiceDef>,
}

fn default_priority() -> ScenePriority {
    ScenePriority::Normal
}

/// 一个选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceDef {
    pub id: String,
    pub label: String,
    /// 确定影响（文案；后端保证不撒谎）。
    #[serde(default)]
    pub impact_certain: String,
    /// 可能影响（悬念文案）。
    #[serde(default)]
    pub impact_possible: String,
    /// 受限效果列表。
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// 下一场景（None = 本弧在本选择处收束）。
    #[serde(default)]
    pub next_scene: Option<String>,
}

/// 受限效果（Rust enum；不做任意世界写入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    /// 创建承诺（解析为具体 due_month_index 与可检查 key）。
    CreatePromise {
        promise: String,
        #[serde(default = "default_due_months")]
        due_months: i32,
    },
    /// 兑现承诺。
    FulfillPromise { promise: String },
    /// 承诺失信。
    BreakPromise { promise: String },
    /// 设置剧情 flag。
    SetStoryFlag { flag: String },
    /// 主体与场景主演员的关系变化。
    RelationDelta { amount: f64 },
    /// 队伍凝聚力变化。
    TeamCohesionDelta { delta: f64 },
    /// 主体士气变化。
    MoraleDelta { delta: i32 },
    /// 主体声誉变化。
    ReputationDelta { delta: i32 },
    /// 记录弧终态。
    MarkTerminal { state: String },
    /// 保留旧 schema 值；执行会明确拒绝，正式签约必须走 TransferEngine。
    AcceptTransfer,
    /// 明确留队（提升队伍关系）。
    StayTeam,
}

fn default_due_months() -> i32 {
    2
}

impl NarrativeContent {
    /// 从 JSON 加载并**完整校验**（schema/重复 ID/悬空 next_scene/非法效果/无出口循环）。
    pub fn from_json_str(json: &str) -> Result<Self, SimError> {
        let content: NarrativeContent = serde_json::from_str(json)
            .map_err(|e| SimError::asset("<narrative>", format!("剧情包解析失败：{e}")))?;
        content.validate()?;
        Ok(content)
    }

    /// 结构校验（加载即调用；失败返回资产错误）。
    pub fn validate(&self) -> Result<(), SimError> {
        let fail = |message: String| SimError::asset("<narrative>", message);
        let mut arcs = HashSet::new();
        for arc in &self.arcs {
            if arc.id.trim().is_empty() || !arcs.insert(arc.id.as_str()) {
                return Err(fail(format!("重复或空弧 ID：{}", arc.id)));
            }
        }
        let mut ids = HashSet::new();
        let produced: HashSet<&str> = self
            .scenes
            .iter()
            .flat_map(|s| &s.choices)
            .flat_map(|c| &c.effects)
            .filter_map(|e| {
                if let Effect::CreatePromise { promise, .. } = e {
                    Some(promise.as_str())
                } else {
                    None
                }
            })
            .collect();
        for scene in &self.scenes {
            if scene.id.trim().is_empty() || !ids.insert(scene.id.as_str()) {
                return Err(fail(format!("重复场景 ID：{}", scene.id)));
            }
            if !arcs.contains(scene.arc.as_str()) {
                return Err(fail(format!(
                    "场景 {} 引用未定义弧 {}",
                    scene.id, scene.arc
                )));
            }
            if scene.choices.is_empty() {
                return Err(fail(format!("场景 {} 无任何选项", scene.id)));
            }
            match &scene.trigger {
                Trigger::ArcEntry { arc } if arc != &scene.arc => {
                    return Err(fail(format!("场景 {} 的入口弧不一致", scene.id)));
                }
                Trigger::AfterScene { after_scene } | Trigger::DelayMonths { after_scene, .. }
                    if !self
                        .scenes
                        .iter()
                        .any(|s| &s.id == after_scene && s.arc == scene.arc) =>
                {
                    return Err(fail(format!(
                        "场景 {} 悬空或跨弧 after_scene {after_scene}",
                        scene.id
                    )));
                }
                Trigger::PromiseDue { promise } if !produced.contains(promise.as_str()) => {
                    return Err(fail(format!("场景 {} 未定义承诺 {promise}", scene.id)));
                }
                _ => {}
            }
            match scene.trigger {
                Trigger::DelayMonths { months, .. } if months <= 0 => {
                    return Err(fail(format!("场景 {} 延迟必须为正", scene.id)));
                }
                Trigger::AfterSeries { min_series }
                | Trigger::HasTransferContact { min_series }
                    if min_series < 0 =>
                {
                    return Err(fail(format!("场景 {} 场次非法", scene.id)));
                }
                Trigger::AfterLoss {
                    min_losses,
                    min_series,
                } if min_losses < 0 || min_series < 0 => {
                    return Err(fail(format!("场景 {} 失利条件非法", scene.id)));
                }
                _ => {}
            }
            let mut choices = HashSet::new();
            for choice in &scene.choices {
                let context = format!("场景 {} / 选项 {}", scene.id, choice.id);
                if choice.id.trim().is_empty() || !choices.insert(&choice.id) {
                    return Err(fail(format!("{context} 重复或空选项 ID")));
                }
                if let Some(next) = &choice.next_scene
                    && !self
                        .scenes
                        .iter()
                        .any(|s| &s.id == next && s.arc == scene.arc)
                {
                    return Err(fail(format!("{context} 悬空或跨弧 next_scene {next}")));
                }
                for effect in &choice.effects {
                    match effect {
                        Effect::CreatePromise {
                            promise,
                            due_months,
                        } if promise.trim().is_empty() || *due_months <= 0 => {
                            return Err(fail(format!("{context} 非法承诺参数")));
                        }
                        Effect::FulfillPromise { promise } | Effect::BreakPromise { promise }
                            if !produced.contains(promise.as_str()) =>
                        {
                            return Err(fail(format!("{context} 未定义承诺 {promise}")));
                        }
                        Effect::RelationDelta { amount }
                            if !amount.is_finite() || amount.abs() > 100.0 =>
                        {
                            return Err(fail(format!("{context} 非法关系增量")));
                        }
                        Effect::TeamCohesionDelta { delta }
                            if !delta.is_finite() || delta.abs() > 100.0 =>
                        {
                            return Err(fail(format!("{context} 非法凝聚力增量")));
                        }
                        Effect::SetStoryFlag { flag } if flag.trim().is_empty() => {
                            return Err(fail(format!("{context} 空 flag")));
                        }
                        Effect::MarkTerminal { state } if state.trim().is_empty() => {
                            return Err(fail(format!("{context} 空终态")));
                        }
                        _ => {}
                    }
                }
            }
        }
        // A scene can execute only once. Even a cycle with an optional exit can strand a
        // player choosing its back edge, so reject every cycle, not merely closed SCCs.
        fn visit<'a>(
            id: &'a str,
            content: &'a NarrativeContent,
            active: &mut HashSet<&'a str>,
            done: &mut HashSet<&'a str>,
        ) -> Result<(), SimError> {
            if done.contains(id) {
                return Ok(());
            }
            if !active.insert(id) {
                return Err(SimError::asset(
                    "<narrative>",
                    format!("无出口循环或回访已消费场景：{id}"),
                ));
            }
            if let Some(scene) = content.scene(id) {
                for next in scene.choices.iter().filter_map(|c| c.next_scene.as_deref()) {
                    visit(next, content, active, done)?;
                }
            }
            active.remove(id);
            done.insert(id);
            Ok(())
        }
        let mut done = HashSet::new();
        for scene in &self.scenes {
            visit(&scene.id, self, &mut HashSet::new(), &mut done)?;
        }
        let incoming: HashSet<&str> = self
            .scenes
            .iter()
            .flat_map(|s| &s.choices)
            .filter_map(|c| c.next_scene.as_deref())
            .collect();
        for scene in &self.scenes {
            if !incoming.contains(scene.id.as_str())
                && matches!(
                    scene.trigger,
                    Trigger::AfterScene { .. }
                        | Trigger::DelayMonths { .. }
                        | Trigger::PromiseDue { .. }
                )
            {
                return Err(fail(format!("场景 {} 需要前序却没有入边", scene.id)));
            }
        }
        Ok(())
    }

    /// 按 id 查场景。
    pub fn scene(&self, id: &str) -> Option<&SceneDef> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

impl Default for NarrativeContent {
    /// 空内容（无剧情；旧档/测试路径），加载失败时不阻塞引擎。
    fn default() -> Self {
        Self {
            content_version: 1,
            arcs: Vec::new(),
            scenes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "content_version": 1,
      "arcs": [{"id": "a", "title": "A", "order": 1}],
      "scenes": [{
        "id": "a.one", "arc": "a", "title": "T",
        "trigger": {"kind": "arc_entry", "arc": "a"},
        "paragraphs": ["p"],
        "choices": [
          {"id": "X", "label": "l", "effects": [{"kind": "set_story_flag", "flag": "f"}], "next_scene": null},
          {"id": "Y", "label": "m", "effects": [], "next_scene": null}
        ]
      }]
    }"#;

    #[test]
    fn loads_and_validates_sample() {
        let c = NarrativeContent::from_json_str(SAMPLE).expect("样例应合法");
        assert_eq!(c.scenes.len(), 1);
        assert_eq!(c.scene("a.one").unwrap().choices.len(), 2);
    }

    #[test]
    fn rejects_duplicate_id() {
        // 直接构造两个同 id 场景。
        let bad = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],
          "scenes":[
            {"id":"d","arc":"a","title":"1","trigger":{"kind":"arc_entry","arc":"a"},"choices":[{"id":"X","label":"l"}]},
            {"id":"d","arc":"a","title":"2","trigger":{"kind":"arc_entry","arc":"a"},"choices":[{"id":"X","label":"l"}]}
          ]}"#;
        let err = NarrativeContent::from_json_str(bad).unwrap_err();
        assert!(err.to_string().contains("重复场景 ID"));
    }

    #[test]
    fn rejects_dangling_next_scene() {
        let bad = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],
          "scenes":[{"id":"d","arc":"a","title":"1","trigger":{"kind":"arc_entry","arc":"a"},
          "choices":[{"id":"X","label":"l","next_scene":"ghost"}]}]}"#;
        let err = NarrativeContent::from_json_str(bad).unwrap_err();
        assert!(err.to_string().contains("next_scene"));
    }

    #[test]
    fn rejects_scene_without_choices() {
        let bad = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],
          "scenes":[{"id":"d","arc":"a","title":"1","trigger":{"kind":"arc_entry","arc":"a"},"choices":[]}]}"#;
        let err = NarrativeContent::from_json_str(bad).unwrap_err();
        assert!(err.to_string().contains("无任何选项"));
    }
    #[test]
    fn rejects_duplicate_choices_and_cycles_and_ghost_triggers() {
        let content = NarrativeContent::from_json_str(SAMPLE).unwrap();
        let mut duplicate = content.clone();
        duplicate.scenes[0].choices[1].id = "X".into();
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("选项 ID")
        );
        let mut cycle = content.clone();
        cycle.scenes[0].choices[0].next_scene = Some("a.one".into());
        assert!(cycle.validate().unwrap_err().to_string().contains("循环"));
        let mut ghost = content;
        ghost.scenes[0].trigger = Trigger::AfterScene {
            after_scene: "ghost".into(),
        };
        assert!(
            ghost
                .validate()
                .unwrap_err()
                .to_string()
                .contains("after_scene")
        );
    }

    #[test]
    fn shipping_content_has_thirteen_valid_scenes() {
        let content = NarrativeContent::from_json_str(include_str!(
            "../../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap();
        assert_eq!(content.content_version, 2);
        assert_eq!(content.scenes.len(), 13);
        assert_eq!(content.arcs.len(), 3);
        assert!(
            !content
                .scenes
                .iter()
                .flat_map(|s| &s.choices)
                .flat_map(|c| &c.effects)
                .any(|e| matches!(e, Effect::AcceptTransfer))
        );
    }
}
