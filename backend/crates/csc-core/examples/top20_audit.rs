//! TOP20 专项统计审计（第三阶段）：N 个确定性 seed × 20 年模拟，
//! 输出历年榜单的角色/评分/荣誉统计 JSON 到 stdout。
//!
//! 用法（release 构建）：
//!   cargo run -p csc-core --release --example top20_audit -- ../assets 50 > top20_audit.json
//!
//! 禁止任何角色补偿——本工具只测量「个人年度赛场表现」模型的自然分布。

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

/// 读资产目录（与 csc-app 相同的装载逻辑）。
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
    let n_seeds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let years: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let months = years * 12;

    let (standings, baseline, ratings, profile) = load_assets(assets_dir);

    let mut per_seed: Vec<serde_json::Value> = Vec::new();
    let mut role_counts: std::collections::HashMap<String, u64> = Default::default();
    let mut role_rating_sum: std::collections::HashMap<String, f64> = Default::default();
    let mut role_rating_n: std::collections::HashMap<String, u64> = Default::default();
    let mut all_ratings: Vec<f64> = Vec::new();
    let mut rank_vs_mvp: Vec<(f64, f64)> = Vec::new();
    let mut rank_vs_rating: Vec<(f64, f64)> = Vec::new();
    let mut tier_counts: std::collections::HashMap<String, u64> = Default::default();
    let mut igl_cases: Vec<serde_json::Value> = Vec::new();
    let mut anomalies: Vec<serde_json::Value> = Vec::new();
    // 连续入选：player_name → 最近年份与连续年数
    let mut streaks: std::collections::HashMap<String, (i32, u32, u32)> = Default::default(); // (last_year, cur_streak, max_streak)
    let mut total_entries = 0u64;

    let t0 = Instant::now();
    for seed in 1..=n_seeds {
        let calibration = CalibrationAssets {
            text: None,
            baseline: baseline.as_deref(),
            ratings: ratings.as_deref(),
            rating_profile: profile.as_deref(),
        };
        let mut engine =
            Engine::load_from_standings(&standings, &calibration, seed, SimClock::of(2026, 1, 1))
                .expect("装载失败");
        let bottom =
            csc_util::id::TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
        engine
            .create_protagonist(
                "Audit",
                csc_domain::tier::Tier::Tier4,
                bottom,
                &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
                None,
            )
            .expect("主角创建失败");
        let mut auto = csc_core::engine::auto_decision();
        engine.run_season(months, &mut auto).expect("推进失败");

        // 赛事级别分布（本 seed 全程）
        for r in engine.tournaments().results() {
            let tier = format!("{:?}", r.event.tier);
            *tier_counts.entry(tier).or_insert(0) += 1;
        }

        let mut boards_json: Vec<serde_json::Value> = Vec::new();
        for board in engine.tournaments().top20_history() {
            let mut entries: Vec<serde_json::Value> = Vec::new();
            for e in &board.entries {
                total_entries += 1;
                let role = engine
                    .world()
                    .player(e.player_id)
                    .map(|p| format!("{:?}", p.role))
                    .unwrap_or_else(|| "UNKNOWN".into());
                let serde_role = engine
                    .world()
                    .player(e.player_id)
                    .and_then(|p| serde_json::to_string(&p.role).ok())
                    .unwrap_or_else(|| "\"UNKNOWN\"".into());
                *role_counts.entry(serde_role.clone()).or_insert(0) += 1;
                *role_rating_sum.entry(serde_role.clone()).or_insert(0.0) += e.weighted_rating;
                *role_rating_n.entry(serde_role.clone()).or_insert(0) += 1;
                all_ratings.push(e.weighted_rating);
                rank_vs_mvp.push((e.rank as f64, (e.mvp_count as f64) + (e.evp_count as f64)));
                rank_vs_rating.push((e.rank as f64, e.weighted_rating));
                // 连续入选
                let key = e.player_name.clone();
                let (last_year, cur, mx) = streaks.get(&key).copied().unwrap_or((0, 0, 0));
                let (cur, mx) = if last_year == board.year - 1 {
                    (cur + 1, mx.max(cur + 1))
                } else {
                    (1, mx.max(1))
                };
                streaks.insert(key, (board.year, cur, mx));
                // 纯指挥案例
                if role == "Igl" {
                    igl_cases.push(serde_json::json!({
                        "name": e.player_name, "year": board.year, "rank": e.rank,
                        "rating": e.weighted_rating, "maps": e.maps,
                        "mvp": e.mvp_count, "evp": e.evp_count, "score": e.score
                    }));
                }
                entries.push(serde_json::json!({
                    "name": e.player_name, "role": serde_role, "rank": e.rank,
                    "score": e.score, "rating": e.weighted_rating,
                    "honor": e.honor_points, "playoff": e.playoff_rating,
                    "maps": e.maps, "mvp": e.mvp_count, "evp": e.evp_count,
                    "wildcard": e.wildcard
                }));
            }
            // 异常样本：rating 排名与名次偏差（board 内）
            let mut sorted: Vec<&csc_tournaments::top20::Top20BoardEntry> =
                board.entries.iter().collect();
            sorted.sort_by(|a, b| b.weighted_rating.total_cmp(&a.weighted_rating));
            let rating_rank: std::collections::HashMap<u32, usize> = sorted
                .iter()
                .enumerate()
                .map(|(i, e)| (e.player_id.0, i + 1))
                .collect();
            for e in &board.entries {
                let rr = *rating_rank.get(&e.player_id.0).unwrap_or(&99) as i64;
                let drift = rr - e.rank as i64;
                if drift.abs() >= 5 {
                    anomalies.push(serde_json::json!({
                        "name": e.player_name, "year": board.year, "rank": e.rank,
                        "rating_rank": rr, "rating": e.weighted_rating,
                        "mvp": e.mvp_count, "evp": e.evp_count, "score": e.score
                    }));
                }
            }
            boards_json.push(serde_json::json!({ "year": board.year, "entries": entries }));
        }
        per_seed.push(serde_json::json!({ "seed": seed, "boards": boards_json }));
        eprintln!(
            "seed {seed} 完成（{:.1}s 累计）",
            t0.elapsed().as_secs_f64()
        );
    }

    // 聚合统计
    let mut role_dist: Vec<serde_json::Value> = role_counts
        .iter()
        .map(|(r, n)| serde_json::json!({
            "role": r, "count": n, "pct": *n as f64 * 100.0 / total_entries as f64,
            "avg_rating": role_rating_sum.get(r).copied().unwrap_or(0.0) / *role_rating_n.get(r).unwrap_or(&1) as f64,
            "n": role_rating_n.get(r).copied().unwrap_or(0)
        }))
        .collect();
    role_dist.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));

    let mut ratings_sorted = all_ratings.clone();
    ratings_sorted.sort_by(|a, b| a.total_cmp(b));
    let q = |p: f64| ratings_sorted[((ratings_sorted.len() - 1) as f64 * p) as usize];
    let mean = all_ratings.iter().sum::<f64>() / all_ratings.len() as f64;

    let pearson = |xs: &[(f64, f64)]| -> f64 {
        let n = xs.len() as f64;
        let mx = xs.iter().map(|x| x.0).sum::<f64>() / n;
        let my = xs.iter().map(|x| x.1).sum::<f64>() / n;
        let cov = xs.iter().map(|x| (x.0 - mx) * (x.1 - my)).sum::<f64>();
        let vx = xs.iter().map(|x| (x.0 - mx).powi(2)).sum::<f64>().sqrt();
        let vy = xs.iter().map(|x| (x.1 - my).powi(2)).sum::<f64>().sqrt();
        if vx == 0.0 || vy == 0.0 {
            0.0
        } else {
            cov / (vx * vy)
        }
    };

    let streak_list: Vec<u32> = streaks.values().map(|(_, _, mx)| *mx).collect();
    let mean_streak = streak_list.iter().sum::<u32>() as f64 / streak_list.len().max(1) as f64;
    let multi_year = streak_list.iter().filter(|s| **s >= 2).count();

    let out = serde_json::json!({
        "config": { "seeds": n_seeds, "years": years, "entries_total": total_entries },
        "role_distribution": role_dist,
        "rating": { "mean": mean, "p5": q(0.05), "p25": q(0.25), "p50": q(0.50), "p75": q(0.75), "p95": q(0.95) },
        "correlations": {
            "rank_vs_mvp_evp": pearson(&rank_vs_mvp),
            "rank_vs_rating": pearson(&rank_vs_rating)
        },
        "streaks": { "players": streaks.len(), "multi_year_players": multi_year, "mean_max_streak": mean_streak },
        "igl_cases": igl_cases,
        "igl_case_count": igl_cases.len(),
        "tier_event_counts": tier_counts,
        "anomaly_count": anomalies.len(),
        "anomaly_samples": anomalies.iter().take(30).cloned().collect::<Vec<_>>(),
        "per_seed": per_seed,
        "elapsed_secs": t0.elapsed().as_secs_f64()
    });
    println!("{}", serde_json::to_string(&out).expect("序列化失败"));
}
