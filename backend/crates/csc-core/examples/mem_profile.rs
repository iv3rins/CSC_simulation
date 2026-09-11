//! 分配热点画像（第四阶段补充）：dhat 堆分析 12 个月推进，输出分配热点。
//!
//! 用法：
//!   cargo run -p csc-core --release --example mem_profile -- ../assets
//! dhat 输出写到 stderr（含 dhat-heap.json，可用 dhat-rs HTML 查看）。

use csc_core::{CalibrationAssets, Engine};
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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
    let _profiler = dhat::Profiler::new_heap();

    let (standings, baseline, ratings, profile) = load_assets(assets_dir);
    let calibration = CalibrationAssets {
        text: None,
        baseline: baseline.as_deref(),
        ratings: ratings.as_deref(),
        rating_profile: profile.as_deref(),
    };
    let mut engine =
        Engine::load_from_standings(&standings, &calibration, 42, SimClock::of(2026, 1, 1))
            .expect("装载失败");
    let bottom = csc_util::id::TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
    engine
        .create_protagonist(
            "Mem",
            csc_domain::tier::Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(42 ^ 0x5EED_C0DE),
            None,
        )
        .expect("主角创建失败");
    let mut auto = csc_core::engine::auto_decision();
    engine.run_season(12, &mut auto).expect("推进失败");
    eprintln!("mem_profile: 12 个月完成");
}
