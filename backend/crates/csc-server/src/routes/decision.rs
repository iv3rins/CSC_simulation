use super::{err, game_of};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use csc_decision::point::PlayerDecision;
use serde::Deserialize;

pub(super) async fn get_pending(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let pending = entry.pending.lock().expect("pending 锁").clone();
    Json(serde_json::json!({ "pending": pending })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct SubmitDecisionsRequest {
    pub decisions: Vec<PlayerDecision>,
    /// 提交针对的批次 id（对应 PendingBatch.batch_id）；缺省/不匹配 → 409
    #[serde(default)]
    pub batch_id: Option<u64>,
    /// 幂等身份（前端重试携带同一 ID；同 ID 同载荷返回原确认，异载荷拒绝）。
    #[serde(default)]
    pub request_id: Option<String>,
}

pub(super) async fn submit_decisions(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<SubmitDecisionsRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    match entry.submit_decisions(req.request_id, req.decisions, req.batch_id) {
        Ok(()) => Json(serde_json::json!({ "submitted": true })).into_response(),
        Err(e) => err(StatusCode::CONFLICT, e),
    }
}

// —— 主动训练（玩家想起来才练；不再是月度强制决策） ——

/// 训练计划选项（与旧月度训练决策点同构，改为前端主动拉取展示）。
pub(super) async fn get_training_options(
    State(_state): State<AppState>,
    Path(_id): Path<u64>,
) -> Response {
    Json(csc_core::batch::training_options()).into_response()
}

#[derive(Debug, Deserialize)]
pub struct TrainingRequest {
    /// TrainingFocus.name()：AIM / UTILITY / CLUTCH / PHYSICAL / MENTAL / COMMUNICATION / REST
    pub focus: String,
}

/// 设置本赛季主目标（产品层赛季闭环；写入存档 v6）。
#[derive(Debug, Deserialize)]
pub struct SeasonGoalRequest {
    pub goal: String,
}

pub(super) async fn set_season_goal(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<SeasonGoalRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    match entry.set_season_goal(req.goal) {
        Ok(()) => Json(serde_json::json!({ "set": true })).into_response(),
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// 排定下月训练计划：写入 `CareerInfo.pending_training`（存档字段，回放可复现），
/// 下一月推进开头应用。推进中/等待决策期间拒绝（模拟线程正忙）。
pub(super) async fn plan_training(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<TrainingRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.advancing.load(std::sync::atomic::Ordering::SeqCst) {
        return err(
            StatusCode::CONFLICT,
            "本局正在推进中（或等待决策）——完成本月后再安排训练",
        );
    }
    let focus = req.focus.trim().to_uppercase();
    let focus_resp = focus.clone();
    let entry2 = entry.arc();
    let result = tokio::task::spawn_blocking(move || entry2.plan_training(focus)).await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({
            "planned": true,
            "focus": focus_resp,
            "applies_next_month": true,
        }))
        .into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

/// 资金运用（2026 复审新增）：买断自己换队 / 投资自己 / 购买 CS 饰品。
/// 请求体：{ action: "BUYOUT"|"INVEST"|"SKIN", param?: "AIM"|"ENDURANCE"|"MENTAL"|"rare" }
/// 推进中/等待决策期间拒绝（模拟线程正忙）。
pub(super) async fn post_finance(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.advancing.load(std::sync::atomic::Ordering::SeqCst) {
        return err(
            StatusCode::CONFLICT,
            "本局正在推进中（或等待决策）——完成本月后再运用资金",
        );
    }
    let action = req
        .get("action")
        .and_then(|v| v.as_str())
        .map(|s| s.to_uppercase())
        .ok_or_else(|| "缺少 action 参数（BUYOUT/INVEST/SKIN）".to_string())
        .unwrap_or_default();
    if action.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "缺少 action 参数（BUYOUT/INVEST/SKIN）",
        );
    }
    let param = req
        .get("param")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let entry2 = entry.arc();
    let result = tokio::task::spawn_blocking(move || entry2.finance(action, param)).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

// —— 主动操作（FIFA 生涯模式：玩家选择事件，而不是等事件发生再选） ——

#[derive(Debug, Deserialize)]
pub struct StatementRequest {
    pub tone: String,
}

pub(super) async fn post_statement(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<StatementRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.advancing.load(std::sync::atomic::Ordering::SeqCst) {
        return err(StatusCode::CONFLICT, "本局正在推进中——推进完成后再发言");
    }
    let result = tokio::task::spawn_blocking(move || entry.arc().statement(req.tone)).await;
    match result {
        Ok(Ok(message)) => Json(serde_json::json!({ "message": message })).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

#[derive(Debug, Deserialize)]
pub struct MatchPlanRequest {
    pub style: String,
}

pub(super) async fn post_match_plan(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<MatchPlanRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.advancing.load(std::sync::atomic::Ordering::SeqCst) {
        return err(
            StatusCode::CONFLICT,
            "本局正在推进中——推进完成后再设置赛前 BP",
        );
    }
    let result = tokio::task::spawn_blocking(move || entry.arc().set_match_plan(req.style)).await;
    match result {
        Ok(Ok(())) => Json(serde_json::json!({ "set": true })).into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}
