//! 存档辅助：待决策转会市场提取 / 磁盘包封装 / GameLoader 配置。

use std::fs;

use flate2::Compression;
use flate2::write::GzEncoder;

use csc_core::state::GameState;
use csc_decision::point::DecisionPoint;

use super::types::{PendingBatch, PersistedGame, Policy, canonical_state_bytes};

/// 从待决策批次提取转会窗市场（纯函数，便于单测）。
/// 仅取 `DecisionPoint::TransferWindow` 的 offers（其余决策点被过滤）；
/// 无任何转会窗 offer → `None`。
pub(super) fn transfer_market_from_pending(
    pending: &Option<PendingBatch>,
) -> Option<serde_json::Value> {
    let batch = pending.as_ref()?;
    let batch_date = batch.date.clone().unwrap_or_default();
    let deadline = csc_time::civil::month_end_label(&batch_date);
    let offers: Vec<csc_decision::offer::TransferOffer> = batch
        .points
        .iter()
        .filter_map(|p| match p {
            DecisionPoint::TransferWindow { offers, .. } => Some(offers.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    if offers.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "date": batch_date,
        "deadline": deadline,
        "offers": offers,
    }))
}

/// 恢复被卸载游戏所需的配置资产（当前只有新秀实力分布）。
#[derive(Debug, Clone, Default)]
pub struct GameLoader {
    pub rating_profile: Option<String>,
    pub narrative: Option<csc_core::narrative::NarrativeContent>,
}

impl GameLoader {
    /// 从 AssetsBundle 构建（由 server/state.rs 调用）。
    pub fn new(rating_profile: Option<String>) -> Self {
        Self {
            rating_profile,
            narrative: None,
        }
    }
}
/// 用最新快照构建指定版本的持久化 envelope（带 CRC）。
pub(super) fn build_persisted_envelope(
    policy: Policy,
    story: bool,
    snapshot: GameState,
    format_version: u32,
) -> Result<PersistedGame, String> {
    let state_json = canonical_state_bytes(&snapshot)?;
    Ok(PersistedGame {
        format: "csc-persisted-game".into(),
        format_version,
        policy,
        story,
        career_session: super::career::CareerSession::default(),
        state_crc32: crc32fast::hash(&state_json),
        state: snapshot,
    })
}

/// gzip 压缩并原子写盘（先写 tmp 再 rename，保证并发恢复/淘汰下不会读到半成品包）。
pub(super) fn write_persisted_package(
    path: &std::path::Path,
    envelope: &PersistedGame,
) -> Result<(), String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut encoder, envelope).map_err(|e| format!("序列化失败：{e}"))?;
    let bytes = encoder.finish().map_err(|e| format!("压缩失败：{e}"))?;
    let dir = path.parent().ok_or("持久化路径非法")?;
    fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("写盘失败：{e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("原子替换失败：{e}"))?;
    Ok(())
}
