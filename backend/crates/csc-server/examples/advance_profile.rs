//! 服务端推进吞吐画像：真实资产 + GameManager 批量推进，
//! 验证「批次结束才构建完整快照」的性能契约，并输出 /view 与 /state 的体积。
//!
//! 用法：
//!   cargo run -p csc-server --release --example advance_profile -- ../assets 120
//!
//! 旧实现逐月 `engine.snapshot()` 会随历史线性放大（20 年末单次 ≈56ms 且
//! 产生 68MB clone）；新实现只在批次结束构建一次，月步只广播标量摘要。

use std::sync::Arc;
use std::time::Instant;

use csc_core::{CalibrationAssets, Engine};
use csc_server::game::{GameManager, Policy};
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;

type AssetBundle = (
    Vec<(String, String)>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn load_assets(dir: &str) -> AssetBundle {
    let mut standings = Vec::new();
    let mut baseline = None;
    let mut ratings = None;
    let mut profile = None;
    for entry in std::fs::read_dir(dir).expect("资产目录不可读") {
        let path = entry.expect("遍历失败").path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.starts_with("standings_") && name.ends_with(".json") {
            standings.push((name, std::fs::read_to_string(&path).expect("读取失败")));
        } else if name == "roles_baseline.json" {
            baseline = Some(std::fs::read_to_string(&path).expect("读取失败"));
        } else if name == "player_ratings.json" {
            ratings = Some(std::fs::read_to_string(&path).expect("读取失败"));
        } else if name == "rating_profile.json" {
            profile = Some(std::fs::read_to_string(&path).expect("读取失败"));
        }
    }
    (standings, baseline, ratings, profile)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let assets_dir = args.get(1).map(String::as_str).unwrap_or("../assets");
    let months: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120);
    let seed = 42u64;

    let (standings, baseline, ratings, profile) = load_assets(assets_dir);
    let calibration = CalibrationAssets {
        text: None,
        baseline: baseline.as_deref(),
        ratings: ratings.as_deref(),
        rating_profile: profile.as_deref(),
    };
    let mut engine =
        Engine::load_from_standings(&standings, &calibration, seed, SimClock::of(2026, 1, 1))
            .expect("装载失败");
    let bottom = csc_util::id::TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
    engine
        .create_protagonist(
            "ServerProfile",
            csc_domain::tier::Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
            None,
        )
        .expect("主角创建失败");

    let mgr = Arc::new(GameManager::new());
    let id = mgr.create(engine, Policy::Auto);
    let entry = mgr.get(id).expect("游戏存在");

    let t = Instant::now();
    let summaries = entry
        .advance_blocking(months, std::time::Duration::from_secs(600))
        .expect("推进失败");
    let advance_ms = t.elapsed().as_secs_f64() * 1000.0;

    let view = entry.view.lock().expect("view 锁").clone();
    let view_json = serde_json::to_string(&view).expect("view 序列化失败");
    let state = entry.state().expect("取状态");
    let state_json = serde_json::to_string(&state).expect("state 序列化失败");

    let out = serde_json::json!({
        "config": { "seed": seed, "months": months, "assets": assets_dir },
        "advance_ms": advance_ms,
        "steps": summaries.len(),
        "client_view_bytes": view_json.len(),
        "game_state_bytes": state_json.len(),
        "ratio": view_json.len() as f64 / state_json.len() as f64,
    });
    println!("{}", serde_json::to_string(&out).expect("输出序列化失败"));
    mgr.shutdown(id);
}
