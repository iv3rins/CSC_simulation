//! 长程浸泡测试 + 性能基准（P2）——生涯世界的不变量守护。
//!
//! 世界推进后必须仍然成立的不变量：
//! - roster 恒 5 人（人口再生补位契约）；
//! - 选手年龄/疲劳/伤病倒计时/合同年限不越界（clamp 契约）；
//! - 事件流 seq 严格递增（增量同步游标契约）；
//! - 生涯档案逐赛季归档（跨年结算契约）；
//! - VRS matchHistory 窗口剪枝生效（B2：长生涯内存不无界增长）；
//! - 玩家经济不为负（经济闭环收入端契约）。
//!
//! 分级：12 个月快速版随默认测试跑（~秒级）；20 年完整版与性能基准标
//! `#[ignore]`（release 模式门禁：`cargo test --release -- --ignored soak`）。
//! 本套件已捕获过真实 bug：`next_i32_bound` 负 v 被接受 → `NAME_POOL[-11]`
//! 越界（Kotlin 重写版上游缺陷，已在 csc-util 修正，见 rng.rs）。

use csc_core::engine::Engine;
use csc_decision::source::AutoDecisionSource;
use csc_domain::tier::Tier;
use csc_domain::tourney_tier::TourneyTier;
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;

/// 生成 N 队 × 5 人的 standings JSON（积分递减；结构对齐真实资产）。
fn standings_fixture(team_count: usize) -> String {
    let mut rankings = String::new();
    for ti in 0..team_count {
        if ti > 0 {
            rankings.push(',');
        }
        let roster: Vec<String> = (0..5).map(|i| format!("T{ti}P{i}")).collect();
        let points = (2000_i64 - ti as i64 * 50).max(250);
        rankings.push_str(&format!(
            r#"{{"ranking":{},"points":{},"teamName":"Team{}","roster":[{}]}}"#,
            ti + 1,
            points,
            ti,
            roster
                .iter()
                .map(|n| format!(r#""{n}""#))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    format!(r#"{{"rankings":[{rankings}]}}"#)
}

/// 装载满编世界 + 主角（用 `Engine::create_protagonist` 替换 Team0 最弱 NPC，
/// 自带 VRS 阵容迁移——手动 release/assign 会让 VRS 失步，20 年门禁因此刷
/// 「未知队伍签名」警告且拖慢模拟）。
fn fixture_engine(team_count: usize, seed: u64) -> Engine {
    let files = vec![(
        "standings_global_2026_01_05.json".to_string(),
        standings_fixture(team_count),
    )];
    let mut eng = Engine::load_from_standings(
        &files,
        &csc_core::CalibrationAssets::default(),
        seed,
        SimClock::of(2026, 1, 1),
    )
    .expect("fixture 资产合法");
    // 主角：垫底队伍起步（青训新秀叙事）
    let bottom = csc_util::id::TeamId(eng.world().all_teams().len().saturating_sub(1) as u32);
    eng.create_protagonist(
        "MyPlayer",
        Tier::Tier4,
        bottom,
        &mut Xoshiro256StarStar::seed(seed ^ 0x5EED),
        None,
    )
    .expect("主角创建");
    // R2.1：与服务器 create_game 同流——开局物化当月（2026-01，month_idx=-1），
    // 首次月推进必须结算开局月（不再滞留 Future）。
    eng.plan_opening_month(-1);
    eng
}

/// 世界不变量全量检查（任意时刻可调用）。
fn assert_invariants(eng: &Engine) {
    // 1. roster 恒 5 人
    for team in &eng.world().teams {
        assert_eq!(
            team.roster_ids.len(),
            5,
            "队伍「{}」roster 偏离 5 人",
            team.name
        );
    }
    // 2. 选手数值越界检查（arena 是**墓碑制**：退役 NPC 保留在 arena 以维持
    //    「id = Vec 索引」不变量，但必须无归属、不进自由市场、不进任何查询）
    let free_agents = eng.world().all_free_agents();
    for pc in &eng.world().players {
        if pc.retired {
            assert!(pc.team.is_none(), "退役者「{}」不应有队伍归属", pc.name);
            assert!(
                !free_agents.contains(&pc.id),
                "退役者「{}」不应在自由市场",
                pc.name
            );
            continue;
        }
        assert!(
            (16..=50).contains(&pc.age),
            "年龄越界：{} 岁（{}）",
            pc.age,
            pc.name
        );
        assert!(
            (0.0..=100.0).contains(&pc.fatigue),
            "疲劳越界：{}（{}）",
            pc.fatigue,
            pc.name
        );
        if let Some(injury) = &pc.injury {
            assert!(injury.days_left >= 0, "伤病倒计时为负（{}）", pc.name);
        }
        if let Some(career) = &pc.career {
            assert!(career.contract_years >= 0, "合同年限为负（{}）", pc.name);
            assert!(career.finance.cash >= 0, "玩家现金为负（{}）", pc.name);
            assert!(
                (0..=100).contains(&career.reputation),
                "声誉越界（{}）",
                pc.name
            );
        }
    }
    // 3. 事件流 seq 严格递增（增量同步游标契约）
    let journal = eng.journal().all();
    for (i, e) in journal.iter().enumerate() {
        assert_eq!(
            e.seq(),
            i as i32,
            "事件流 seq 不连续：第 {i} 条 seq={}",
            e.seq()
        );
    }
    // 4. VRS matchHistory 窗口剪枝（B2：长生涯不无界增长）
    let history = eng.vrs().database().match_history_len();
    assert!(history < 30_000, "matchHistory 剪枝失效：{history} 条");
    // 5. VRS 实时排名必须回写到队伍展示缓存（2026 修复：排名不再静止）
    for team in &eng.world().teams {
        let sig = eng.world().signature_of(team.id);
        if let Some(st) = eng.vrs().team_of(&sig) {
            assert_eq!(
                team.vrs_ranking, st.ranking,
                "队伍「{}」VRS 排名缓存与数据库不同步",
                team.name
            );
            assert_eq!(
                team.vrs_value, st.points,
                "队伍「{}」VRS 积分缓存与数据库不同步",
                team.name
            );
        }
    }
    // 6. 经济生态健康线（2026 修订补：此前 soak 只查玩家现金，不查队伍预算——
    //    15 赛季长程验证发现 NPC 运营成本 10 万/年造成 38/40 队结构性赤字，
    //    修复为 3 万后中位预算 +150 万。本断言守这条生态线：预算中位数不得
    //    大面积结构性赤字（-500 万线），也不得通胀爆炸（+3000 万线）。
    if !eng.world().teams.is_empty() {
        let mut budgets: Vec<i64> = eng.world().teams.iter().map(|t| t.budget).collect();
        budgets.sort_unstable();
        let median = budgets[budgets.len() / 2];
        assert!(median > -5_000_000, "队伍预算中位数结构性赤字：{median}");
        assert!(median < 30_000_000, "队伍预算中位数通胀异常：{median}");
    }
}

/// **12 个月快速浸泡**（默认套件）：40 队满编世界推进 1 年（T1 锁 16 队时
/// T2 仍有足够队伍——日历并行生态要求 ≥40 队），覆盖跨年结算
/// （归档/成长/人口/薪资/合同）+ 3 次转会窗，秒级完成。
#[test]
fn twelve_month_soak_keeps_invariants() {
    let mut eng = fixture_engine(40, 42);
    let mut auto = AutoDecisionSource;
    eng.run_season(12, &mut auto).expect("auto 推进必成功");
    assert_eq!(eng.month(), 12);
    assert_invariants(&eng);
    // 跨年归档契约：主角有 1 个赛季记录
    let player_id = eng.player().expect("主角存在");
    assert_eq!(
        eng.archive().seasons_of(player_id).len(),
        1,
        "12 个月应有 1 个赛季归档"
    );
}

/// **王朝队不被转会窗拆散**（2026-08 修复回归）：一年 3 个转会窗后，
/// 开局 TOP6 队伍仍保留 VRS 积分——修复前每窗 1 次换人会在第三窗被
/// 跨窗累计成「3 人离队 → 积分清零」，T1 王朝直接掉进积分榜底部。
#[test]
fn dynasty_teams_keep_vrs_points_through_transfer_windows() {
    let mut eng = fixture_engine(40, 42);
    let dynasty: Vec<csc_util::id::TeamId> = {
        let mut teams: Vec<_> = eng.world().teams.iter().collect();
        teams.sort_by_key(|t| t.vrs_ranking);
        teams.into_iter().take(6).map(|t| t.id).collect()
    };
    let mut auto = AutoDecisionSource;
    eng.run_season(12, &mut auto).expect("auto 推进必成功");

    for tid in dynasty {
        let team = eng.world().team(tid).expect("王朝队仍存在");
        let sig = eng.world().signature_of(tid);
        let vrs_team = eng.vrs().team_of(&sig).expect("VRS 状态存在");
        assert!(
            vrs_team.points > 0,
            "王朝队「{}」积分被清零（转会窗拆队）——当前积分 {}",
            team.name,
            vrs_team.points
        );
        assert!(
            vrs_team.ranking <= 40,
            "王朝队「{}」排名跌出世界榜：{}",
            team.name,
            vrs_team.ranking
        );
    }
}

/// **世界赛事不依赖主角参赛**（2026 修复回归）：主角从垫底 T4 队伍起步，
/// 一年内日历上的全部 T1 / Major 仍必须完整模拟——NPC 的逐图 Rating、
/// MVP/EVP 荣誉进入年度累计器，TOP20 才能像真实 HLTV 一样满员竞争。
/// 此前把 TOP20 样本门槛调低是错误方向：问题不是「赛历不够」，
/// 而是「玩家不在 T1 时世界赛事是否照常跑」。
#[test]
fn t1_world_is_simulated_regardless_of_protagonist_tier() {
    let mut eng = fixture_engine(40, 7);
    let mut auto = AutoDecisionSource;
    eng.run_season(12, &mut auto).expect("auto 推进必成功");

    // 1. 顶级赛事照常排期并完赛：R2.1 起开局月（2026-01）在首次月推进时结算，
    //    12 次月推进覆盖 2026-01..2027-01 共 13 个日历月（开局月 + 12 个
    //    常规月）。T1 桶：2026 年度 25 场（2 Major + 18 S + 5 A）+ 2027-01 两场
    //    （BLAST Bounty / IEM Kraków 2027）= 27；T2 13×5 = 65；T3 13×8 = 104。
    let results = eng.tournaments().results_ref();
    let top_count = results.iter().filter(|r| r.event.tier.is_elite()).count();
    let major_count = results
        .iter()
        .filter(|r| r.event.tier == TourneyTier::Major)
        .count();
    let t2_count = results
        .iter()
        .filter(|r| r.event.tier == TourneyTier::T2)
        .count();
    let t3_count = results
        .iter()
        .filter(|r| r.event.tier == TourneyTier::Qualify)
        .count();
    assert_eq!(
        top_count, 27,
        "开局月结算后 12 次月推进覆盖 13 个日历月：T1 桶 25（2026）+ 2（2027-01）"
    );
    assert_eq!(major_count, 2, "全年 2 场 Major 必须照常模拟");
    assert_eq!(t2_count, 65, "13 个日历月 × 5 场 T2");
    assert_eq!(t3_count, 104, "13 个日历月 × 8 场 T3");
    // R2.1 开局月闭环：2026-01 赛事必须已完成（不再滞留 Future）。
    let jan_2026 = results
        .iter()
        .filter(|r| r.event.date.starts_with("2026-01"))
        .count();
    assert_eq!(
        jan_2026, 15,
        "开局月（2026-01）15 场赛事必须在首次月推进时结算（2 T1 + 5 T2 + 8 T3）"
    );

    // 2. 跨年结算产出完整 TOP20 榜单（20 人满员，不因主角未进 T1 而空缺）
    let history = eng.tournaments().top20_history();
    assert_eq!(history.len(), 1, "跨年后应有 2026 年度榜单快照");
    let board = &history[0];
    assert_eq!(board.year, 2026);
    assert_eq!(board.entries.len(), 20, "T1 世界赛事完整模拟 → TOP20 满员");
    let normal = board.entries.iter().filter(|e| !e.wildcard).count();
    assert!(
        normal >= 15,
        "榜单主体应是打满 T1 的 NPC 主力，而非通配外卡：{normal}/20"
    );
    assert!(
        board
            .entries
            .iter()
            .all(|e| e.maps >= csc_tournaments::top20::Top20Evaluator::MIN_MAPS),
        "入围者均满足 T1+ 最低样本门槛"
    );
}

/// **128 队扩展世界浸泡**（默认套件）：验证扩充后的世界规模下，日历/锁/
/// 转会/人口/VRS 不变量仍然成立，且低级别赛事能覆盖绝大多数队伍（世界不是
/// 只有 TOP40 在玩）。40 队版本仍由 `twelve_month_soak_keeps_invariants` 守护。
#[test]
fn expanded_128_team_world_keeps_invariants() {
    let mut eng = fixture_engine(128, 42);
    let mut auto = AutoDecisionSource;
    eng.run_season(12, &mut auto)
        .expect("128 队 auto 推进必成功");

    assert_eq!(eng.month(), 12);
    assert_eq!(eng.world().all_teams().len(), 128, "世界应为 128 队");
    assert_invariants(&eng);

    // 参与广度：一年内至少 100 支不同队伍进过赛事（TOP40 之外的队伍不再陪跑）。
    let mut participants = std::collections::HashSet::new();
    for ev in eng.tournaments().results_ref() {
        for series in &ev.series {
            participants.insert(series.team_a_id);
            participants.insert(series.team_b_id);
        }
    }
    assert!(
        participants.len() >= 100,
        "128 队世界中至少 100 队应实际参赛，实际 {}",
        participants.len()
    );
}

/// **128 队 20 年浸泡测试**（release 门禁，Sprint 1）：产品世界的长生涯质量基线。
/// 除了不变量，额外验证：
/// - 主角 20 个赛季档案完整；
/// - 至少 100 支队伍参与过比赛；
/// - 20 年快照 JSON 体积有上限（当前实测约 177MB，留 256MB 预算）。
#[test]
#[ignore = "长程门禁：cargo test --release -- --ignored twenty_year_128"]
fn twenty_year_128_team_soak() {
    let mut eng = fixture_engine(128, 42);
    let mut auto = AutoDecisionSource;
    let start = std::time::Instant::now();
    eng.run_season(240, &mut auto)
        .expect("128 队 20 年推进必成功");

    assert_eq!(eng.month(), 240);
    assert_invariants(&eng);

    let player_id = eng
        .world()
        .players
        .iter()
        .find(|p| p.name == "MyPlayer")
        .map(|p| p.id)
        .expect("主角实体应保留（退役不收缩 arena）");
    let seasons = eng.archive().seasons_of(player_id);
    assert_eq!(seasons.len(), 20, "20 年应有 20 个赛季档案");

    let mut participants = std::collections::HashSet::new();
    for ev in eng.tournaments().results_ref() {
        for series in &ev.series {
            participants.insert(series.team_a_id);
            participants.insert(series.team_b_id);
        }
    }
    assert!(
        participants.len() >= 100,
        "128 队世界参与广度不足：{}",
        participants.len()
    );

    let snapshot_bytes = serde_json::to_vec(&eng.snapshot())
        .map(|v| v.len())
        .unwrap_or(0);
    assert!(
        snapshot_bytes <= 256 * 1024 * 1024,
        "20 年快照超预算：{snapshot_bytes} bytes"
    );

    println!(
        "soak128: 240 个月 / {} 事件 / {} 决策 / {:.2?} / 快照 {:.1}MB",
        eng.journal().len(),
        eng.decision_log().count(),
        start.elapsed(),
        snapshot_bytes as f64 / 1e6
    );
}

/// **20 年浸泡测试**（`#[ignore]` 门禁）：240 个月全速推进（debug ~6s），跑完全程不 panic
/// 且不变量全绿。覆盖：跨年结算 ×20、人口再生（≥35 岁退役 + 新秀补位）、
/// 转会窗 ×60、NPC 转会市场、伤病循环、月度决策批次、VRS reseed 窗口剪枝。
#[test]
#[ignore = "长程门禁：cargo test --release -- --ignored twenty_year"]
fn twenty_year_soak_keeps_invariants() {
    let mut eng = fixture_engine(40, 42);
    let mut auto = AutoDecisionSource;
    let start = std::time::Instant::now();

    eng.run_season(240, &mut auto).expect("auto 推进必成功");

    let elapsed = start.elapsed();
    println!(
        "soak: 240 个月 / {} 事件 / {} 决策 / {:.2?}（{:.1} ms/月）",
        eng.journal().len(),
        eng.decision_log().count(),
        elapsed,
        elapsed.as_secs_f64() * 1000.0 / 240.0
    );

    assert_eq!(eng.month(), 240);
    assert_invariants(&eng);
    // 生涯档案：主角应有 ~20 个赛季记录（跨年归档契约）。
    // 注意：主角 36 岁退役（population 修订）后 `Engine::player()` 不再返回——
    // 从 arena 按名字查找（退役者保留实体，档案仍可查）。
    let player_id = eng
        .world()
        .players
        .iter()
        .find(|p| p.name == "MyPlayer")
        .map(|p| p.id)
        .expect("主角存在（含退役保留）");
    let seasons = eng.archive().seasons_of(player_id);
    assert!(
        (18..=21).contains(&seasons.len()),
        "归档赛季数异常：{}",
        seasons.len()
    );
    // 人口更替确实发生（老将退役过、新秀补位过）
    let retirements = eng
        .journal()
        .all()
        .iter()
        .filter(|e| e.kind() == csc_events::event::WorldEventKind::Retirement)
        .count();
    assert!(retirements > 0, "20 年应有退役事件");
}

/// 性能基准（`#[ignore]` 门禁用：`cargo test --release -- --ignored bench`）。
#[test]
#[ignore = "性能基准：release 模式运行"]
fn bench_advance_month_throughput() {
    let mut eng = fixture_engine(40, 42);
    let mut auto = AutoDecisionSource;
    // 预热 12 个月
    eng.run_season(12, &mut auto).expect("auto 推进必成功");
    let start = std::time::Instant::now();
    eng.run_season(120, &mut auto).expect("auto 推进必成功");
    let per_month = start.elapsed().as_secs_f64() * 1000.0 / 120.0;
    println!("bench: {per_month:.2} ms/月（120 个月样本）");
    assert!(per_month < 1000.0, "月度推进吞吐异常：{per_month:.1} ms/月");
}
