use super::{err, game_of};
use crate::game::WsIn;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use csc_simulation::{LiveMapConfig, LiveRoundDecision};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

// —— LIVE 逐回合会话 ——

/// 玩家今日可 LIVE 的对阵（比赛日闸门查询）。
/// 响应 `{ "matches": LiveGateFixture[] }`；空数组 = 今天无玩家比赛可 LIVE。
pub(super) async fn get_live_today(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let matches = tokio::task::spawn_blocking(move || entry.arc().live_today()).await;
    match matches {
        Ok(matches) => Json(serde_json::json!({ "matches": matches })).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

/// 跳过**今日**比赛日 LIVE 提示（持久语义）：把主角今日 pending 对阵全部标记
/// `skipped`，使 `live/today` 与后续推进不再重复弹出提示条。**不改变比赛结算**——
/// 月末 `settle_month` 仍走确定性后台模拟回填比分（跳过的比赛照常出结果）。
/// 响应 `{ "skipped": N }`（N = 本次标记的场次数，非比赛日/无对阵时 0）。
pub(super) async fn skip_live_today(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let result = tokio::task::spawn_blocking(move || entry.arc().skip_live_today()).await;
    match result {
        Ok(Ok(skipped)) => Json(serde_json::json!({ "skipped": skipped })).into_response(),
        Ok(Err(e)) => err(StatusCode::CONFLICT, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct StartLiveRequest {
    #[serde(flatten)]
    pub config: LiveMapConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DecideLiveRequest {
    pub decision: LiveRoundDecision,
}

pub(super) async fn start_live(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<StartLiveRequest>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.start(id, req.config) {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(message) => err(StatusCode::CONFLICT, message),
    }
}

pub(super) async fn get_live(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.get(id, &match_id) {
        Some(view) => Json(view).into_response(),
        None => err(
            StatusCode::NOT_FOUND,
            format!("LIVE 比赛 {match_id} 不存在"),
        ),
    }
}

pub(super) async fn advance_live(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.advance(id, &match_id) {
        Ok(view) => Json(view).into_response(),
        Err(message) if message.contains("不存在") => err(StatusCode::NOT_FOUND, message),
        Err(message) => err(StatusCode::CONFLICT, message),
    }
}

pub(super) async fn decide_live(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
    Json(req): Json<DecideLiveRequest>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.decide(id, &match_id, req.decision) {
        Ok(view) => Json(view).into_response(),
        Err(message) if message.contains("不存在") => err(StatusCode::NOT_FOUND, message),
        Err(message) => err(StatusCode::CONFLICT, message),
    }
}

pub(super) async fn get_live_review(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.review(id, &match_id) {
        Some(review) => Json(review).into_response(),
        None => err(
            StatusCode::NOT_FOUND,
            format!("LIVE 比赛 {match_id} 不存在"),
        ),
    }
}

pub(super) async fn skip_live(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.skip(id, &match_id) {
        Ok(view) => Json(view).into_response(),
        Err(message) if message.contains("不存在") => err(StatusCode::NOT_FOUND, message),
        Err(message) => err(StatusCode::CONFLICT, message),
    }
}

/// R2：标记本场 LIVE 已看完（watched 语义，幂等）。响应 `{ "watched": true }`。
/// 看完后前端不再重复弹「观看回放」入口。
pub(super) async fn mark_live_watched(
    State(state): State<AppState>,
    Path((id, match_id)): Path<(u64, String)>,
) -> Response {
    let _game = match game_of(&state, id).await {
        Ok(game) => game,
        Err(response) => return *response,
    };
    match state.live_sessions.mark_watched(id, &match_id) {
        Ok(view) => Json(serde_json::json!({ "watched": view.watched })).into_response(),
        Err(message) if message.contains("不存在") => err(StatusCode::NOT_FOUND, message),
        Err(message) => err(StatusCode::CONFLICT, message),
    }
}

// —— WebSocket ——

pub(super) async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    ws.on_upgrade(move |socket| ws_loop(socket, entry.arc()))
}

/// WS 会话循环：事件广播 → 客户端；客户端消息（decide/advance）→ 模拟线程。
///
/// P2-3 服务端主动心跳：每 `WS_PING_INTERVAL` 发一次 JSON 文本层 ping
/// （JS 无法发协议级 ping 帧，沿用 C-E3 的应用层心跳协议），`WS_PONG_TIMEOUT`
/// 内未收到任何 pong 即判客户端死亡并关闭连接——弥补「前端 30s 心跳在后台
/// 标签页被浏览器节流」的盲区。前端 ws.ts 的 `case "ping"` 已被动回 pong，
/// 双方对称（任何 pong 都证明链路存活）。
pub(super) async fn ws_loop(
    mut socket: axum::extract::ws::WebSocket,
    entry: Arc<crate::game::GameEntry>,
) {
    use axum::extract::ws::Message;

    /// 服务端主动 ping 周期（与前端 PING_INTERVAL_MS 同值对称）。
    const WS_PING_INTERVAL: Duration = Duration::from_secs(30);
    /// 判死阈值：发出 ping 后无任何 pong 的容忍时限（与前端 PONG_TIMEOUT_MS 同值对称）。
    const WS_PONG_TIMEOUT: Duration = Duration::from_secs(10);

    let mut events_rx = entry.events_tx.subscribe();
    // 服务端主动 ping 定时器：首 tick 立即完成（首 ping 在循环首轮发出）。
    let mut ping_interval = tokio::time::interval(WS_PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // 判死截止：Some(deadline) 表示已发 ping 且尚未收到 pong。
    let mut pong_deadline: Option<tokio::time::Instant> = None;
    loop {
        let deadline_fut = async {
            match pong_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                // 无未决 ping：永不就绪（不给 select 空转）。
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = ping_interval.tick() => {
                // 周期心跳：只携带 ts 供诊断；客户端仅需回 {"type":"pong"}。
                // 先发 ping 并设置 deadline，下一轮 select 才可能选中判死分支——
                // 不存在「ping 刚发出即被判死」的竞态（sleep_until 最早 10s 后就绪）。
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                if socket
                    .send(Message::Text(format!(r#"{{"type":"ping","ts":{ts}}}"#).into()))
                    .await
                    .is_err()
                {
                    break; // 客户端已断开
                }
                pong_deadline = Some(tokio::time::Instant::now() + WS_PONG_TIMEOUT);
            }
            _ = deadline_fut => {
                // 判死：超时无 pong → 关闭连接（socket drop 自动回收 broadcast 订阅）。
                // 前端 onclose 走既有指数退避重连 + syncJournal/syncPending 恢复。
                break;
            }
            event = events_rx.recv() => {
                match event {
                    Ok(ev) => {
                        let json = serde_json::to_string(&ev).expect("WsEvent 序列化必然成功");
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break; // 客户端断开
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // 慢消费者：丢事件（REST journal?since= 兜底），发提示
                        let _ = socket.send(Message::Text(format!(r#"{{"type":"lagged","skipped":{n}}}"#).into())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // 应用层心跳（C-E3）：客户端 ping → 服务端 pong（JSON 文本层，
                        // 非 WS 协议帧——JS 无法发协议级 ping）。
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            match v.get("type").and_then(|t| t.as_str()) {
                                Some("ping") => {
                                    let _ = socket.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                                    continue;
                                }
                                Some("pong") => {
                                    // P2-3：任何 pong 都证明链路存活（前端主动 ping 的
                                    // pong 与服务端主动 ping 的 pong 语义兼容，双方对称）。
                                    pong_deadline = None;
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        let parsed: Result<WsIn, _> = serde_json::from_str(&text);
                        match parsed {
                            Ok(WsIn::Decide {
                                decisions,
                                batch_id,
                                request_id,
                            }) => {
                                if let Err(e) =
                                    entry.submit_decisions(request_id, decisions, batch_id)
                                {
                                    let _ = socket.send(Message::Text(format!(r#"{{"type":"error","message":"{e}"}}"#).into())).await;
                                }
                            }
                            Ok(WsIn::Advance { months }) => {
                                let months = months.unwrap_or(1).clamp(1, 240); // 与 REST /advance 对齐
                                if let Err(e) = entry.request_advance(months) {
                                    let _ = socket.send(Message::Text(format!(r#"{{"type":"error","message":"{e}"}}"#).into())).await;
                                }
                            }
                            Err(e) => {
                                let _ = socket.send(Message::Text(format!(r#"{{"type":"error","message":"非法消息：{e}"}}"#).into())).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
