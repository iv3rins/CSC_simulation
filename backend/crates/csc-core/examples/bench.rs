//! 性能画像基准（第四阶段）：1 月 / 1 赛季 / 5 年 / 20 年耗时、月推进 p50/p95/p99、
//! 存档大小与序列化耗时、事件/赛事/VRS 历史增长速度、查询耗时。输出 JSON 到 stdout。
//!
//! 用法（release 构建）：
//!   cargo run -p csc-core --release --example bench -- ../assets > bench.json
//!
//! 注意：本基准不含 RSS（进程内存由外部采样，见 PowerShell 脚本）。

use std::time::{Duration, Instant};

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

fn percentile(sorted: &mut [Duration], p: f64) -> f64 {
    sorted.sort();
    let idx = ((sorted.len() - 1) as f64 * p) as usize;
    sorted[idx].as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let assets_dir = args.get(1).map(String::as_str).unwrap_or("../assets");
    let seed: u64 = 42;
    let (standings, baseline, ratings, profile) = load_assets(assets_dir);

    let t_load0 = Instant::now();
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
            "Bench",
            csc_domain::tier::Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
            None,
        )
        .expect("主角创建失败");
    let load_ms = t_load0.elapsed().as_secs_f64() * 1000.0;

    // 逐月推进计时（240 个月）
    let mut monthly: Vec<Duration> = Vec::with_capacity(240);
    let mut growth_samples: Vec<serde_json::Value> = Vec::new();
    let mut auto = csc_core::engine::auto_decision();
    for m in 1..=240 {
        let t = Instant::now();
        engine.advance_month(&mut auto).expect("推进失败");
        monthly.push(t.elapsed());
        if m % 12 == 0 {
            growth_samples.push(serde_json::json!({
                "month": m,
                "journal_events": engine.journal().all().len(),
                "tournament_results": engine.tournaments().results().len(),
                "vrs_match_history": engine.vrs().database().match_history_len(),
                "world_players": engine.world().all_players().len()
            }));
        }
    }

    let mut sorted = monthly.clone();
    let p50 = percentile(&mut sorted, 0.50);
    let p95 = percentile(&mut sorted, 0.95);
    let p99 = percentile(&mut sorted, 0.99);
    let month_mean = monthly
        .iter()
        .map(|d| d.as_secs_f64() * 1000.0)
        .sum::<f64>()
        / monthly.len() as f64;
    let season_ms = monthly[..12].iter().sum::<Duration>().as_secs_f64() * 1000.0;
    let five_year_ms = monthly[..60].iter().sum::<Duration>().as_secs_f64() * 1000.0;
    let twenty_year_ms = monthly.iter().sum::<Duration>().as_secs_f64() * 1000.0;

    // 存档序列化/反序列化（20 年末快照）
    let t_ser = Instant::now();
    let json = serde_json::to_string(&engine.snapshot()).expect("快照序列化失败");
    let ser_ms = t_ser.elapsed().as_secs_f64() * 1000.0;
    let save_bytes = json.len();
    let t_de = Instant::now();
    let state = csc_core::state::GameState::from_json_str(&json).expect("反序列化失败");
    let de_ms = t_de.elapsed().as_secs_f64() * 1000.0;

    // 查询耗时
    let t_q = Instant::now();
    let top20 = engine.query().top20();
    let top20_ms = t_q.elapsed().as_secs_f64() * 1000.0;
    let t_rank = Instant::now();
    let _rankings = engine.query().rankings();
    let rankings_ms = t_rank.elapsed().as_secs_f64() * 1000.0;
    let t_sum = Instant::now();
    let _summary = engine.query().world_summary();
    let summary_ms = t_sum.elapsed().as_secs_f64() * 1000.0;

    // 存档→恢复一致性（save→load→继续推进必须与原路径一致）
    let mut restored = Engine::empty(seed, SimClock::of(2026, 1, 1));
    restored.restore(state);
    let t_adv = Instant::now();
    let mut auto2 = csc_core::engine::auto_decision();
    restored.advance_month(&mut auto2).expect("恢复后推进失败");
    let _ = t_adv;

    let out = serde_json::json!({
        "config": { "seed": seed, "months": 240, "assets": assets_dir },
        "load_ms": load_ms,
        "monthly_ms": { "mean": month_mean, "p50": p50, "p95": p95, "p99": p99, "min": monthly.iter().map(|d| d.as_secs_f64()*1000.0).fold(f64::INFINITY, f64::min), "max": monthly.iter().map(|d| d.as_secs_f64()*1000.0).fold(0.0, f64::max) },
        "totals_ms": { "season_12m": season_ms, "five_years_60m": five_year_ms, "twenty_years_240m": twenty_year_ms },
        "save": { "bytes": save_bytes, "serialize_ms": ser_ms, "deserialize_ms": de_ms },
        "query_ms": { "top20": top20_ms, "top20_entries": top20.len(), "rankings": rankings_ms, "summary": summary_ms },
        "growth": growth_samples
    });
    println!("{}", serde_json::to_string(&out).expect("序列化失败"));
    eprintln!(
        "完成：20 年 {:.1}s，存档 {:.1}MB",
        twenty_year_ms / 1000.0,
        save_bytes as f64 / 1e6
    );
}
