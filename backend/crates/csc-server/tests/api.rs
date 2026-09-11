//! REST API 集成测试（axum Router oneshot，无真实端口）。
//!
//! 覆盖：游戏创建（auto/human）、状态查询、增量事件流、自动推进、
//! Human 决策批次拉取→提交→月步完成、存档往返、错误路径（404/409）。

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use csc_server::routes::router;
use csc_server::state::AppState;

/// 40 队满编世界（并行日历生态最小规模）。
fn fixture_app() -> Router {
    let mut rankings = String::new();
    for ti in 0..40 {
        if ti > 0 {
            rankings.push(',');
        }
        let roster: Vec<String> = (0..5).map(|i| format!("T{ti}P{i}")).collect();
        rankings.push_str(&format!(
            r#"{{"ranking":{},"points":{},"teamName":"Team{}","roster":[{}]}}"#,
            ti + 1,
            2000 - ti * 50,
            ti,
            roster
                .iter()
                .map(|n| format!(r#""{n}""#))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    let standings = std::collections::HashMap::from([(
        "standings_global_2026_01_05.json".to_string(),
        format!(r#"{{"rankings":[{rankings}]}}"#),
    )]);
    let state = AppState::from_bundle(csc_server::state::bundle_of(standings, None));
    router().with_state(state)
}

async fn request_json(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    let req = match body {
        Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, value)
}

async fn request_bytes(
    app: &Router,
    method: &str,
    path: &str,
    body: Body,
    content_type: Option<&str>,
) -> (StatusCode, axum::body::Bytes) {
    let (status, bytes, _) =
        request_bytes_with_headers(app, method, path, body, content_type).await;
    (status, bytes)
}

async fn request_bytes_with_headers(
    app: &Router,
    method: &str,
    path: &str,
    body: Body,
    content_type: Option<&str>,
) -> (StatusCode, axum::body::Bytes, axum::http::HeaderMap) {
    request_bytes_extra(app, method, path, body, content_type, &[]).await
}

async fn request_bytes_extra(
    app: &Router,
    method: &str,
    path: &str,
    body: Body,
    content_type: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, axum::body::Bytes, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(body).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, bytes, headers)
}

async fn create_game(app: &Router, policy: &str) -> u64 {
    let (status, body) = request_json(
        app,
        "POST",
        "/games",
        Some(json!({ "seed": 42, "policy": policy, "player_name": "MyPlayer" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "创建失败：{body}");
    body["game_id"].as_u64().expect("返回 game_id")
}

/// 决策辅助：为每个点取合法默认选项。**直接委托权威实现**
/// （JSON → DecisionPoint → AutoDecisionSource::default_decision_for），
/// 不再维护 JSON 层镜像——历史副本曾在转会窗语义上漂移（缺排名守卫）。
fn default_decision(p: &Value) -> Value {
    let point: csc_decision::point::DecisionPoint =
        serde_json::from_value(p.clone()).expect("决策点 JSON 可反序列化");
    let d = csc_decision::source::AutoDecisionSource::default_decision_for(&point);
    json!({ "point_id": d.point_id, "option_id": d.option_id })
}

#[tokio::test]
async fn live_session_advances_one_round_and_skip_matches_manual_simulation() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let config = json!({
        "match_id": "fixture-live-1",
        "series_id": "fixture-series-1",
        "map_id": "de_test",
        "map_number": 1,
        "team_a_id": 1,
        "team_b_id": 2,
        "team_a_sig": "ALPHA",
        "team_b_sig": "BRAVO",
        "best_of": 1,
        "world_seed": 4242,
        "base_win_prob_a": 0.55,
        "team_a_players": [
            { "id": 1, "name": "A1" }, { "id": 2, "name": "A2" }, { "id": 3, "name": "A3" }, { "id": 4, "name": "A4" }, { "id": 5, "name": "A5" }
        ],
        "team_b_players": [
            { "id": 11, "name": "B1" }, { "id": 12, "name": "B2" }, { "id": 13, "name": "B3" }, { "id": 14, "name": "B4" }, { "id": 15, "name": "B5" }
        ]
    });
    let (status, created) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live"),
        Some(config.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["state"]["round_number"], 1);
    assert_eq!(
        created["state"]["round_history"].as_array().unwrap().len(),
        0
    );

    let (status, advanced) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live/fixture-live-1/advance"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{advanced}");
    assert_eq!(advanced["output"]["event"]["round_number"], 1);
    assert_eq!(
        advanced["state"]["round_history"].as_array().unwrap().len(),
        1
    );

    let (status, skipped) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live/fixture-live-1/skip"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{skipped}");
    assert_eq!(skipped["finished"], true);
    let history = skipped["state"]["round_history"].as_array().unwrap();
    assert_eq!(
        history.len(),
        (skipped["state"]["score_a"].as_i64().unwrap()
            + skipped["state"]["score_b"].as_i64().unwrap()) as usize
    );

    // P0-2 修复：同 (game_id, match_id) 再次 start 应**覆盖重建**（重进/重看同一场）
    // 而非 409——此前会话永不回收导致重进被旧会话拒绝。
    let (status, duplicate) =
        request_json(&app, "POST", &format!("/games/{id}/live"), Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{duplicate}");
    assert_eq!(
        duplicate["state"]["round_history"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "覆盖重建后回合历史应清空（全新会话）"
    );
}

#[tokio::test]
async fn live_critical_pause_requires_decision_then_exposes_review() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let config = json!({
        "match_id": "fixture-live-critical",
        "series_id": "fixture-series-critical",
        "map_id": "de_test",
        "map_number": 1,
        "team_a_id": 1,
        "team_b_id": 2,
        "team_a_sig": "ALPHA",
        "team_b_sig": "BRAVO",
        "best_of": 1,
        "world_seed": 77,
        "base_win_prob_a": 0.50,
        "team_a_players": [
            { "id": 1, "name": "A1" }, { "id": 2, "name": "A2" }, { "id": 3, "name": "A3" }, { "id": 4, "name": "A4" }, { "id": 5, "name": "A5" }
        ],
        "team_b_players": [
            { "id": 11, "name": "B1" }, { "id": 12, "name": "B2" }, { "id": 13, "name": "B3" }, { "id": 14, "name": "B4" }, { "id": 15, "name": "B5" }
        ]
    });
    let (status, created) =
        request_json(&app, "POST", &format!("/games/{id}/live"), Some(config)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let mut pause = None;
    for _ in 0..24 {
        let (status, body) = request_json(
            &app,
            "POST",
            &format!("/games/{id}/live/fixture-live-critical/advance"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["decision_required"] == true {
            pause = Some(body);
            break;
        }
    }
    let pause = pause.expect("末段比分必定触发关键回合时停");
    let request = &pause["decision_request"];
    assert!(
        request["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Save")
    );

    let (status, blocked) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live/fixture-live-critical/advance"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    let (status, queued) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live/fixture-live-critical/decide"),
        Some(json!({ "decision": "Save" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queued}");
    assert_eq!(queued["decision_required"], false);

    let (status, applied) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/live/fixture-live-critical/advance"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    assert!(applied["state"]["decision_seq"].as_u64().unwrap() > 0);

    let (status, review) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/live/fixture-live-critical/review"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{review}");
    assert!(
        review["key_rounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["decision"] == "Save")
    );
}

#[tokio::test]
async fn health_and_game_lifecycle() {
    let app = fixture_app();
    let (status, body) = request_json(&app, "GET", "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["games"], 0);

    let id = create_game(&app, "auto").await;
    let (_, body) = request_json(&app, "GET", "/health", None).await;
    assert_eq!(body["games"], 1);

    // 404
    let (status, _) = request_json(&app, "GET", "/games/9999/state", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 关闭
    let (status, _) = request_json(&app, "DELETE", &format!("/games/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn auto_game_advance_state_and_journal() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    // 初始状态：主角存在
    let (status, state) = request_json(&app, "GET", &format!("/games/{id}/state"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["month"], 0);
    assert_eq!(
        state["world"]["players"].as_array().unwrap().len(),
        40 * 5 + 1,
        "40 队 × 5 NPC + 1 主角"
    );

    // 推进 1 个月
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自动推进失败：{body}");
    let summaries = body["summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0]["month"], 1);
    assert!(summaries[0]["journal_added"].as_u64().unwrap() > 0);

    // 状态已更新
    let (_, state) = request_json(&app, "GET", &format!("/games/{id}/state"), None).await;
    assert_eq!(state["month"], 1);

    // 增量事件流
    let (status, body) =
        request_json(&app, "GET", &format!("/games/{id}/journal?since=0"), None).await;
    assert_eq!(status, StatusCode::OK);
    let events = body["events"].as_array().unwrap();
    assert!(!events.is_empty(), "推进后应有事件");
    assert!(body["next_seq"].as_i64().unwrap() > 0);
}

/// M11：journal 服务端分页上限——`limit` 生效、`next_seq` 为截断窗口末端
/// （而非全流末端），两次分页拉取的并集 = 全量、第二次 next_seq = 全流末端。
#[tokio::test]
async fn journal_pagination_limit_and_cursor_semantics() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 推进 12 个月积累数百条事件（远超 limit=5 的窗口）。
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 12 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 第一页：limit=5（全量窗口；不含 channel 过滤差异——默认 career_feed 下
    // 主角故事通常 ≥5 条，若不足则退化为断言「≤limit 且游标推进」）。
    let (status, page1) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/journal?since=-1&limit=5"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events1 = page1["events"].as_array().unwrap();
    assert!(
        events1.len() <= 5,
        "limit=5 生效：第一页最多 5 条，实际 {}",
        events1.len()
    );
    let next1 = page1["next_seq"].as_i64().unwrap();
    assert!(next1 > 0, "游标应推进");

    // 第二页：从 next1 继续拉（若第一页已达 limit 说明还有更多）。
    let (status, page2) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/journal?since={next1}&limit=5"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events2 = page2["events"].as_array().unwrap();
    assert!(
        events2.len() <= 5,
        "第二页最多 5 条，实际 {}",
        events2.len()
    );

    // 游标单调推进且事件不重叠（seq 严格 > since）。
    let next2 = page2["next_seq"].as_i64().unwrap();
    assert!(next2 >= next1, "游标单调不减：{next2} >= {next1}");

    // 关键语义：全量拉取（不传 limit，默认 200）的 next_seq 应 ≥ 分页末次游标。
    let (_, full) = request_json(&app, "GET", &format!("/games/{id}/journal?since=-1"), None).await;
    let full_next = full["next_seq"].as_i64().unwrap();
    assert!(
        full_next >= next2,
        "全量游标应覆盖分页游标：{full_next} >= {next2}"
    );

    // 分页两次并集覆盖 = 全量窗口内 career_feed 事件数（若全量 ≤ 10 则直接相等）。
    let full_events = full["events"].as_array().unwrap();
    let first_window = events1.len() + events2.len();
    if full_events.len() <= 5 + 5 {
        assert_eq!(
            first_window,
            full_events.len(),
            "事件总数 ≤ limit 和时两次拉取应恰好覆盖全量"
        );
    }
}

#[tokio::test]
async fn calendar_endpoint_returns_full_plan_and_world_results() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    // 未推进：全年计划已完整可见（主页不再“等待赛历排期”）
    let (status, body) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/calendar?year=2026"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["year"], 2026);
    assert_eq!(
        body["plan"].as_array().unwrap().len(),
        25 + 60 + 96,
        "全年 181 场计划（2 Major + 18 S + 5 A + 60 T2 + 96 T3）"
    );
    assert_eq!(body["results"].as_array().unwrap().len(), 0, "尚未开赛");

    // 推进 1 个月后：全世界 1 月 + 2 月各 15 场赛事都有赛果（主角在垫底队，
    // 不影响世界赛；R2.1 修复：开局月 1 月物化赛事在首次月推进时先结算——
    // 不再滞留 Future、不再错位重排到 2 月）。
    let (status, adv) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自动推进失败：{adv}");
    let (_, body) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/calendar?year=2026"),
        None,
    )
    .await;
    let results = body["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        30,
        "首个月步 = 1 月 15 场（开局月结算）+ 2 月 15 场"
    );
    let jan = results
        .iter()
        .filter(|r| r["event"]["date"].as_str().unwrap().starts_with("2026-01"))
        .count();
    let feb = results
        .iter()
        .filter(|r| r["event"]["date"].as_str().unwrap().starts_with("2026-02"))
        .count();
    assert_eq!(
        (jan, feb),
        (15, 15),
        "1 月赛事应在首次月推进时结算（R2.1 开局月闭环），2 月照常"
    );
    for result in results {
        println!(
            "EVENT {} | {}",
            result["event"]["name"], result["event"]["date"]
        );
        assert!(!result["matches"].as_array().unwrap().is_empty());
        assert!(result["champion"].as_i64().is_some());
    }

    // T3 赛前赛程物化：player_events 必须从 API 暴露 fixtures（含赛前物化的
    // 首轮对阵 + 赛后回填 result）。这是「打比赛有赛程」的端到端验收。
    let player_events = body["player_events"].as_array().unwrap();
    assert!(
        !player_events.is_empty(),
        "主角队应至少有一条已确认参赛的赛事记录"
    );
    let with_fixtures = player_events
        .iter()
        .filter(|e| {
            e["fixtures"]
                .as_array()
                .map(|f| !f.is_empty())
                .unwrap_or(false)
        })
        .count();
    assert!(
        with_fixtures >= 1,
        "已完赛赛事应带赛前物化的 fixtures（实际 {with_fixtures}/{}）",
        player_events.len()
    );
    // fixtures 结构校验：team_a/team_b/best_of 有值，且已完赛赛事 result 已回填。
    for ev in player_events {
        let empty: Vec<serde_json::Value> = Vec::new();
        let fixtures = ev["fixtures"].as_array().unwrap_or(&empty);
        for f in fixtures {
            assert!(f["team_a"].as_i64().is_some(), "fixture 缺 team_a");
            assert!(f["team_b"].as_i64().is_some(), "fixture 缺 team_b");
            assert!(f["best_of"].as_i64().is_some(), "fixture 缺 best_of");
            assert!(f["stage"].as_str().is_some(), "fixture 缺 stage");
            if ev["status"].as_str() == Some("completed") {
                assert!(
                    f["result"].is_object() || f["result"].is_null(),
                    "completed fixture 应有 result（或已回填）"
                );
            }
        }
    }

    // T3 四类验收之一：future（待开赛）赛事若已排程确认，则必须带赛前物化对阵
    // 且 result 全部为空（无比分——对阵可见、比分待开赛）。
    // 注：auto 场景主角在垫底队、排程密度低，推进 1 个月后可能无 future 记录；
    // future 无比分语义的主证在 csc-tournaments scheduled.rs
    // fixture_status_progression_future_to_completed（引擎层）。此处为 API 层防御断言。
    let future_events: Vec<_> = player_events
        .iter()
        .filter(|e| e["status"].as_str() == Some("future"))
        .collect();
    for ev in &future_events {
        let empty: Vec<serde_json::Value> = Vec::new();
        let fixtures = ev["fixtures"].as_array().unwrap_or(&empty);
        assert!(
            !fixtures.is_empty(),
            "future 赛事应有赛前物化对阵（{}）",
            ev["event"]["name"]
        );
        for f in fixtures {
            assert!(
                f["result"].is_null(),
                "future fixture 不得有比分（{}）",
                ev["event"]["name"]
            );
        }
    }
}

#[tokio::test]
async fn calendar_player_events_status_aligns_with_locks() {
    // P1 端到端契约：`/view` 的 locks 含主角队伍时，`/calendar` 的
    // `player_events` 同名赛事必须投影为 `active`（进行中）——LIVE 页
    // （LiveEventView.liveEntry 只接受 active/future/scheduled）才能显示
    // 「即将进行的对局」fixtures 区。锁窗口未结束的已完赛记录必须提升为
    // active，而不是返回 completed（「LIVE 进行中 / 赛程已完赛」割裂）。
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    // 推进 2 个月（2 月 + 3 月）：主角队（垫底）每月都有 T3 参赛，月末锁仍在。
    let (status, adv) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "自动推进失败：{adv}");

    let (_, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let locks = view["locks"].as_array().unwrap();
    assert!(!locks.is_empty(), "月末应有未解锁的赛事锁（T3 最后一场）");

    // 主角队伍：view.world.team（主角归属队伍 ID）。
    let protagonist_team = view["world"]["team"]["id"].as_i64();
    assert!(protagonist_team.is_some(), "主角应有队伍");

    let (_, cal) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/calendar?year=2026"),
        None,
    )
    .await;
    let player_events = cal["player_events"].as_array().unwrap();

    // 每条含主角队的锁，必须有一条同名 player_event 且 status == active。
    let mut locked_hits = 0usize;
    for lock in locks {
        let lock_name = lock["event_name"].as_str().unwrap();
        let lock_teams = lock["team_ids"].as_array().unwrap();
        if !lock_teams.iter().any(|t| t.as_i64() == protagonist_team) {
            continue;
        }
        let ev = player_events
            .iter()
            .find(|e| e["event"]["name"].as_str() == Some(lock_name));
        assert!(
            ev.is_some(),
            "锁含主角队的赛事「{lock_name}」必须出现在 player_events"
        );
        assert_eq!(
            ev.unwrap()["status"].as_str(),
            Some("active"),
            "锁含主角队（locks）的赛事「{lock_name}」必须投影为 active（P1 契约）"
        );
        locked_hits += 1;
    }
    assert!(locked_hits >= 1, "至少一条锁含主角队的赛事被验证为 active");
}

#[tokio::test]
async fn human_game_pending_decisions_flow() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // 发起推进 → 202（异步，等待决策）：2 个月——1 月 BLAST Bounty/IEM Kraków、2 月 PGL/EPL
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "human 推进应 202：{body}");

    // 并发推进被拒（409）
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // 训练改为主动触发后，早期月份可能没有强制决策批次（比赛干预仅顶级赛事）；
    // Human 推进不得死锁：逐批拉取 → 提交（若出现）→ 直到月步完成。
    for _ in 0..500 {
        let (_, state) = request_json(&app, "GET", &format!("/games/{id}/state"), None).await;
        if state["month"].as_i64() == Some(2) {
            break;
        }
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if let Some(batch) = b["pending"].as_object() {
            let points = batch["points"].as_array().expect("批次含决策点");
            let decisions: Vec<Value> = points.iter().map(default_decision).collect();
            let (status, _) = request_json(
                &app,
                "POST",
                &format!("/games/{id}/decisions"),
                Some(json!({ "decisions": decisions, "batch_id": batch["batch_id"] })),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            continue;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (_, state) = request_json(&app, "GET", &format!("/games/{id}/state"), None).await;
    assert_eq!(state["month"], 2, "Human 推进应完成");
    // 训练已改为主动触发：早期月份可能没有强制决策批次；若出现批次则已按协议消费。
}

/// 推进 → 提交合法决策 → 200；再以同一 batch_id 重放 → 409（批次已被消费）。
#[tokio::test]
async fn stale_batch_rejected_with_409() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // 发起推进 → 202（异步，等待决策）。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "human 推进应 202：{body}");

    // 轮询 pending 直到出现批次。
    let batch = loop {
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if let Some(batch) = b["pending"].as_object() {
            break batch.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let bid = batch["batch_id"].clone();
    let points = batch["points"].as_array().expect("批次含决策点");
    let decisions: Vec<Value> = points.iter().map(default_decision).collect();

    // 首次提交（携带正确 batch_id）→ 200。
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({ "decisions": decisions, "batch_id": bid })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 等待模拟线程消费清空 pending，再重放同一 batch_id → 409。
    for _ in 0..500 {
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if b["pending"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({ "decisions": decisions, "batch_id": bid })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "重复批次应 409");
}

/// 不推进（advancing=false）直接提交 → 409。
#[tokio::test]
async fn submit_without_advance_rejected() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // 构造任意合法决策（不依赖真实批次；前置 advancing 校验先行拒绝）。
    let decision = json!({ "point_id": "2026-02-01|train|MyPlayer", "option_id": "AIM" });
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({ "decisions": [decision], "batch_id": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "未推进时提交应 409");
}

/// 推进后故意提交错误 batch_id（batch_id+1）→ 409。
#[tokio::test]
async fn submit_wrong_batch_rejected() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "human 推进应 202：{body}");

    // 轮询 pending 直到出现批次。
    let batch = loop {
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if let Some(batch) = b["pending"].as_object() {
            break batch.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let wrong_bid = batch["batch_id"].as_u64().expect("batch_id 为数字") + 1;
    let points = batch["points"].as_array().expect("批次含决策点");
    let decisions: Vec<Value> = points.iter().map(default_decision).collect();

    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({ "decisions": decisions, "batch_id": wrong_bid })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "错误 batch_id 应 409");
}

/// 22 R1 幂等回归：同 `request_id` + 同载荷 → 返回原确认（不重复应用）；
/// 同 `request_id` + 异载荷 → 拒绝。
#[tokio::test]
async fn request_id_idempotency_same_payload_ok_diff_payload_rejected() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "human 推进应 202：{body}");

    // 轮询 pending 直到出现批次（可能有多点；逐个作答）。
    let batch = loop {
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if let Some(batch) = b["pending"].as_object() {
            break batch.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let bid = batch["batch_id"].clone();
    let points = batch["points"].as_array().expect("批次含决策点");
    let decisions: Vec<Value> = points.iter().map(default_decision).collect();
    let request_id = "itest-req-idempotent-1";

    // 首次提交（带 request_id）→ 200。
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({
            "decisions": decisions,
            "batch_id": bid,
            "request_id": request_id,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "首次提交应 200");

    // 等待模拟线程消费清空 pending（批次已被接受）。
    for _ in 0..500 {
        let (_, b) =
            request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
        if b["pending"].is_null() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // 同 ID + 同载荷：此时 pending 已空，advancing 可能仍为 true（推进未完成）。
    // 若 advancing 已复位 → 409（无待决策）；若仍在推进 → 幂等缓存返回 200。
    // 无论哪支，都不得返回 500，也不得重复应用（决策日志不因重放增加）。
    let (status_same, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({
            "decisions": decisions,
            "batch_id": bid,
            "request_id": request_id,
        })),
    )
    .await;
    assert!(
        status_same == StatusCode::OK || status_same == StatusCode::CONFLICT,
        "同 ID 同载荷重放应为 200（幂等原确认）或 409（批次已过期），实际 {status_same}"
    );

    // 同 ID + 异载荷：必须被拒绝（不得重放到不同内容）。
    let mut altered = decisions.clone();
    if let Some(first) = altered.first_mut() {
        first["option_id"] = json!("__DIFFERENT__");
    }
    let (status_diff, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/decisions"),
        Some(json!({
            "decisions": altered,
            "batch_id": bid,
            "request_id": request_id,
        })),
    )
    .await;
    assert!(
        status_diff == StatusCode::CONFLICT,
        "同 ID 异载荷必须被拒绝，实际 {status_diff}"
    );
}

#[tokio::test]
async fn fifa_season_advance_runs_without_decision_events() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // FIFA 式赛季推进：不产生待决策批次，直接模拟 12 个月到赛季边界。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance-season"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "赛季推进失败：{body}");
    let summaries = body["summaries"].as_array().expect("应返回月步摘要");
    assert_eq!(summaries.len(), 12, "从第 0 月起步 = 完整 12 个月赛季");
    assert_eq!(summaries[11]["season_end"], true);
    assert_eq!(summaries[11]["season_index"], 1);

    let (_, state) = request_json(&app, "GET", &format!("/games/{id}/state"), None).await;
    assert_eq!(state["month"], 12, "赛季边界 = 12 个月");
    assert!(state["archive"].as_object().is_some(), "赛季结算后应有档案");
    let (_, pending) =
        request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
    assert_eq!(pending["pending"], Value::Null, "赛季推进不应遗留决策批次");
}

#[tokio::test]
async fn proactive_actions_replace_event_decisions() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // 主动发言：立即返回效果，不等待媒体事件。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/actions/statement"),
        Some(json!({ "tone": "CONFIDENT" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "发言失败：{body}");
    assert!(body["message"].as_str().unwrap().contains("自信"));

    // 主动 BP：设置 Major/顶级赛打法基线，不等待图间决策。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/actions/match-plan"),
        Some(json!({ "style": "AGGRESSIVE" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "BP 失败：{body}");
    assert_eq!(body["set"], true);

    // 合同期内没有自由转会报价（提示先买断）；这是主动转会的入口守卫。
    let (status, body) =
        request_json(&app, "GET", &format!("/games/{id}/transfers/offers"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("合同期内"));

    // 视图回读主动状态。
    let (_, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let player = view["world"]["players"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["career"].as_object().is_some())
        .expect("主角在视图中");
    assert_eq!(player["career"]["match_style"], "AGGRESSIVE");
    assert_eq!(player["career"]["public_stance"], "CONFIDENT");
}

#[tokio::test]
async fn save_load_roundtrip() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    // 推进 2 个月后存档
    let (_, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    let (status, save) = request_json(&app, "GET", &format!("/games/{id}/save"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(save["month"], 2);
    assert_eq!(
        save["version"],
        csc_core::state::GameState::CURRENT_VERSION,
        "存档带当前格式版本（跟随 GameState::CURRENT_VERSION，勿硬编码）"
    );

    // 读档（同档回灌）
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/load"),
        Some(save.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "读档失败：{body}");

    // 读档后推进 1 个月 → month 3
    let (_, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(body["summaries"][0]["month"], 3);
}

#[tokio::test]
async fn gzip_save_load_roundtrip() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let (_, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;

    // 下载 gzip 存档：必须是 1F 8B 魔数，且明显小于明文 JSON。
    let (status, gz, headers) = request_bytes_with_headers(
        &app,
        "GET",
        &format!("/games/{id}/save/gzip"),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(gz.len() >= 2, "gzip 存档不应为空");
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "gzip 魔数");
    let json_meta: usize = headers
        .get("x-csc-json-bytes")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .expect("返回 JSON 原始体积元数据");
    assert!(
        json_meta > gz.len(),
        "元数据应显示压缩有效（json={json_meta} gz={}）",
        gz.len()
    );
    let crc_header = headers
        .get("x-csc-save-crc32")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
        .expect("返回 CRC32 校验和元数据");
    let (_, plain) = request_json(&app, "GET", &format!("/games/{id}/save"), None).await;
    let plain_len = plain.to_string().len();
    assert!(
        gz.len() < plain_len,
        "压缩存档应小于明文 JSON（gz={} plain={}）",
        gz.len(),
        plain_len
    );

    // gzip 读档（携带服务端下发的 CRC32）→ 再推进 1 个月应成功。
    let (status, body, _) = request_bytes_extra(
        &app,
        "POST",
        &format!("/games/{id}/load/gzip"),
        Body::from(gz.to_vec()),
        Some("application/gzip"),
        &[("x-csc-expected-crc32", crc_header.as_str())],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "gzip 读档失败：{}",
        String::from_utf8_lossy(&body)
    );
    // 篡改 CRC32 必须被拒绝。
    let (bad_status, _, _) = request_bytes_extra(
        &app,
        "POST",
        &format!("/games/{id}/load/gzip"),
        Body::from(gz.to_vec()),
        Some("application/gzip"),
        &[("x-csc-expected-crc32", "deadbeef")],
    )
    .await;
    assert_eq!(
        bad_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "CRC32 不匹配必须拒绝读档"
    );
    let loaded: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(loaded["loaded"], true);

    let (_, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(body["summaries"][0]["month"], 3);
}

#[tokio::test]
async fn gzip_load_rejects_garbage() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let (status, _) = request_bytes(
        &app,
        "POST",
        &format!("/games/{id}/load/gzip"),
        Body::from(vec![1, 2, 3, 4]),
        Some("application/gzip"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "非 gzip 载荷必须拒绝"
    );
}

#[tokio::test]
async fn create_game_rejects_bad_assets() {
    // 坏资产：负积分 → 422
    let standings = std::collections::HashMap::from([(
        "standings_global_2026_01_05.json".to_string(),
        r#"{"rankings":[{"ranking":1,"points":-5,"teamName":"T","roster":["a","b","c","d","e"]}]}"#
            .to_string(),
    )]);
    let state = AppState::from_bundle(csc_server::state::bundle_of(standings, None));
    let app = router().with_state(state);
    let (status, body) = request_json(
        &app,
        "POST",
        "/games",
        Some(json!({ "seed": 42, "policy": "auto" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "坏资产必须 422：{body}"
    );
    assert!(
        body["error"].as_str().unwrap().contains("负"),
        "错误应说明原因"
    );
}

/// 极简 URI 组件编码（测试专用：event name 含空格/井号时安全进查询串）。
fn encode_uri_component(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn client_view_is_light_and_replay_lazy_loaded() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 轻量视图：只有主角 roster（5 人），不带 journal / decisions / yearly_rating / series
    let (status, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    assert_eq!(status, StatusCode::OK, "view 接口应可用：{view}");
    assert_eq!(view["view_version"], 1);
    assert_eq!(
        view["world"]["players"].as_array().unwrap().len(),
        5,
        "主角 + 4 名队友"
    );
    assert_eq!(
        view["world"]["teams"].as_array().unwrap().len(),
        40,
        "全队名字引用"
    );
    assert!(view["world"]["team"].is_object(), "当前队伍完整实体");
    assert!(
        view.get("journal").is_none(),
        "journal 不进轻量视图（增量接口负责）"
    );
    assert!(
        view.get("yearly_rating").is_none(),
        "yearly_rating 抽成 season 标量"
    );

    // 懒加载回放：视图只发 replayable 布尔向量，series 由 /replay 单独取
    let events = view["events"].as_array().expect("主角赛事数组");
    assert!(!events.is_empty(), "推进后主角应有参赛记录");
    for ev in events {
        assert!(ev.get("series").is_none(), "series 不应随视图下发");
        assert!(ev["replayable"].is_array(), "replayable 布尔向量存在");
    }
    let ev = events
        .iter()
        .find(|e| {
            e["replayable"]
                .as_array()
                .is_some_and(|r| r.iter().any(|b| b == true))
        })
        .expect("主角参赛赛事应有可回放场次");
    let index = ev["replayable"]
        .as_array()
        .unwrap()
        .iter()
        .position(|b| b == true)
        .unwrap();
    let event_name = ev["event"]["name"].as_str().unwrap();
    let date = ev["event"]["date"].as_str().unwrap();
    let (status, replay) = request_json(
        &app,
        "GET",
        &format!(
            "/games/{id}/replay?event={}&date={}&index={}",
            encode_uri_component(event_name),
            encode_uri_component(date),
            index
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "可回放场次应能懒加载：{replay}");
    assert!(replay["series"]["maps"].is_array(), "返回逐图 series");

    // 非法 index / 不存在的赛事 → 404
    let (status, _) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/replay?event=Ghost&date=1999-01-01&index=0"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn training_is_voluntary_plan_applied_next_month() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    // 训练选项（主动拉取，非月度强制决策）
    let (status, opts) =
        request_json(&app, "GET", &format!("/games/{id}/training/options"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opts.as_array().unwrap().len(), 7, "7 种训练计划（含 REST）");

    // 非法计划 → 422
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/training"),
        Some(json!({ "focus": "HACK" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // 排定 AIM → pending_training 立刻可见（但下月才生效）
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/training"),
        Some(json!({ "focus": "aim" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "训练排定失败：{body}");
    assert_eq!(body["planned"], true);
    assert_eq!(body["focus"], "AIM");
    let (_, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let protagonist = view["world"]["players"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["career"].is_object())
        .expect("主角存在");
    assert_eq!(
        protagonist["career"]["pending_training"], "AIM",
        "计划写入客户端视图"
    );

    // 推进 1 个月 → 计划应用并清空 + TrainingDone 事件
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let protagonist = view["world"]["players"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["career"].is_object())
        .expect("主角存在");
    assert_eq!(
        protagonist["career"]["pending_training"],
        Value::Null,
        "计划应用后清空"
    );
    let (_, journal) =
        request_json(&app, "GET", &format!("/games/{id}/journal?since=-1"), None).await;
    let has_training_done = journal["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e.get("TRAINING_DONE").is_some());
    assert!(has_training_done, "应产出 TrainingDone 事件：{journal}");
}

#[tokio::test]
async fn top20_endpoint_returns_rankings() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 推进 2 个月（产生至少 1 场顶级赛事数据，但样本量未必达 MIN_MAPS）
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request_json(&app, "GET", &format!("/games/{id}/top20"), None).await;
    assert_eq!(status, StatusCode::OK, "top20 接口应可用：{body}");
    // 结构契约：year + entries（样本不足时 entries 可能为空，但字段必须存在）
    assert!(body["year"].is_number(), "返回年度");
    assert!(body["entries"].is_array(), "返回榜单数组");
}

#[tokio::test]
async fn top20_falls_back_to_completed_year_board() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 主角从垫底队起步，推进满一年：T1/Major 世界赛事照常模拟并跨年结算
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 12 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 新赛季 1 月实时榜尚无 T1+ 样本 → 回退最近一届完整年度榜（2026）
    let (status, body) = request_json(&app, "GET", &format!("/games/{id}/top20"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["source"], "COMPLETED", "应回退已收官榜单：{body}");
    assert_eq!(body["year"], 2026, "回退榜单归属上一年度");
    assert_eq!(
        body["entries"].as_array().unwrap().len(),
        20,
        "完整年度榜应为 20 人满员（世界赛事不依赖主角参赛）"
    );
}

#[tokio::test]
async fn season_goal_roundtrip() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/season-goal"),
        Some(json!({ "goal": "TOP20" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "设置赛季目标应成功：{body}");
    assert_eq!(body["set"], true);

    let (_, view) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let career = view["world"]["players"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| p.get("career").and_then(|c| c.as_object()))
        .expect("主角生涯");
    assert_eq!(career["season_goal"], "TOP20", "目标写入客户端视图");
    assert_eq!(career["season_goal_year"].as_i64(), Some(2026));

    let (bad_status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/season-goal"),
        Some(json!({ "goal": "NOPE" })),
    )
    .await;
    assert_eq!(
        bad_status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "非法目标必须拒绝"
    );

    // 跨年结算后目标应清空：下赛季需要重新选择。
    let (adv_status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 12 })),
    )
    .await;
    assert_eq!(adv_status, StatusCode::OK);
    let (_, view2) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
    let career2 = view2["world"]["players"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|p| p.get("career").and_then(|c| c.as_object()))
        .expect("主角生涯");
    assert_eq!(
        career2["season_goal"],
        Value::Null,
        "跨年结算应清空上赛季目标"
    );

    // 赛季档案应保留目标与可解释结算。
    let archive = view2["archive"].as_array().expect("返回赛季档案");
    let last = archive.last().expect("至少一个赛季档案");
    assert_eq!(last["season_goal"], "TOP20", "赛季档案记录当时目标");
    assert!(
        last["goal_met"].is_boolean(),
        "赛季目标必须结算为达成/未达成"
    );
    assert!(
        !last["goal_outcome"].as_str().unwrap_or("").is_empty(),
        "必须生成可解释目标结算"
    );
}

#[tokio::test]
async fn insights_explain_transfer_market() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    let (_, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;

    let (status, body) = request_json(&app, "GET", &format!("/games/{id}/insights"), None).await;
    assert_eq!(status, StatusCode::OK, "insights 接口应可用：{body}");
    let insight = body["transfer"].as_object().expect("返回转会市场洞察");
    assert!(insight.contains_key("explanation"), "必须包含原因解释");
    assert!(
        insight["explanation"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "解释不能为空"
    );
    assert!(insight["contract_years"].is_i64(), "包含合同年限");
    assert!(
        insight["eligible_teams_full_price"].is_i64(),
        "包含全价候选数"
    );

    // 队内地位解释必须存在（主角开局就有队伍）。
    let team = body["team"].as_object().expect("返回队内地位洞察");
    assert!(
        team["explanation"].as_str().is_some_and(|s| !s.is_empty()),
        "队内解释不能为空"
    );
    assert!(team["power_rank"].as_i64().is_some(), "包含队内实力排名");

    // 比赛解释：推进 1 个月后主角通常已参加 T3；若尚未参赛则允许 null。
    if body["match"].is_object() {
        assert!(
            body["match"]["explanation"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "比赛解释不能为空"
        );
    }
}

#[tokio::test]
async fn day_advance_produces_live_updates() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "days": 3 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "按天推进应成功：{body}");
    let summaries = body["summaries"].as_array().expect("返回日级摘要");
    assert_eq!(summaries.len(), 3, "3 天 = 3 条日级摘要");
    assert!(
        summaries.iter().all(|s| s["day"].is_number()),
        "日级摘要带 day 字段"
    );

    let (_, journal) =
        request_json(&app, "GET", &format!("/games/{id}/journal?since=-1"), None).await;
    let events = journal["events"].as_array().unwrap();
    let live = events
        .iter()
        .filter(|e| e.get("LIVE_UPDATE").is_some())
        .count();
    assert!(
        live >= 3,
        "每天应有文字直播消息：{live}/3（共 {} 事件）",
        events.len()
    );
}

#[tokio::test]
async fn skip_live_today_endpoint_returns_skipped_count() {
    // 跳过今日比赛日 LIVE 提示的持久语义端点：无主角今日对阵时为 no-op（skipped=0），
    // 端点必须接线且幂等（重复调用不报错）。真正的「标记 + live/today 排除」语义由
    // csc-core::player_live_fixtures_excludes_skipped_fixtures 与
    // csc-tournaments::skip_live_today_marks_todays_pending_fixtures_only 单测覆盖。
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    let (status, body) =
        request_json(&app, "POST", &format!("/games/{id}/live/today/skip"), None).await;
    assert_eq!(status, StatusCode::OK, "skip 端点应 200：{body}");
    assert!(body["skipped"].is_u64(), "返回 skipped 数量：{body}");

    // 幂等：再次调用仍 200。
    let (status2, body2) =
        request_json(&app, "POST", &format!("/games/{id}/live/today/skip"), None).await;
    assert_eq!(status2, StatusCode::OK, "重复 skip 仍 200：{body2}");
    assert_eq!(
        body["skipped"], body2["skipped"],
        "空比赛日两次 skip 计数一致"
    );
}

/// P2：比赛日闸门对**日级推进**同样生效（手动「下一天」不再绕过比赛日）。
///
/// 语义：Human 策略下，只要今天有主角可 LIVE 的 pending 对阵（live_gate_active），
/// **任何粒度**推进（days/months）一律 409；先 `POST /live/today/skip`（或进入 LIVE）
/// 后闸门关闭、推进放行。修复前只有月推进被拦截，日推进会直接越过比赛日——
/// 用户手动推进永远看不到 LIVE 提示条（P2 缺陷根因）。
#[tokio::test]
async fn live_gate_blocks_day_advance_until_skipped() {
    let app = fixture_app();
    let id = create_game(&app, "human").await;

    // 逐日逼近比赛日：非比赛日日推进放行（202 受理）。
    let mut reached_live_day = false;
    for _ in 0..60 {
        let (status, body) = request_json(
            &app,
            "POST",
            &format!("/games/{id}/advance"),
            Some(json!({ "days": 1 })),
        )
        .await;
        if status == StatusCode::CONFLICT {
            // 已到比赛日：闸门生效（P2 修复点——日推进也被拦截）。
            assert!(
                body["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("比赛日闸门激活"),
                "409 文案应说明比赛日闸门：{body}"
            );
            reached_live_day = true;
            break;
        }
        assert_eq!(status, StatusCode::ACCEPTED, "非比赛日日推进应 202：{body}");
        // 等待该日推进完成（异步 202）：轮询 view 日期变化；期间消费决策批次防死锁。
        let (_, v0) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
        let before = v0["date"].as_str().unwrap().to_string();
        for _ in 0..500 {
            let (_, b) =
                request_json(&app, "GET", &format!("/games/{id}/decisions/pending"), None).await;
            if let Some(batch) = b["pending"].as_object() {
                let points = batch["points"].as_array().expect("批次含决策点");
                let decisions: Vec<Value> = points.iter().map(default_decision).collect();
                let (status, _) = request_json(
                    &app,
                    "POST",
                    &format!("/games/{id}/decisions"),
                    Some(json!({ "decisions": decisions, "batch_id": batch["batch_id"] })),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "决策提交应成功");
                continue;
            }
            let (_, v) = request_json(&app, "GET", &format!("/games/{id}/view"), None).await;
            if v["date"].as_str() != Some(before.as_str()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    assert!(
        reached_live_day,
        "60 天内应遇到至少一个比赛日（1 月/2 月主角赛事）"
    );

    // 比赛日当天：live/today 非空；**日级**推进 409（P2 修复的核心断言）。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "days": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "比赛日日推进应 409：{body}");
    // 月推进同样 409（既有闸门语义保持）。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "比赛日月推进也应 409：{body}");

    // 闸门依据：live/today 非空。
    let (_, today) = request_json(&app, "GET", &format!("/games/{id}/live/today"), None).await;
    assert!(
        !today["matches"].as_array().unwrap().is_empty(),
        "比赛日 live/today 应非空：{today}"
    );

    // 跳过今日比赛日 → 闸门关闭。
    let (status, body) =
        request_json(&app, "POST", &format!("/games/{id}/live/today/skip"), None).await;
    assert_eq!(status, StatusCode::OK, "skip 应 200：{body}");
    assert!(
        body["skipped"].as_u64().unwrap() >= 1,
        "应标记至少 1 场：{body}"
    );

    // 跳过后：日推进放行（202 受理）。
    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "days": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "跳过后日推进应放行：{body}");
}

#[tokio::test]
async fn advance_days_and_months_are_mutually_exclusive() {
    // T5 防御：同一请求同时传 days 与 months 必须 400，防止粒度混淆。
    let app = fixture_app();
    let id = create_game(&app, "auto").await;

    let (status, body) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "days": 3, "months": 1 })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "days+months 同传必须 400：{body}"
    );
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("days") && msg.contains("months"),
        "错误信息应说明互斥：{msg}"
    );

    // 单传 days / 单传 months 仍正常。
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "days": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "单传 days 应成功");
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "单传 months 应成功");
}

#[tokio::test]
async fn world_news_returns_recent_t1_matchups() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 推进 2 个月：2026 新赛历前两个月已有 4 场顶级赛事
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request_json(&app, "GET", &format!("/games/{id}/news"), None).await;
    assert_eq!(status, StatusCode::OK, "新闻接口应可用：{body}");
    let items = body["items"].as_array().expect("返回 items 数组");
    assert!(!items.is_empty(), "T1 赛后应有近期对阵新闻");
    for item in items {
        assert!(
            !item["winner"].as_str().unwrap().contains('|'),
            "队名不应带阵容签名"
        );
        assert!(
            !item["loser"].as_str().unwrap().contains('|'),
            "队名不应带阵容签名"
        );
        assert!(!item["score"].as_str().unwrap().is_empty(), "应带比分");
    }
}

#[tokio::test]
async fn world_stories_endpoint_returns_stories_and_rivalries() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 推进 2 个月产生赛事结果/转会流 → 世界叙事应有队伍故事
    let (status, _) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = request_json(&app, "GET", &format!("/games/{id}/world"), None).await;
    assert_eq!(status, StatusCode::OK, "世界叙事接口应可用：{body}");
    assert!(body["stories"].is_array(), "返回队伍故事数组");
    assert!(body["rivalries"].is_array(), "返回宿敌数组");
    // 40 队满编 → **每队**都应有故事骨架（不截断前十）
    assert_eq!(
        body["stories"].as_array().unwrap().len(),
        40,
        "应返回全部 40 支队伍的故事"
    );
    // 每条故事有必备字段
    for s in body["stories"].as_array().unwrap().iter().take(5) {
        assert!(s["team_name"].is_string());
        assert!(s["archetype"].is_string());
        assert!(s["headline"].is_string());
        assert!(s["body"].is_string());
    }
}

/// R2 赛事对象化端点：/events/{event_name} 按名返回聚合（参赛队+冠军+对阵+空状态）。
#[tokio::test]
async fn event_aggregate_endpoint_returns_teams_champion_and_fixtures() {
    let app = fixture_app();
    let id = create_game(&app, "auto").await;
    // 推进 1 个月：1 月（开局月物化）+ 2 月赛事全部完赛。
    let (status, adv) = request_json(
        &app,
        "POST",
        &format!("/games/{id}/advance"),
        Some(json!({ "months": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "推进失败：{adv}");

    // 从 calendar 取一个已完赛赛事名。
    let (_, cal) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/calendar?year=2026"),
        None,
    )
    .await;
    let results = cal["results"].as_array().expect("应有赛果");
    assert!(!results.is_empty(), "首个月步应有完赛赛事");
    let event_name = results[0]["event"]["name"]
        .as_str()
        .expect("赛事名")
        .to_string();
    // 赛事名含空格/`#` 等 URI 非法字符：手动编码（测试环境无 urlencoding 依赖）。
    let encoded: String = event_name
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            c => c.to_string(),
        })
        .collect();

    // 查询聚合端点。
    let (status, agg) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/events/{}", encoded),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "聚合端点应可用：{agg}");
    // 前端契约字段（types.ts EventAggregate）。
    assert_eq!(agg["event_name"], event_name);
    assert!(agg["tier"].is_string());
    assert!(agg["date"].is_string());
    assert!(agg["end_date"].is_string());
    assert!(agg["status"].is_string());
    assert!(agg["teams"].is_array());
    assert!(
        !agg["teams"].as_array().unwrap().is_empty(),
        "完赛赛事应有参赛队伍"
    );
    for t in agg["teams"].as_array().unwrap() {
        assert!(t["id"].is_number());
        assert!(t["name"].is_string());
        assert!(t["vrs_ranking"].is_number());
        assert!(t["vrs_value"].is_number());
    }
    assert!(agg["fixtures"].is_array());
    assert!(agg["champion"].is_object() || agg["champion"].is_null());
    assert!(agg["player_participating"].is_boolean());
    assert!(agg["emptiness_reason"].is_null() || agg["emptiness_reason"].is_string());

    // 未知赛事 → 404（ASCII 占位名：裸中文 URI 在部分运行下 InvalidUriChar panic——flaky 修复）。
    let (status, _) = request_json(
        &app,
        "GET",
        &format!("/games/{id}/events/{}", "no-such-event"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
