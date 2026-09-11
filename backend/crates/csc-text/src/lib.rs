//! # csc-text —— 文案/叙事文本包（结构拆分：硬编码文本外置为配置）
//!
//! 设计：
//! - **单一事实源**：`assets/text/zh-CN.json`（仓库根）是全部文案的编辑入口，
//!   经 `include_str!` **编译期嵌入**为默认包——核心保持零 IO（含 wasm32），
//!   无配置时行为与硬编码时代完全一致（现有测试即回归护栏）；
//! - **运行时覆盖**：调用方（server/CLI）可传入覆盖 JSON（切语言/调文案），
//!   按键合并（`HashMap::extend`），缺键回退默认包；
//! - **缺键不 panic**：`get`/`format` 对缺失键回退到键名本身——文案缺项
//!   表现为可见的键名，便于发现缺失，不炸引擎。
//!
//! 键约定：`模块.场景.变体`（如 `narrator.training_done.mine.title`）；
//! 占位符 `{0}`、`{1}`…按位置替换（见 [`TextBundle::format`]）。
//!
//! 确定性契约：文案是纯展示数据，不进入任何决策/RNG 路径；从代码搬入 JSON
//! 不改变任何模拟输出（存档内 journal 文本内容不变）。

use std::collections::HashMap;

use serde::Deserialize;

use csc_util::SimError;

/// 默认中文包（编译期嵌入；与 `assets/text/zh-CN.json` 同步——资产是编辑入口，
/// 嵌入值是零配置回退。修改文案请改资产文件，随后本常量随编译更新）。
const DEFAULT_ZH: &str = include_str!("../../../../assets/text/zh-CN.json");

#[derive(Debug, Deserialize)]
struct BundleFile {
    text: HashMap<String, String>,
    /// 随机文案池（每日直播等按确定性 RNG 下标取一条）
    #[serde(default)]
    lists: HashMap<String, Vec<String>>,
}

/// 文案包：按键取词 + 占位符格式化。
#[derive(Debug, Clone)]
pub struct TextBundle {
    entries: HashMap<String, String>,
    lists: HashMap<String, Vec<String>>,
}

impl Default for TextBundle {
    fn default() -> Self {
        Self::parse(None).expect("内嵌默认文案包必然合法")
    }
}

impl TextBundle {
    /// 构建文案包：默认包 + 可选覆盖 JSON（覆盖包可只写要改的键）。
    /// 解析失败返回 `Err(SimError::Asset)`（外部输入边界 D6：坏资产显式报错）。
    pub fn parse(overrides: Option<&str>) -> Result<Self, SimError> {
        let file: BundleFile = serde_json::from_str(DEFAULT_ZH).map_err(|e| {
            SimError::asset("assets/text/zh-CN.json", format!("默认文案包解析失败：{e}"))
        })?;
        let mut entries = file.text;
        let mut lists = file.lists;
        if let Some(json) = overrides {
            let file: BundleFile = serde_json::from_str(json).map_err(|e| {
                SimError::asset("text overrides", format!("文案覆盖包解析失败：{e}"))
            })?;
            entries.extend(file.text);
            lists.extend(file.lists);
        }
        Ok(Self { entries, lists })
    }

    /// 取词（缺键回退到键名本身——文案缺项不 panic，便于发现缺失）。
    ///
    /// 生命周期说明：返回引用受 `self` 与 `key` 中较短者约束（缺键时返回键本身，
    /// 因此不能只绑 `self`）。键通常是字面量，调用点无感知。
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.entries.get(key).map(String::as_str).unwrap_or(key)
    }

    /// 无回退的取词（键与返回生命周期解耦；缺键返回 None）。
    pub fn get_opt(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// 取词 + 位置占位符替换（`{0}` `{1}` …；缺键同样回退键名）。
    pub fn format(&self, key: &str, args: &[&str]) -> String {
        let mut out = self.get(key).to_string();
        for (i, arg) in args.iter().enumerate() {
            out = out.replace(&format!("{{{i}}}"), arg);
        }
        out
    }

    /// 取随机池中的一条（`idx` 由调用方从确定性 RNG 获得）；池缺键/越界回退键名。
    pub fn pick<'a>(&'a self, key: &'a str, idx: usize) -> &'a str {
        self.lists
            .get(key)
            .and_then(|l| l.get(idx))
            .map(String::as_str)
            .unwrap_or(key)
    }

    /// 取随机池中的一条并做占位符替换。
    pub fn pick_format(&self, key: &str, idx: usize, args: &[&str]) -> String {
        let mut out = self.pick(key, idx).to_string();
        for (i, arg) in args.iter().enumerate() {
            out = out.replace(&format!("{{{i}}}"), arg);
        }
        out
    }

    /// 文案条目数（诊断用）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空（诊断用）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bundle_parses_and_has_keys() {
        let b = TextBundle::default();
        assert!(b.len() > 100, "默认文案包应有足够条目：{}", b.len());
        assert!(!b.get("narrator.match.win.title").starts_with("narrator."));
    }

    #[test]
    fn overrides_merge_and_missing_keys_fallback() {
        let b = TextBundle::parse(Some(r#"{"text": {"test.custom": "自定义 {0} 文案"}}"#)).unwrap();
        // 覆盖键生效
        assert_eq!(b.format("test.custom", &["A"]), "自定义 A 文案");
        // 默认键仍可用
        assert!(!b.get("narrator.match.win.title").starts_with("narrator."));
        // 缺键回退键名（不 panic）
        assert_eq!(b.get("no.such.key"), "no.such.key");
    }

    #[test]
    fn positional_placeholders() {
        let b = TextBundle::parse(Some(
            r#"{"text": {"test.positional": "第 {0} 月，{1} 场"}}"#,
        ))
        .unwrap();
        let out = b.format("test.positional", &["3", "25"]);
        assert_eq!(out, "第 3 月，25 场");
        // 缺键回退键名（不 panic、不替换）
        assert_eq!(b.format("no.such.key", &["x"]), "no.such.key");
    }

    #[test]
    fn bad_override_rejected() {
        assert!(TextBundle::parse(Some("{bad json")).is_err());
    }
}
