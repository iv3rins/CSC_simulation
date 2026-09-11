use super::{err, game_of};
use crate::game::career::{CareerChoice, CareerCommand};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub(super) async fn get_career(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let session = entry.career_session.lock().expect("career session 锁");
    let snapshot = entry.snapshot.lock().expect("snapshot 锁");
    let scene = snapshot.narrative.active_scene.as_ref().and_then(|active| {
        let def = state.assets.narrative.as_ref()?.scene(&active.scene_id)?;
        Some(json!({
            "instance_id":active.instance_id,"scene_id":active.scene_id,"date":active.date,
            "actor_role":active.actor_role,"actor_name":active.actor_name,
            "title":def.title,"chapter_title":def.chapter_title,"location":def.location,
            "paragraphs":def.paragraphs,
            "choices":def.choices.iter().map(|c|json!({"id":c.id,"label":c.label,"impact_certain":c.impact_certain,"impact_possible":c.impact_possible})).collect::<Vec<_>>()
        }))
    });
    Json(json!({
        "game_id":id,"story":entry.story,"generation":session.generation,
        "status":if scene.is_some(){"awaiting_choice"}else{"idle"},
        "capabilities":{"advance_world":false},
        "limitation":"正式赛事的可恢复推进尚未完成；当前支持序章选择、资料与存档。",
        "scene":scene,"progress":snapshot.narrative,
        "pending":entry.pending.lock().expect("pending 锁").clone(),
        "view":entry.view.lock().expect("view 锁").clone()
    }))
    .into_response()
}

async fn run(state: AppState, id: u64, command: CareerCommand) -> Response {
    let handle = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // Keep the lease until the command completes; no synchronous recv on Tokio workers.
    match tokio::task::spawn_blocking(move || handle.career_command(command)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(message)) => err(StatusCode::CONFLICT, message),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "生涯命令线程异常"),
    }
}

pub(super) async fn continue_career(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    run(state, id, CareerCommand::Continue).await
}

pub(super) async fn choose_career(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(request): Json<CareerChoice>,
) -> Response {
    run(state, id, CareerCommand::Choose(request)).await
}

pub(super) async fn career_history(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let session = entry.career_session.lock().expect("career session 锁");
    Json(json!({"items":session.receipts,"next_cursor":null})).into_response()
}

pub(super) async fn career_operation(
    State(state): State<AppState>,
    Path((id, request_id)): Path<(u64, String)>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let session = entry.career_session.lock().expect("career session 锁");
    match session
        .receipts
        .iter()
        .find(|r| r.request.request_id == request_id)
    {
        Some(receipt) => Json(json!(receipt)).into_response(),
        None => err(
            StatusCode::NOT_FOUND,
            "尚无该请求的已提交回执；请使用相同身份重试",
        ),
    }
}
