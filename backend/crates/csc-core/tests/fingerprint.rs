//! G3 确定性指纹门禁（P0）——世界级模拟的可复现性锁。
//!
//! 世界指纹 = FNV-1a64( sim_version.to_le_bytes() ‖ canonical_json_bytes(state) )。
//! 同 seed + 同决策 → 月级指纹序列逐月全等；存档/恢复续跑与不中断续跑全等。
//! 本套件**禁止 `#[ignore]`**（随 `cargo test --workspace` 默认执行，CI 门禁）。

use csc_core::engine::Engine;
use csc_core::state::GameState;
use csc_decision::source::AutoDecisionSource;
use csc_domain::tier::Tier;
use csc_events::event::WorldEvent;
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
    let bottom = csc_util::id::TeamId(eng.world().all_teams().len().saturating_sub(1) as u32);
    eng.create_protagonist(
        "MyPlayer",
        Tier::Tier4,
        bottom,
        &mut Xoshiro256StarStar::seed(seed ^ 0x5EED),
        None,
    )
    .expect("主角创建");
    eng.plan_opening_month(-1); // 与 server create 同流（R2.1）
    eng
}

/// 世界指纹 = FNV-1a64( sim_version.to_le_bytes() ‖ canonical_json_bytes(state) )。
/// 版本进指纹头：bump WORLD_SIM_VERSION 后的指纹漂移是显式预期行为。
fn world_fingerprint(state: &GameState) -> u64 {
    let bytes = csc_util::canonical_json_bytes(state).expect("canonical 序列化必然成功");
    let mut buf = Vec::with_capacity(4 + bytes.len());
    buf.extend_from_slice(&state.sim_version.to_le_bytes());
    buf.extend_from_slice(&bytes);
    csc_util::fnv1a64(&buf)
}

/// 采集 12 个月的 (month, 指纹) 序列。
fn run_12_months(seed: u64) -> Vec<(i32, u64)> {
    let mut eng = fixture_engine(40, seed);
    let mut seq = Vec::with_capacity(12);
    seq.push((0, world_fingerprint(&eng.snapshot()))); // 第 0 月末（开局基线）
    for _ in 0..12 {
        eng.advance_month(&mut AutoDecisionSource).expect("推进");
        seq.push((eng.month(), world_fingerprint(&eng.snapshot())));
    }
    seq
}

/// G3 t1：同 seed 双引擎独立构建，各跑 12 个月——月级指纹序列全等 + 第 12 月末存档级指纹全等。
#[test]
fn same_seed_12_month_double_run_identical() {
    let a = run_12_months(42);
    let b = run_12_months(42);
    assert_eq!(a, b, "同 seed 同决策 = 月级指纹序列必须逐月全等");
    assert_eq!(
        a.last().map(|p| p.1),
        b.last().map(|p| p.1),
        "第 12 月末存档级指纹全等"
    );
    // 不同 seed 必须漂移（sanity：指纹有区分度）
    assert_ne!(run_12_months(43).last().map(|p| p.1), a.last().map(|p| p.1));
}

/// R1 回归：无 contacts 主角的确定性 + VRS 排名兜底真实覆盖。
///
/// P2-4 返工背景：TeamId 化初版把 VRS 排名兜底降级为「不生成 TeamContact 事件」，
/// 导致 `transfer_contacts` 为空的主角 `kinds` 长度不同 → `rng.next_i32_bound`
/// 消费分叉 → 世界轨迹漂移（pro-review-1 动态 probe 实锤）。原指纹测试碰巧绿
/// 是因为 fixture 主角每月都有 contacts（`refresh_contacts` 对合同 ≤2 年生成），
/// 兜底分支从未被真实执行。
///
/// 本测试把主角合同设为 3 年（`refresh_contacts` 只给 ≤2 年生成 → 主角全程
/// 无 contacts），验证：
/// 1. 双跑 12 个月月级指纹序列全等——兜底路径下 RNG 消费序仍稳定；
/// 2. journal 中确实出现过 TeamContact/SecretTeamContact 决策镜像——兜底分支
///    被真实覆盖（而非静默跳过）。
fn no_contacts_fixture_engine(team_count: usize, seed: u64) -> Engine {
    let mut eng = fixture_engine(team_count, seed);
    let pid = eng
        .world()
        .players
        .iter()
        .find(|p| p.is_player())
        .map(|p| p.id)
        .expect("主角存在");
    eng.world_mut()
        .player_mut(pid)
        .expect("主角存在")
        .career_mut()
        .expect("主角有生涯")
        .contract_years = 3;
    eng
}

fn run_12_months_no_contacts(seed: u64) -> (Vec<(i32, u64)>, bool) {
    let mut eng = no_contacts_fixture_engine(40, seed);
    let mut seq = Vec::with_capacity(12);
    seq.push((0, world_fingerprint(&eng.snapshot())));
    for _ in 0..12 {
        eng.advance_month(&mut AutoDecisionSource).expect("推进");
        seq.push((eng.month(), world_fingerprint(&eng.snapshot())));
    }
    let saw_contact = eng
        .journal()
        .all()
        .iter()
        .filter(|e| matches!(e, WorldEvent::DecisionMade { .. }))
        .any(|e| {
            let WorldEvent::DecisionMade { point_id, .. } = e else {
                return false;
            };
            point_id.contains("|TEAM_CONTACT|") || point_id.contains("|SECRET_TEAM_CONTACT|")
        });
    (seq, saw_contact)
}

/// R1 回归门禁（P2-4 兜底恢复）：无 contacts 主角双跑指纹全等 + 兜底分支真实触发。
#[test]
fn no_contacts_protagonist_double_run_identical_and_covers_fallback() {
    let (a, saw_a) = run_12_months_no_contacts(42);
    let (b, saw_b) = run_12_months_no_contacts(42);
    assert_eq!(
        a, b,
        "无 contacts 主角：月级指纹序列必须逐月全等（VRS 兜底路径 RNG 消费序稳定）"
    );
    assert!(
        saw_a,
        "无 contacts 主角必须真实触发 VRS 排名兜底（产生 TeamContact 决策）"
    );
    assert!(saw_b, "双跑都应覆盖兜底分支");
}

/// G3 t2：A 跑 6 月 → snapshot 存入 B → 双方再跑 6 月 → 第 12 月末指纹与月级序列全等。
#[test]
fn mid_save_restore_continue_equals_uninterrupted() {
    let mut a = fixture_engine(40, 42);
    for _ in 0..6 {
        a.advance_month(&mut AutoDecisionSource).expect("推进");
    }
    let snapshot: GameState = a.snapshot();
    let mut b = Engine::empty(
        0,
        SimClock::of(snapshot.sim_year, snapshot.sim_month, snapshot.sim_day),
    );
    b.restore(snapshot);
    let mut a_seq = vec![(a.month(), world_fingerprint(&a.snapshot()))];
    let mut b_seq = vec![(b.month(), world_fingerprint(&b.snapshot()))];
    for _ in 0..6 {
        a.advance_month(&mut AutoDecisionSource).expect("推进");
        b.advance_month(&mut AutoDecisionSource).expect("推进");
        a_seq.push((a.month(), world_fingerprint(&a.snapshot())));
        b_seq.push((b.month(), world_fingerprint(&b.snapshot())));
    }
    assert_eq!(a_seq, b_seq, "存档/恢复续跑与不中断续跑必须全等");
    let uninter = run_12_months(42);
    assert_eq!(
        uninter.last().map(|p| p.1),
        a_seq.last().map(|p| p.1),
        "续跑第 12 月末指纹 = 不中断双跑第 12 月末指纹"
    );
}

/// G3 t3（S3 验证）：同一状态改 sim_version → 指纹必变（版本升级漂移是显式语义）。
#[test]
fn fingerprint_is_version_sensitive() {
    let mut eng = fixture_engine(40, 42);
    eng.advance_month(&mut AutoDecisionSource).expect("推进");
    let mut state = eng.snapshot();
    let base = world_fingerprint(&state);
    state.sim_version = 2; // 模拟语义升级
    assert_ne!(world_fingerprint(&state), base, "版本标签必须进入指纹");
}
