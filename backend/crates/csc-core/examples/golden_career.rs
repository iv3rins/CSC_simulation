//! 黄金生涯集（Sprint 1）：单个 seed 的 20 年产品指标审计。
//!
//! 输出内容（JSON stdout）：
//! - 每赛季主角档案：队伍/排名/图数/rating/胜场/收入/伤病/荣誉；
//! - 生涯汇总：转会次数、自由身月数、伤病天数、冠军/TOP20 数、峰值 rating；
//! - 决策分布：类型/选项计数（按 point_id 前缀归类）；
//! - 事件重复度：journal 类型计数；
//! - 世界集中度：冠军队伍分布、TOP3 冠军份额、128 队参与广度；
//! - 性能：装载/逐月推进/快照大小。
//!
//! 用法（release）：
//!   cargo run -p csc-core --release --example golden_career -- ../assets 42 20 > golden_seed42.json

use std::collections::HashMap;
use std::time::Instant;

use csc_core::{CalibrationAssets, Engine};
use csc_domain::tier::Tier;
use csc_time::clock::SimClock;
use csc_util::id::TeamId;
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

fn decision_kind(point_id: &str) -> &'static str {
    if point_id.contains("|life|") {
        "LIFE_EVENT"
    } else if point_id.contains("|inter|") {
        "MATCH_INTERVENTION"
    } else if point_id.contains("|blunder|") {
        "TEAMMATE_BLUNDER"
    } else if point_id.contains("|transfer|") {
        "TRANSFER_WINDOW"
    } else if point_id.contains("|injury|") {
        "INJURY_DECISION"
    } else if point_id.contains("|sponsor|") {
        "SPONSORSHIP_OFFER"
    } else if point_id.contains("|training|") {
        "TRAINING_FOCUS"
    } else {
        "UNKNOWN"
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let assets_dir = args.get(1).map(String::as_str).unwrap_or("../assets");
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(42);
    let years: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);

    let (standings, baseline, ratings, profile) = load_assets(assets_dir);
    let t_load = Instant::now();
    let calibration = CalibrationAssets {
        text: None,
        baseline: baseline.as_deref(),
        ratings: ratings.as_deref(),
        rating_profile: profile.as_deref(),
    };
    let mut engine =
        Engine::load_from_standings(&standings, &calibration, seed, SimClock::of(2026, 1, 1))
            .expect("装载失败");
    let bottom = TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
    let pid = engine
        .create_protagonist(
            "Golden",
            Tier::Tier4,
            bottom,
            &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
            None,
        )
        .expect("主角创建失败");
    let load_ms = t_load.elapsed().as_secs_f64() * 1000.0;

    let mut auto = csc_core::engine::auto_decision();
    let mut monthly_ms: Vec<f64> = Vec::with_capacity((years * 12) as usize);
    let mut seasons: Vec<serde_json::Value> = Vec::new();
    let t_run = Instant::now();
    for _year_no in 1..=years {
        for _ in 0..12 {
            let t = Instant::now();
            engine.advance_month(&mut auto).expect("推进失败");
            monthly_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        if let Some(record) = engine.archive().seasons_of(pid).last().cloned() {
            let team_rank = record
                .team_name
                .as_ref()
                .and_then(|name| engine.world().team_by_name(name))
                .map(|tid| engine.world().team(tid).map(|t| t.vrs_ranking).unwrap_or(0))
                .unwrap_or(0);
            seasons.push(serde_json::json!({
                "year": record.year,
                "team": record.team_name,
                "team_rank": team_rank,
                "events": record.events_played,
                "maps": record.maps_played,
                "rating": record.rating,
                "wins": record.wins,
                "earnings": record.earnings,
                "injury_days": record.injury_days,
                "honours": record.honours.len(),
            }));
        }
    }
    let total_ms = t_run.elapsed().as_secs_f64() * 1000.0;
    monthly_ms.sort_by(|a, b| a.total_cmp(b));
    let p = |p: f64| -> f64 { monthly_ms[((monthly_ms.len() - 1) as f64 * p) as usize] };

    // 生涯汇总
    let career_totals = engine.archive().totals_of(pid);
    let pc = engine.world().player(pid);
    let final_career = pc.and_then(|p| p.career.as_ref());
    let journal = engine.journal().all();
    let transfer_count = journal
        .iter()
        .filter(|e| {
            e.kind() == csc_events::event::WorldEventKind::TransferDone
                && e.player_id() == Some(pid)
        })
        .count();
    let injury_events = journal
        .iter()
        .filter(|e| {
            e.kind() == csc_events::event::WorldEventKind::InjuryOccurred
                && e.player_id() == Some(pid)
        })
        .count();
    let retired = pc.is_none_or(|p| p.retired);

    // 决策分布
    let mut decisions: HashMap<&'static str, usize> = HashMap::new();
    let mut choices: HashMap<String, usize> = HashMap::new();
    let mut life_kinds: HashMap<String, usize> = HashMap::new();
    let mut life_repeats = 0usize;
    let mut last_life_kind: Option<String> = None;
    for d in engine.decision_log().entries() {
        let kind = decision_kind(&d.point_id);
        *decisions.entry(kind).or_default() += 1;
        *choices.entry(d.option_id.clone()).or_default() += 1;
        if kind == "LIFE_EVENT" {
            let event_kind = d
                .point_id
                .split('|')
                .nth(2)
                .unwrap_or("UNKNOWN")
                .to_string();
            *life_kinds.entry(event_kind.clone()).or_default() += 1;
            if last_life_kind.as_deref() == Some(event_kind.as_str()) {
                life_repeats += 1;
            }
            last_life_kind = Some(event_kind);
        } else {
            last_life_kind = None;
        }
    }

    // 事件类型重复度
    let mut journal_kinds: HashMap<String, usize> = HashMap::new();
    for e in &journal {
        *journal_kinds.entry(format!("{:?}", e.kind())).or_default() += 1;
    }

    // 冠军集中度 + 参与广度
    let mut champion_counts: HashMap<TeamId, usize> = HashMap::new();
    let mut participants = std::collections::HashSet::new();
    for ev in engine.tournaments().results_ref() {
        *champion_counts.entry(ev.champion).or_default() += 1;
        for series in &ev.series {
            participants.insert(series.team_a_id);
            participants.insert(series.team_b_id);
        }
    }
    let mut champions: Vec<usize> = champion_counts.values().copied().collect();
    champions.sort_by(|a, b| b.cmp(a));
    let total_championships: usize = champions.iter().sum();
    let top3_share =
        champions.iter().take(3).sum::<usize>() as f64 / total_championships.max(1) as f64;

    let snapshot = engine.snapshot();
    let snapshot_bytes = serde_json::to_vec(&snapshot).map(|v| v.len()).unwrap_or(0);

    let out = serde_json::json!({
        "config": { "seed": seed, "years": years, "teams": engine.world().all_teams().len() },
        "performance": {
            "load_ms": load_ms,
            "total_ms": total_ms,
            "month_p50_ms": p(0.50),
            "month_p95_ms": p(0.95),
            "snapshot_json_bytes": snapshot_bytes,
        },
        "seasons": seasons,
        "career": {
            "player_id": pid.0,
            "seasons": career_totals.as_ref().map(|t| t.seasons).unwrap_or(0),
            "peak_rating": career_totals.as_ref().map(|t| t.peak_rating).unwrap_or(0.0),
            "total_earnings": career_totals.as_ref().map(|t| t.total_earnings).unwrap_or(0),
            "transfers": transfer_count,
            "injury_events": injury_events,
            "honours": career_totals.as_ref().map(|t| t.honours.len()).unwrap_or(0),
            "months_unsigned_final": final_career.map(|c| c.months_unsigned).unwrap_or(0),
            "reputation_final": final_career.map(|c| c.reputation).unwrap_or(0),
            "retired": retired,
            "soft_lock_seasons_no_maps": seasons.iter().filter(|s| s["maps"] == 0).count(),
        },
        "decisions": {
            "total": engine.decision_log().count(),
            "by_kind": decisions,
            "choices": choices,
            "life_kinds": life_kinds,
            "life_consecutive_repeats": life_repeats,
        },
        "world": {
            "event_kinds": journal_kinds,
            "champion_counts": champion_counts,
            "top3_champion_share": top3_share,
            "distinct_participants": participants.len(),
            "team_count": engine.world().all_teams().len(),
            "player_count": engine.world().all_players().len(),
        },
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
