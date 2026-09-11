use super::game_of;
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};

// —— 洞察与参照（生涯/比赛/队伍洞察、世界叙事、真实 TOP20 参照、生态校准） ——
/// 生涯洞察（结果解释层）：转会市场为什么有/没有报价。
/// 数据由 csc-systems::TransferEngine::market_insight 从当前 world/vrs 规则推导，
/// 前端只展示不猜测。
pub(super) async fn get_insights(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let Some(pid) = snap
        .world
        .players
        .iter()
        .find(|p| p.is_player() && !p.retired)
        .map(|p| p.id)
    else {
        return Json(serde_json::json!({
            "transfer": serde_json::Value::Null,
            "match": serde_json::Value::Null,
            "team": serde_json::Value::Null,
        }))
        .into_response();
    };
    let vrs = csc_vrs::engine::VrsEngine::from_database(snap.vrs.clone());
    let transfer =
        csc_systems::transfer::TransferEngine::market_insight(&snap.world, &vrs, pid).ok();
    let match_insight = csc_core::match_insight(&snap.world, &snap.events, pid, &state.text);
    let team_insight = csc_core::team_status_insight(&snap.world, pid, &state.text);
    Json(serde_json::json!({
        "transfer": transfer,
        "match": match_insight,
        "team": team_insight,
    }))
    .into_response()
}

/// 世界叙事（队伍风云 + 宿敌）：纯只读派生，让玩家看到 NPC 世界的「活的故事」
/// ——谁在重建/谁崛起/谁的王朝/谁在低谷、以及宿敌恩怨。数据源 = 既有 GameState
/// （阵容年龄/赛事结果/转会流），不新增状态。
pub(super) async fn get_world_stories(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    // world_stories 接收 &GameState 做只读派生（不 clone 完整快照）
    Json(
        serde_json::to_value(csc_core::world_stories(&snap, &state.text))
            .unwrap_or(serde_json::Value::Null),
    )
    .into_response()
}

/// 真实 HLTV TOP20 三年「传奇参照」榜（`assets/top20-data/*.json`，随资产启动装载）。
/// 与模拟榜单同一 API 面，供前端同屏对照——玩家可看到真实传奇的名次，并对照
/// 自己模拟世界的 TOP20 排名（同「精神」竞争，非同数值混榜）。
/// 若年龄基线（roles_baseline.json）可用，则按 nickname 关联为每人补上
/// `age_in_year`（该上榜年份的年龄）。
pub(super) async fn get_top20_reference(
    State(state): State<AppState>,
    Path(_id): Path<u64>,
) -> Response {
    let baseline = state
        .assets
        .baseline
        .as_deref()
        .and_then(|b| csc_entities::baseline::RoleBaseline::from_json_str(b).ok());
    let years: Vec<serde_json::Value> = state
        .assets
        .real_top20
        .years()
        .iter()
        .map(|y| {
            let players: Vec<serde_json::Value> = y
                .players
                .iter()
                .map(|p| {
                    let mut v = serde_json::to_value(p).unwrap_or(serde_json::Value::Null);
                    if let (Some(obj), Some(b)) = (v.as_object_mut(), baseline.as_ref())
                        && let Some(base_age) = b.age_of(&p.nickname)
                    {
                        obj.insert(
                            "age_in_year".into(),
                            serde_json::json!(base_age - (csc_simulation::BASELINE_YEAR - y.year)),
                        );
                    }
                    v
                })
                .collect();
            serde_json::json!({ "year": y.year, "players": players })
        })
        .collect();
    Json(serde_json::json!({ "years": years })).into_response()
}

/// 生态系统校准报告：用真实三年 TOP20 名次轨迹拟合虚拟成长/衰减/轮换参数，
/// 并给出"未来新人 TOP20 走势"的推演文案。数据源 = 启动时装载的 top20-data
/// + roles_baseline（nickname 关联年龄）。
pub(super) async fn get_calibration(
    State(state): State<AppState>,
    Path(_id): Path<u64>,
) -> Response {
    let baseline = state
        .assets
        .baseline
        .as_deref()
        .and_then(|b| csc_entities::baseline::RoleBaseline::from_json_str(b).ok());
    let report = csc_simulation::EcoCalibrator::report(&state.assets.real_top20, baseline.as_ref());
    Json(serde_json::json!({
        "elite_tail_ratio": report.profile.elite_tail_ratio,
        "career_drift": report.profile.career_drift,
        "rookie_elite_rate": report.profile.rookie_elite_rate,
        "distinct_players": report.distinct_players,
        "appearance_histogram": report.appearance_histogram,
        "movement_samples": report.movement_samples,
        "newcomer_rates": report.newcomer_rates,
        "peak_age": {
            "samples": report.peak_age.samples.len(),
            "histogram": report.peak_age.histogram,
            "mean": report.peak_age.mean,
            "median": report.peak_age.median,
            "min": report.peak_age.min,
            "max": report.peak_age.max,
        },
        "summary": report.summary,
    }))
    .into_response()
}
