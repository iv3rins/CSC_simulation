use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use csc_server::{
    game::{GameLoader, GameManager},
    routes::router,
    state::{AppState, bundle_of},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;

fn fixture() -> (AppState, PathBuf) {
    let rankings: Vec<Value> = (0..40).map(|i|json!({"ranking":i+1,"points":2000-i*45,"teamName":format!("Team{i}"),"roster":(0..5).map(|p|format!("T{i}P{p}")).collect::<Vec<_>>()})).collect();
    let mut bundle = bundle_of(
        std::collections::HashMap::from([(
            "standings_global_2026_01_05.json".into(),
            json!({"rankings":rankings}).to_string(),
        )]),
        None,
    );
    bundle.narrative = Some(
        csc_core::narrative::NarrativeContent::from_json_str(include_str!(
            "../../../../assets/narrative/zh-CN/career.json"
        ))
        .unwrap(),
    );
    let dir = std::env::temp_dir().join(format!(
        "csc-career-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut state = AppState::from_bundle(bundle);
    state.games = manager(&state, dir.clone());
    (state, dir)
}

fn manager(state: &AppState, dir: PathBuf) -> Arc<GameManager> {
    Arc::new(GameManager::with_config(
        Some(dir),
        None,
        Some(Arc::new(GameLoader {
            rating_profile: None,
            narrative: state.assets.narrative.clone(),
        })),
    ))
}

async fn request(app: &Router, method: &str, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(if value.is_null() {
                    Body::empty()
                } else {
                    Body::from(value.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn scene_commit_is_idempotent_after_consumption_and_disk_restore() {
    let (mut state, dir) = fixture();
    let app = router().with_state(state.clone());
    let (status, created) = request(&app, "POST", "/games", json!({"story":true,"seed":42})).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["game_id"].as_u64().unwrap();
    let base = format!("/games/{id}");
    let (_, career) = request(&app, "GET", &format!("{base}/career"), Value::Null).await;
    assert_eq!(career["capabilities"]["advance_world"], false);
    let choice = json!({"generation":career["generation"],"request_id":"once","scene_instance_id":career["scene"]["instance_id"],"choice_id":career["scene"]["choices"][0]["id"]});
    let choices_path = format!("{base}/career/choices");
    let (a, b) = tokio::join!(
        request(&app, "POST", &choices_path, choice.clone()),
        request(&app, "POST", &choices_path, choice.clone())
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1, b.1);
    let (_, history) = request(&app, "GET", &format!("{base}/career/history"), Value::Null).await;
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    let (_, before) = request(&app, "GET", &format!("{base}/state"), Value::Null).await;
    let mut bad = choice.clone();
    bad["choice_id"] = json!("DIFFERENT");
    assert_eq!(
        request(&app, "POST", &format!("{base}/career/choices"), bad)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "GET", &format!("{base}/state"), Value::Null)
            .await
            .1,
        before
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("{base}/advance"),
            json!({"months":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // Stop the actor without DELETE /games semantics (which deliberately deletes its save).
    state.games.get(id).unwrap().shutdown();
    state.games = manager(&state, dir);
    let restored = router().with_state(state.clone());
    let retry = request(
        &restored,
        "POST",
        &format!("{base}/career/choices"),
        choice.clone(),
    )
    .await;
    assert_eq!(retry.0, StatusCode::OK, "{}", retry.1);
    assert_eq!(retry.1, a.1);
    assert_eq!(
        request(&restored, "GET", &format!("{base}/state"), Value::Null)
            .await
            .1,
        before
    );
    let (loaded, message) = request(&restored, "POST", &format!("{base}/load"), before).await;
    assert_eq!(loaded, StatusCode::OK, "{message}");
    assert_eq!(
        request(&restored, "POST", &format!("{base}/career/choices"), choice)
            .await
            .0,
        StatusCode::CONFLICT
    );
    state.games.shutdown(id);
}

#[tokio::test]
async fn failed_persistence_preserves_scene_and_world() {
    let (mut state, dir) = fixture();
    std::fs::write(&dir, b"this path is a file, not a directory").unwrap();
    state.games = manager(&state, dir);
    let app = router().with_state(state.clone());
    let (_, created) = request(&app, "POST", "/games", json!({"story":true})).await;
    let id = created["game_id"].as_u64().unwrap();
    let base = format!("/games/{id}");
    let (_, career) = request(&app, "GET", &format!("{base}/career"), Value::Null).await;
    let (_, before) = request(&app, "GET", &format!("{base}/state"), Value::Null).await;
    let choice = json!({"generation":career["generation"],"request_id":"fail-write","scene_instance_id":career["scene"]["instance_id"],"choice_id":career["scene"]["choices"][0]["id"]});
    assert_eq!(
        request(&app, "POST", &format!("{base}/career/choices"), choice)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "GET", &format!("{base}/state"), Value::Null)
            .await
            .1,
        before
    );
    let (_, history) = request(&app, "GET", &format!("{base}/career/history"), Value::Null).await;
    assert!(history["items"].as_array().unwrap().is_empty());
    state.games.shutdown(id);
}
