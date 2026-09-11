use super::{err, game_of};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

// —— 主动操作（FIFA 生涯模式：玩家选择事件，而不是等事件发生再选） ——

/// 当前挂起的转会窗市场（去弹窗契约）：提取 pending 批次中主角的
/// `DecisionPoint::TransferWindow` 报价，供市场页【提示】+ 渲染。
/// 无挂起报价 → `{ "market": null }`（前端显示空态/隐藏提示）。
pub(super) async fn get_transfer_market(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let market = tokio::task::spawn_blocking(move || entry.arc().transfer_market()).await;
    match market {
        Ok(Some(value)) => Json(serde_json::json!({ "market": value })).into_response(),
        Ok(None) => Json(serde_json::json!({ "market": null })).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

/// 查询当前可签约候选（合同到期 / 买断后才有）。
pub(super) async fn get_transfer_offers(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    match tokio::task::spawn_blocking(move || entry.arc().transfer_offers()).await {
        Ok(Ok(offers)) => Json(serde_json::json!({ "offers": offers })).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

#[derive(Debug, Deserialize)]
pub struct SignTransferRequest {
    pub team_signature: String,
}

pub(super) async fn sign_transfer(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<SignTransferRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.advancing.load(std::sync::atomic::Ordering::SeqCst) {
        return err(StatusCode::CONFLICT, "本局正在推进中——推进完成后再转会");
    }
    let result =
        tokio::task::spawn_blocking(move || entry.arc().sign_transfer(req.team_signature)).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}
