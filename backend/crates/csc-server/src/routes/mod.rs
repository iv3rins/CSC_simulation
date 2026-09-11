//! REST / WS 路由——服务端协议层（前端消费的唯一 API 面）。
//!
//! 以下端点清单与 [`router()`] 保持一致（47 条 REST + 1 条 WS；增删路由必须同步本文档，
//! 否则文档滞后会误导前端对接——P2-7）。
//!
//! ```text
//! REST（生命周期 / 存档）
//!   GET    /health
//!   POST   /games                              { seed?, policy?, player_name?, role? } → { game_id }
//!   DELETE /games/{id}
//!   GET    /games/{id}/state                   → GameState（完整快照，存档/调试用）
//!   GET    /games/{id}/save                    → 下载存档（canonical JSON + x-csc-* 头）
//!   GET    /games/{id}/save/gzip               → 下载 gzip 存档（x-csc-gzip-bytes / x-csc-save-crc32）
//!   POST   /games/{id}/load                    { GameState } → 读档（自动迁移低版本）
//!   POST   /games/{id}/load/gzip               gzip 存档 → 读档
//!
//! REST（推进 / 决策 / 主动操作）
//!   POST   /games/{id}/advance                 { months? } → auto: [StepSummary] / human: 202
//!   POST   /games/{id}/advance-season          推进到赛季边界
//!   GET    /games/{id}/decisions/pending       → 待决策批次（Human）
//!   POST   /games/{id}/decisions               { decisions: [...] } → 提交决策
//!   GET    /games/{id}/training/options        → 训练计划选项
//!   POST   /games/{id}/training                { focus } → 排定训练计划
//!   POST   /games/{id}/season-goal             { goal }
//!   POST   /games/{id}/finance                 { ... }
//!   POST   /games/{id}/actions/statement       { tone }
//!   POST   /games/{id}/actions/match-plan      { ... }
//!
//! REST（查询）
//!   GET    /games/{id}/view                    → ClientState（轻量视图，高频轮询用）
//!   GET    /games/{id}/calendar                → 赛季赛程
//!   GET    /games/{id}/events/{event_name}     → 单赛事事件聚合
//!   GET    /games/{id}/fixtures/watched        → 已看完对局（POST 标记）
//!   GET    /games/{id}/news                    → 新闻流
//!   GET    /games/{id}/replay                  → { series }（单场直播回放懒加载）
//!   GET    /games/{id}/summary                 → 世界概览
//!   GET    /games/{id}/journal?since=N         → { events, next_seq }（增量）
//!   GET    /games/{id}/archive                 → 生涯档案
//!   GET    /games/{id}/ending                  → 结局/生涯终结视图
//!   GET    /games/{id}/top20                   → 本年度 TOP20
//!   GET    /games/{id}/top20/history           → 历届 TOP20 榜单
//!   GET    /games/{id}/top20/reference         → TOP20 参考依据
//!   GET    /games/{id}/world                   → 世界故事流
//!   GET    /games/{id}/insights                → 洞察
//!   GET    /games/{id}/calibration             → 校准视图
//!
//! REST（转会）
//!   GET    /games/{id}/transfers/market        → 转会窗市场（去弹窗契约）
//!   GET    /games/{id}/transfers/offers        → 主动转会候选
//!   POST   /games/{id}/transfers/sign          { team } → 签约
//!
//! REST（LIVE 比赛）
//!   POST   /games/{id}/live                    { ... } → 开 LIVE（幂等可重进）
//!   GET    /games/{id}/live/today              → 今日主角对阵（pending）
//!   POST   /games/{id}/live/today/skip         → 跳过今日 LIVE
//!   GET    /games/{id}/live/{match_id}         → LIVE 会话状态
//!   POST   /games/{id}/live/{match_id}/advance → 推进一回合
//!   POST   /games/{id}/live/{match_id}/decide  → 提交回合决策
//!   GET    /games/{id}/live/{match_id}/review  → 复盘回放
//!   POST   /games/{id}/live/{match_id}/skip    → 跳过该场
//!   POST   /games/{id}/live/{match_id}/watch   → 标记已看完
//!
//! WS /games/{id}/ws
//!   服务端 → 客户端: { type:"decisions", batch_id, ... } / { type:"step", ... } /
//!                   { type:"journal", events, next_seq } / { type:"notice", ... }
//! ```
//!
//! 统一错误格式：非 2xx 响应体 = `{ "error": "<人话原因>" }`（见 [`err`]）。
//!
//! 模块划分：每个领域一个子模块（advance / decision / insights / lifecycle / live / query / transfer），
//! 路由函数集中在本文件 [`router()`] 注册。
mod advance;
mod career;
mod decision;
mod insights;
mod lifecycle;
mod live;
mod query;
mod transfer;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};

use crate::state::AppState;

use advance::*;
use decision::*;
use insights::*;
use lifecycle::*;
use live::*;
use query::*;
use transfer::*;
/// 全应用路由。
///
/// **请求体上限 128MB**（基准：20 年生涯 GameState ≈68MB；axum 默认 2MB 会把
/// `/load` 打成 413——长生涯读档必须放行）。响应侧无默认上限。
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/games", post(create_game))
        .route("/games/{id}/career", get(career::get_career))
        .route("/games/{id}/career/continue", post(career::continue_career))
        .route("/games/{id}/career/choices", post(career::choose_career))
        .route("/games/{id}/career/history", get(career::career_history))
        .route(
            "/games/{id}/career/operations/{request_id}",
            get(career::career_operation),
        )
        .route("/games/{id}", delete(shutdown_game))
        .route("/games/{id}/state", get(get_state))
        .route("/games/{id}/view", get(get_view))
        .route("/games/{id}/calendar", get(get_calendar))
        .route("/games/{id}/events/{event_name}", get(get_event_aggregate))
        .route("/games/{id}/fixtures/watched", post(mark_fixture_watched))
        .route("/games/{id}/news", get(get_news))
        .route("/games/{id}/replay", get(get_replay))
        .route("/games/{id}/summary", get(get_summary))
        .route("/games/{id}/journal", get(get_journal))
        .route("/games/{id}/archive", get(get_archive))
        .route("/games/{id}/ending", get(get_ending))
        .route("/games/{id}/save", get(save_game))
        .route("/games/{id}/save/gzip", get(save_game_gzip))
        .route("/games/{id}/load", post(load_game))
        .route("/games/{id}/load/gzip", post(load_game_gzip))
        .route("/games/{id}/advance", post(advance))
        .route("/games/{id}/advance-season", post(advance_season))
        .route("/games/{id}/top20", get(get_top20))
        .route("/games/{id}/top20/history", get(get_top20_history))
        .route("/games/{id}/top20/reference", get(get_top20_reference))
        .route("/games/{id}/world", get(get_world_stories))
        .route("/games/{id}/insights", get(get_insights))
        .route("/games/{id}/calibration", get(get_calibration))
        .route("/games/{id}/decisions/pending", get(get_pending))
        .route("/games/{id}/decisions", post(submit_decisions))
        .route("/games/{id}/training/options", get(get_training_options))
        .route("/games/{id}/training", post(plan_training))
        .route("/games/{id}/season-goal", post(set_season_goal))
        .route("/games/{id}/finance", post(post_finance))
        .route("/games/{id}/transfers/market", get(get_transfer_market))
        .route("/games/{id}/transfers/offers", get(get_transfer_offers))
        .route("/games/{id}/transfers/sign", post(sign_transfer))
        .route("/games/{id}/actions/statement", post(post_statement))
        .route("/games/{id}/actions/match-plan", post(post_match_plan))
        .route("/games/{id}/live", post(start_live))
        .route("/games/{id}/live/today", get(get_live_today))
        .route("/games/{id}/live/today/skip", post(skip_live_today))
        .route("/games/{id}/live/{match_id}", get(get_live))
        .route("/games/{id}/live/{match_id}/advance", post(advance_live))
        .route("/games/{id}/live/{match_id}/decide", post(decide_live))
        .route("/games/{id}/live/{match_id}/review", get(get_live_review))
        .route("/games/{id}/live/{match_id}/skip", post(skip_live))
        .route("/games/{id}/live/{match_id}/watch", post(mark_live_watched))
        .route("/games/{id}/ws", get(ws_handler))
        .layer(DefaultBodyLimit::max(128 * 1024 * 1024))
}

/// 统一错误响应（JSON 文本 + 状态码）。
fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

/// 按 id 取游戏（404 兜底）。
///
/// 并发安全协议：
/// - `get_or_restore` 只淘汰空闲局（推进/保存/落盘/租约中都不动）；
/// - 取到 Arc 后在 games 锁内登记请求租约（`GameHandle`，Drop 自动释放）；
/// - 若登记前该局刚好被另一并发请求卸载，本函数重试 `get_or_restore`，
///   因此路由永远拿不到「已退出的模拟线程」；
/// - 容量收缩不在请求路径上做（长档 gzip 落盘昂贵）：`/health` 后台排发收缩，
///   create/get_or_restore 的预卸载阶段也持续回收空闲局。
async fn game_of(state: &AppState, id: u64) -> Result<crate::game::GameHandle, Box<Response>> {
    loop {
        let _entry = state
            .games
            .get_or_restore(id)
            .await
            .ok_or_else(|| Box::new(err(StatusCode::NOT_FOUND, format!("游戏 {id} 不存在"))))?;
        let Some(handle) = state.games.lease_entry(id) else {
            // 取 Arc 与登记租约之间被卸载：重试恢复。
            tokio::task::yield_now().await;
            continue;
        };
        state.games.touch(id);
        return Ok(handle);
    }
}

// —— 基础 ——

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    // 健康检查只排发、不等待容量收缩：长档 gzip 落盘是秒级重操作，
    // /health 必须保持毫秒级响应；压测脚本可轮询到 games ≤ max_active。
    state.games.spawn_rebalance();
    Json(serde_json::json!({
        "status": "ok",
        "games": state.games.count(),
        "persisted_games": state.games.persisted_count(),
        "max_active": state.games.max_active(),
    }))
}
