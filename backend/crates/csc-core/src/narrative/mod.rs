//! 文字生涯叙事子系统（23 号方案 §5）——内容 + 导演 + 效果。
//!
//! 结构：
//! - [`content`]：`assets/narrative/zh-CN/*.json` 的 schema/加载/校验；
//! - [`director`]：`NarrativeDirector::select`——纯函数按优先级选一个场景；
//! - [`apply`]：受限效果应用（关系/凝聚力/士气/声誉/承诺/终态/转会意图）。
//!
//! **持久暂停**：[`NarrativeState::progress`] 入 `GameState.narrative`（v10），
//! 引擎重启后由 `content_version` 重新水合正文——进度与承诺连续。

pub mod apply;
pub mod content;
pub mod director;

use csc_entities::narrative::NarrativeProgress;

pub use apply::{AppliedOutcome, ChoiceContext, apply_choice};
pub use content::{ArcDef, ChoiceDef, Effect, NarrativeContent, SceneDef, ScenePriority, Trigger};
pub use director::{NarrativeFacts, instantiate};

/// 叙事运行时状态（Engine 持有；`progress` 入存档，`content` 为装配配置）。
#[derive(Debug, Clone, Default)]
pub struct NarrativeState {
    /// 内容资产（装配时从 `assets/narrative/zh-CN/*.json` 加载；不入存档）。
    pub content: NarrativeContent,
    /// 剧情进度（入 `GameState.narrative`）。
    pub progress: NarrativeProgress,
}

impl NarrativeState {
    /// 用内容构建（内容版本写入进度）。
    pub fn with_content(content: NarrativeContent) -> Self {
        let progress = NarrativeProgress {
            content_version: content.content_version,
            ..Default::default()
        };
        Self { content, progress }
    }

    /// 从资产 JSON 列表装配（加载失败 → 空内容，不阻塞引擎；错误记录在返回值）。
    pub fn from_asset_jsons(jsons: &[String]) -> Result<Self, csc_util::SimError> {
        let mut merged = NarrativeContent::default();
        for json in jsons {
            let part: NarrativeContent = serde_json::from_str(json)
                .map_err(|e| csc_util::SimError::asset("<narrative>", e.to_string()))?;
            merged.content_version = merged.content_version.max(part.content_version);
            merged.arcs.extend(part.arcs);
            merged.scenes.extend(part.scenes);
        }
        // 合并后再次校验（跨文件重复 ID/悬空引用）。
        merged.validate()?;
        Ok(Self::with_content(merged))
    }

    /// 当前活动场景对应的内容定义（若存在活动场景）。
    pub fn active_scene_def(&self) -> Option<&SceneDef> {
        let active = self.progress.active_scene.as_ref()?;
        self.content.scene(&active.scene_id)
    }

    /// 选择一个可触发场景（不改变状态；仅查询）。
    pub fn select_scene(&self, facts: &NarrativeFacts) -> Option<SceneDef> {
        director::Director::select(&self.content, &self.progress, facts)
    }

    /// 是否有进行中的场景（等待玩家选择）。
    pub fn has_active_scene(&self) -> bool {
        self.progress.active_scene.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_and_validates_multiple_assets() {
        let a = r#"{"content_version":1,"arcs":[{"id":"a","title":"A"}],
          "scenes":[{"id":"a.1","arc":"a","title":"t","trigger":{"kind":"first_loss"},
          "choices":[{"id":"X","label":"l"}]}]}"#;
        let b = r#"{"content_version":2,"arcs":[{"id":"b","title":"B"}],
          "scenes":[{"id":"b.1","arc":"b","title":"t","trigger":{"kind":"first_loss"},
          "choices":[{"id":"Y","label":"m"}]}]}"#;
        let st =
            NarrativeState::from_asset_jsons(&[a.to_string(), b.to_string()]).expect("合并合法");
        assert_eq!(st.content.scenes.len(), 2);
        assert_eq!(st.progress.content_version, 2);
    }
}
