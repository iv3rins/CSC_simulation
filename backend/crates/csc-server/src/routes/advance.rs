use super::{err, game_of};
use crate::game::Policy;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::time::Duration;

// —— 推进与决策 ——

/// FIFA 式赛季推进请求（无参数；POST 空 JSON 或 `{}` 均可）。
#[derive(Debug, Default, Deserialize)]
pub struct AdvanceSeasonRequest {}

pub(super) async fn advance_season(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(_req): Json<AdvanceSeasonRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.story {
        return err(
            StatusCode::CONFLICT,
            "故事模式的正式比赛续体尚未完成，不能通过旧赛季推进绕过选择",
        );
    }
    // 比赛日闸门：赛季推进同样会批内结算玩家 pending 对阵，LIVE 消失。Human 下拦截，
    // 要求先处理比赛日（进入 LIVE/跳过）。
    if entry.policy == crate::game::Policy::Human && entry.live_gate_active() {
        let matches = entry.live_today();
        return err(
            StatusCode::CONFLICT,
            format!(
                "比赛日闸门激活：今天有 {} 场你的比赛可 LIVE。请进入实时观战或先跳过（POST /live/today/skip），再进行赛季推进。",
                matches.len()
            ),
        );
    }
    let entry2 = entry.arc();
    let result = tokio::task::spawn_blocking(move || {
        entry2.advance_season_blocking(Duration::from_secs(300))
    })
    .await;
    match result {
        Ok(Ok(summaries)) => Json(serde_json::json!({
            "mode": "season",
            "summaries": summaries,
        }))
        .into_response(),
        Ok(Err(e)) => err(StatusCode::CONFLICT, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

#[derive(Debug, Deserialize)]
pub struct AdvanceRequest {
    pub months: Option<u32>,
    /// 日级推进：days 优先于 months；逐日产出文字直播 Step 摘要
    pub days: Option<u32>,
}

impl AdvanceRequest {
    /// days 与 months 互斥（T5 防御：同一请求只允许一种粒度，防止误传两者）。
    fn validate(&self) -> Result<(), String> {
        if self.days.is_some() && self.months.is_some() {
            return Err("advance 请求中 days 与 months 不能同时出现".into());
        }
        Ok(())
    }
}

pub(super) async fn advance(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<AdvanceRequest>,
) -> Response {
    if let Err(msg) = req.validate() {
        return err(StatusCode::BAD_REQUEST, msg);
    }
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    if entry.story {
        return err(
            StatusCode::CONFLICT,
            "故事模式的正式比赛续体尚未完成，请使用生涯场景接口",
        );
    }
    let days = req.days.map(|d| d.clamp(1, 360));
    let months = req.months.unwrap_or(1).clamp(1, 240);
    match entry.policy {
        // 全自动：等待完成返回摘要
        Policy::Auto => {
            let entry2 = entry.arc();
            let result = match days {
                Some(d) => {
                    tokio::task::spawn_blocking(move || {
                        entry2.advance_days_blocking(d, std::time::Duration::from_secs(300))
                    })
                    .await
                }
                None => {
                    tokio::task::spawn_blocking(move || {
                        entry2.advance_blocking(months, std::time::Duration::from_secs(300))
                    })
                    .await
                }
            };
            match result {
                Ok(Ok(summaries)) => {
                    Json(serde_json::json!({ "summaries": summaries })).into_response()
                }
                Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
                Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
            }
        }
        // 真人：立即接受，进度经 WS Step 事件 / 状态轮询观察
        Policy::Human => {
            // 比赛日闸门：**无论日级还是月级推进**，只要今天有玩家 pending LIVE 对阵
            // （live_gate_active），一律 409——否则日级推进会在批内把比赛日直接越过，
            // LIVE 消失、20s 提示条永不触发（P2 修复：手动「下一天」不再绕过闸门）。
            // 玩家必须先处理比赛日：进入 LIVE（POST /live）或跳过（POST /live/today/skip）。
            if entry.live_gate_active() {
                let matches = entry.live_today();
                return err(
                    StatusCode::CONFLICT,
                    format!(
                        "比赛日闸门激活：今天有 {} 场你的比赛可 LIVE（{}）。请进入实时观战或先跳过（POST /live/today/skip），再进行推进。",
                        matches.len(),
                        matches
                            .iter()
                            .map(|m| m.event_name.as_str())
                            .collect::<Vec<_>>()
                            .join("、")
                    ),
                );
            }
            let requested = match days {
                Some(d) => entry.request_advance_days(d),
                None => entry.request_advance(months),
            };
            match requested {
                Ok(()) => (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({
                        "status": "advancing",
                        "note": "决策点经 GET /decisions/pending 或 WS 推送；提交后本步完成并推送 step 事件"
                    })),
                )
                    .into_response(),
                Err(e) => err(StatusCode::CONFLICT, e),
            }
        }
    }
}
