//! 应用状态：外部资产（standings/roles_baseline）+ 游戏管理器。
//!
//! 资产读取由 server 层负责（`csc-core` 保持零 IO）——启动时把
//! `assets/` 目录内容读入内存（`AssetsBundle`），新游戏从内存装载。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::game::{GameLoader, GameManager};
use crate::live::LiveSessionStore;

/// 内存化资产包：standings 文件名 → 内容 + roles_baseline（可选）
/// + 真实 HLTV TOP20 三年榜单（可选，用于同榜「传奇参照」）
/// + **真实评级先验**（可选，player_ratings.json——校准真实选手档位）
/// + **新选手实力分布模型**（可选，rating_profile.json——校准虚拟新秀档位）
/// + **叙事文案覆盖包**（可选，text/zh-CN.json——覆盖编译期嵌入的默认文案）。
#[derive(Debug, Clone, Default)]
pub struct AssetsBundle {
    /// standings JSON（文件名 → 内容；文件名决定月份锚定，见 `VrsDatabase::month_of`）
    pub standings: Vec<(String, String)>,
    /// roles_baseline.json 内容（可选——缺失时全部按阵容槽位分配位置）
    pub baseline: Option<String>,
    /// 真实 HLTV TOP20 三年索引（top20-data/*.json；纯域模型由 csc-domain 提供）
    pub real_top20: csc_domain::RealTop20Index,
    /// player_ratings.json 内容（可选——真实选手 Rating 先验，校准虚拟档位）
    pub ratings: Option<String>,
    /// rating_profile.json 内容（可选——新选手实力分布，校准新秀起始档位）
    pub rating_profile: Option<String>,
    /// text/zh-CN.json 内容（可选——叙事文案覆盖包；None = 默认文案）
    pub text: Option<String>,
    pub narrative: Option<csc_core::narrative::NarrativeContent>,
}

impl AssetsBundle {
    /// 从目录装载（零语义校验——语义校验在 `VrsDatabase::parse_json_checked` 与
    /// `RealTop20Year::from_json_str`，新服务启动时执行并显式报错）。
    pub fn load_from_dir(dir: &Path) -> Result<Self, String> {
        let mut standings = Vec::new();
        let mut baseline = None;
        let mut ratings = None;
        let mut rating_profile = None;
        let mut text = None;
        let mut real_top20 = csc_domain::RealTop20Index::default();

        // 顶层 standings / roles_baseline / player_ratings / rating_profile
        for entry in std::fs::read_dir(dir).map_err(|e| format!("读取资产目录失败 {dir:?}：{e}"))?
        {
            let path = entry.map_err(|e| format!("遍历资产目录失败：{e}"))?.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with("standings_") && name.ends_with(".json") {
                let content =
                    std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
                verify_json_structure(
                    &name,
                    &content,
                    "standings_*.json（顶层对象 + rankings[]）",
                )?;
                standings.push((name, content));
            } else if name == "roles_baseline.json" {
                let content =
                    std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
                verify_json_structure(
                    &name,
                    &content,
                    "roles_baseline.json（顶层对象 + players[]）",
                )?;
                baseline = Some(content);
            } else if name == "player_ratings.json" {
                let content =
                    std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
                verify_json_structure(
                    &name,
                    &content,
                    "player_ratings.json（顶层对象 + players[]）",
                )?;
                ratings = Some(content);
            } else if name == "rating_profile.json" {
                let content =
                    std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
                verify_json_structure(&name, &content, "rating_profile.json（顶层对象）")?;
                rating_profile = Some(content);
            }
        }

        // 子目录 text/（zh-CN.json——叙事文案覆盖包）
        let text_dir = dir.join("text");
        let text_file = text_dir.join("zh-CN.json");
        if text_file.is_file() {
            let content =
                std::fs::read_to_string(&text_file).map_err(|e| format!("读取文案包失败：{e}"))?;
            verify_json_structure("text/zh-CN.json", &content, "text/zh-CN.json（顶层对象）")?;
            text = Some(content);
        }

        // 子目录 top20-data/（hltv_top20_{year}.json）
        let top20_dir = dir.join("top20-data");
        if top20_dir.is_dir() {
            for entry in std::fs::read_dir(&top20_dir)
                .map_err(|e| format!("读取 TOP20 资产目录失败 {top20_dir:?}：{e}"))?
            {
                let path = entry
                    .map_err(|e| format!("遍历 TOP20 资产目录失败：{e}"))?
                    .path();
                if !path.is_file() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                // 命名约定 hltv_top20_{year}.json
                let Some(year_str) = name
                    .strip_prefix("hltv_top20_")
                    .and_then(|s| s.strip_suffix(".json"))
                else {
                    continue;
                };
                let Ok(year) = year_str.parse::<i32>() else {
                    continue;
                };
                let content =
                    std::fs::read_to_string(&path).map_err(|e| format!("读取 {name} 失败：{e}"))?;
                let parsed = csc_domain::RealTop20Year::from_json_str(year, &content)
                    .map_err(|e| format!("解析 {name} 失败：{e}"))?;
                real_top20.push(parsed);
            }
        }

        let narrative_path = dir.join("narrative/zh-CN/career.json");
        let narrative = if narrative_path.exists() {
            let json = std::fs::read_to_string(&narrative_path)
                .map_err(|e| format!("读取剧情失败：{e}"))?;
            Some(
                csc_core::narrative::NarrativeContent::from_json_str(&json)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        Ok(Self {
            standings,
            baseline,
            real_top20,
            ratings,
            rating_profile,
            text,
            narrative,
        })
    }
}

/// 启动期资产结构检查（P0-②）：JSON 可解析 + 顶层结构最小集。
///
/// 浅校验（类型 + 必填容器）；语义校验（数值区间、跨资产一致性）由消费点解析器兜底
/// （`VrsDatabase::parse_json_checked`、`RealTop20Year::from_json_str`、
/// `RatingProfile::from_json_str`、`RoleBaseline::from_json_str`），工具侧见
/// `tools/verify_assets.mjs`。坏资产在启动时显式报错，而非延迟到创建游戏时。
fn verify_json_structure(name: &str, content: &str, expect: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("资产 {name} JSON 解析失败（期望 {expect}）：{e}"))?;
    let ok = match value {
        serde_json::Value::Object(_) => {
            if name.starts_with("standings_") {
                value
                    .get("rankings")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            } else if name == "roles_baseline.json" || name == "player_ratings.json" {
                value
                    .get("players")
                    .and_then(|v| v.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            } else {
                true // rating_profile.json / text/zh-CN.json：顶层对象即可，容器细节由消费点校验
            }
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("资产 {name} 结构非法（期望 {expect}）"))
    }
}

/// 应用状态（axum `State` 注入）。
#[derive(Clone)]
pub struct AppState {
    /// 内存化资产包（新游戏装载源）
    pub assets: Arc<AssetsBundle>,
    /// 游戏管理器（每局一个模拟线程）
    pub games: Arc<GameManager>,
    /// 叙事文案包（默认 = 编译期嵌入；assets/text/zh-CN.json 存在时按键覆盖）
    pub text: csc_text::TextBundle,
    /// 进行中 LIVE 地图会话（按 game_id/match_id 隔离）。
    pub live_sessions: LiveSessionStore,
}

impl AppState {
    /// 从资产目录构造（启动时一次）。
    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let assets = AssetsBundle::load_from_dir(dir)?;
        if assets.standings.is_empty() {
            return Err(format!("资产目录 {dir:?} 中没有 standings_*.json"));
        }
        // 持久化目录（2026 架构优化：存档迁出资产树）。
        //  主目录（写入） = CSC_RUNTIME_DIR 环境变量，缺省回退到 OS 临时目录（仓库外）；
        //  兼容读回退 = `backend/runtime/games`（历史存档位置，找不到主目录存档时回退读取）。
        let persist_dir = csc_runtime_dir();
        std::fs::create_dir_all(&persist_dir)
            .map_err(|e| format!("创建持久化目录失败 {persist_dir:?}：{e}"))?;
        let legacy_read_dir = csc_legacy_runtime_dir(dir);
        let loader = Arc::new(GameLoader {
            rating_profile: assets.rating_profile.clone(),
            narrative: assets.narrative.clone(),
        });
        // LIVE 会话仓库先建，注入 GameManager——LRU 淘汰 / shutdown 时据此回收该 game_id
        // 的 LIVE 会话（2026-09 修复 P0-2：会话只增不删的进程级泄漏）。
        let live_sessions = LiveSessionStore::default();
        let games = Arc::new(
            GameManager::with_config(Some(persist_dir), Some(4), Some(loader))
                .with_persist_fallback(Some(legacy_read_dir))
                .with_live_sessions(live_sessions.clone()),
        );
        let text = csc_text::TextBundle::parse(assets.text.as_deref())
            .map_err(|e| format!("文案包解析失败：{e}"))?;
        Ok(Self {
            assets: Arc::new(assets),
            games,
            text,
            live_sessions,
        })
    }

    /// 测试/嵌入式：从内存资产构造。
    pub fn from_bundle(assets: AssetsBundle) -> Self {
        Self {
            assets: Arc::new(assets),
            games: Arc::new(GameManager::new()),
            text: csc_text::TextBundle::default(),
            live_sessions: LiveSessionStore::default(),
        }
    }
}

/// 便捷：内存资产构造（测试用）。
pub fn bundle_of(standings: HashMap<String, String>, baseline: Option<String>) -> AssetsBundle {
    let mut v: Vec<(String, String)> = standings.into_iter().collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    AssetsBundle {
        standings: v,
        baseline,
        real_top20: csc_domain::RealTop20Index::default(),
        ratings: None,
        rating_profile: None,
        text: None,
        narrative: None,
    }
}

/// 主持久化目录解析（2026 架构优化：存档迁出资产树）。
///
/// 优先级：
/// 1. `CSC_RUNTIME_DIR` 环境变量（显式指定，可指向仓库外任何位置）；
/// 2. 缺省 → 系统临时目录下的 `csc-simulation/games`（仓库外，避免污染资产树）。
///
/// 返回最终「games 存档目录」（写入路径）。
fn csc_runtime_dir() -> std::path::PathBuf {
    if let Some(v) = std::env::var("CSC_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Path::new(&v).join("games")
    } else {
        std::env::temp_dir().join("csc-simulation").join("games")
    }
}

/// 兼容读回退目录（历史存档位置）：`backend/runtime/games`。
///
/// `dir` 为资产目录（如 `assets` 或 `backend/assets`），其父级即仓库/运行根，
/// 其下 `runtime/games` 为历史存档写入位置。若该目录不存在则退回到
/// `dir.join("games")`（最旧代码的写入位置），保证旧存档尽量可读。
fn csc_legacy_runtime_dir(dir: &Path) -> std::path::PathBuf {
    let parent = dir.parent().filter(|p| !p.as_os_str().is_empty());
    let runtime_games = parent.unwrap_or(dir).join("runtime").join("games");
    if runtime_games.is_dir() {
        return runtime_games;
    }
    dir.join("games")
}

#[cfg(test)]
mod tests {
    use super::verify_json_structure;

    #[test]
    fn verify_standings_requires_nonempty_rankings() {
        assert!(
            verify_json_structure(
                "standings_x.json",
                r#"{"rankings":[{"ranking":1,"points":1,"teamName":"A","roster":["x"]}]}"#,
                "standings"
            )
            .is_ok()
        );
        assert!(
            verify_json_structure("standings_x.json", r#"{"rankings":[]}"#, "standings").is_err()
        );
        assert!(verify_json_structure("standings_x.json", r#"{"foo":1}"#, "standings").is_err());
        assert!(verify_json_structure("standings_x.json", "not json", "standings").is_err());
    }

    #[test]
    fn verify_players_files_require_nonempty_players() {
        assert!(
            verify_json_structure(
                "roles_baseline.json",
                r#"{"players":[{"player":"a","team":"t","role":"r","ctRole":"r","tRole":"r"}]}"#,
                "roles_baseline"
            )
            .is_ok()
        );
        assert!(
            verify_json_structure(
                "player_ratings.json",
                r#"{"players":[{"player":"a","rating":1.0}]}"#,
                "player_ratings"
            )
            .is_ok()
        );
        assert!(
            verify_json_structure("roles_baseline.json", r#"{"players":[]}"#, "roles_baseline")
                .is_err()
        );
        assert!(
            verify_json_structure(
                "player_ratings.json",
                r#"{"player_count":0}"#,
                "player_ratings"
            )
            .is_err()
        );
    }

    #[test]
    fn verify_object_files_accept_any_object() {
        // rating_profile / text：顶层对象即可（容器细节由消费点解析器校验）
        assert!(
            verify_json_structure("rating_profile.json", r#"{"global":{}}"#, "rating_profile")
                .is_ok()
        );
        assert!(verify_json_structure("text/zh-CN.json", r#"{"lang":"zh-CN"}"#, "text").is_ok());
        assert!(verify_json_structure("text/zh-CN.json", "[]", "text").is_err());
        assert!(verify_json_structure("rating_profile.json", "42", "rating_profile").is_err());
    }
}
