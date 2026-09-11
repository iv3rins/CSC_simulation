//! 客户端视图体积画像（性能优化 P0）：20 年生涯后
//! 完整 GameState JSON vs 轻量 ClientState JSON 的大小与构建/序列化耗时。
//!
//! 用法：
//!   cargo run -p csc-core --release --example client_profile -- ../assets
//!
//! 设计目标：ClientState 是网页端 `GET /games/{id}/view` 的载荷，
//! 应比全量快照小 1~2 个数量级（20 年末完整存档约 68MB）。

use std::time::Instant;

use csc_core::{CalibrationAssets, Engine};
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
    let seed: u64 = 42;
    let months = 240;

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
            "ViewProfile",
            csc_domain::tier::Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
            None,
        )
        .expect("主角创建失败");

    let milestones = [60, 120, 180, 240];
    let mut auto = csc_core::engine::auto_decision();
    let mut samples: Vec<serde_json::Value> = Vec::new();
    for m in 1..=months {
        engine.advance_month(&mut auto).expect("推进失败");
        if milestones.contains(&m) {
            let t_full = Instant::now();
            let full = engine.snapshot();
            let full_build_ms = t_full.elapsed().as_secs_f64() * 1000.0;
            let full_json = serde_json::to_string(&full).expect("全量序列化失败");

            let t_view = Instant::now();
            let view = engine.client_state();
            let view_build_ms = t_view.elapsed().as_secs_f64() * 1000.0;
            let view_json = serde_json::to_string(&view).expect("视图序列化失败");

            samples.push(serde_json::json!({
                "month": m,
                "full_state": {
                    "bytes": full_json.len(),
                    "build_ms": full_build_ms,
                    "journal_events": full.journal.len(),
                    "tournament_results": full.events.len(),
                    "world_players": full.world.players.len(),
                },
                "client_view": {
                    "bytes": view_json.len(),
                    "build_ms": view_build_ms,
                    "world_players": view.world.players.len(),
                    "world_team_refs": view.world.teams.len(),
                    "archive_seasons": view.archive.len(),
                    "events": view.events.len(),
                    "replayable_matches": view.events.iter().map(|e| e.replayable.iter().filter(|b| **b).count()).sum::<usize>(),
                },
                "ratio": view_json.len() as f64 / full_json.len() as f64,
            }));
            eprintln!("{m} 个月完成");
        }
    }

    let out = serde_json::json!({ "config": { "seed": seed, "months": months, "assets": assets_dir }, "samples": samples });
    println!("{}", serde_json::to_string(&out).expect("输出序列化失败"));
    eprintln!(
        "完成：全量/视图逐里程碑已输出（末样本视图 {:.1}KB）",
        samples
            .last()
            .and_then(|s| s["client_view"]["bytes"].as_u64())
            .map(|b| b as f64 / 1e3)
            .unwrap_or_default()
    );
}
