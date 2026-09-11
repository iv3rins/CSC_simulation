use super::*;
use crate::test_fixtures::{boost_power, vrs_with_teams};

use csc_decision::point::PlayerDecision;
use csc_domain::tier::Tier;
use csc_entities::power::PowerCalculator;
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;

#[test]
fn free_agent_gets_offers_and_transfers() {
    let mut world = World::new();
    // 目标队：T1 级（1 名玩家 + 4 NPC）
    let to = world.create_team("Astralis", 1, 2000);
    world
        .create_npc(
            Tier::Tier0,
            Some(to),
            None,
            Some("dev1ce"),
            &mut Xoshiro256StarStar::seed(1),
            None,
        )
        .unwrap();
    world
        .create_npc(
            Tier::Tier1,
            Some(to),
            None,
            Some("npcA"),
            &mut Xoshiro256StarStar::seed(1),
            None,
        )
        .unwrap();
    world
        .create_npc(
            Tier::Tier1,
            Some(to),
            None,
            Some("npcB"),
            &mut Xoshiro256StarStar::seed(1),
            None,
        )
        .unwrap();
    world
        .create_npc(
            Tier::Tier1,
            Some(to),
            None,
            Some("npcC"),
            &mut Xoshiro256StarStar::seed(1),
            None,
        )
        .unwrap();
    world
        .create_npc(
            Tier::Tier2,
            Some(to),
            None,
            Some("npcWeak"),
            &mut Xoshiro256StarStar::seed(1),
            None,
        )
        .unwrap();
    world.team_mut(to).unwrap().budget = 5_000_000;
    // 自由身玩家（拉高实力到 T1 门槛以上：power 85+）
    let p = world.create_player(
        Tier::Tier1,
        Some("MyPlayer"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    boost_power(&mut world, p);

    let mut vrs = vrs_with_teams(&[(
        "Astralis",
        1,
        2000,
        vec!["dev1ce", "npcA", "npcB", "npcC", "npcWeak"],
    )]);
    let clock = SimClock::of(2026, 6, 1);

    let offers = TransferEngine::generate_offers(&world, &vrs, &clock.date_label());
    assert_eq!(offers.len(), 1, "自由身玩家应有转会候选");
    assert_eq!(offers[0].candidates.len(), 1);
    assert_eq!(offers[0].candidates[0].team_id, to);
    // 最弱 NPC 是 npcWeak
    let weakest_power = PowerCalculator::player_power(
        world
            .player(world.player_by_name("npcWeak").unwrap())
            .unwrap(),
    );
    assert!(offers[0].candidates[0].power_delta > 0.0);
    let _ = weakest_power;
    // 报价薪资应为确定性的玩家市场价（>0），随 offer 一并下发供市场页展示。
    assert!(
        offers[0].candidates[0].salary > 0,
        "候选应携带确定性的报价薪资"
    );

    // 决策：转会到 Astralis
    let decision = PlayerDecision::new(
        offers[0].point_id.clone(),
        offers[0].candidates[0].team_signature.clone(),
    );
    let events = TransferEngine::execute_choices(
        &mut world,
        &mut vrs,
        &clock,
        &offers,
        &[decision],
        &mut Xoshiro256StarStar::seed(7),
    )
    .expect("合法决策必成功");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from_team, None, "自由身首签 from_team = null");
    assert_eq!(events[0].to_team, "Astralis");
    // 玩家已入队
    assert_eq!(world.player(p).unwrap().team, Some(to));
    // 被换 NPC 进自由市场
    assert_eq!(
        world.all_free_agents(),
        vec![world.player_by_name("npcWeak").unwrap()]
    );
    // roster 保持 5 人
    assert_eq!(world.team(to).unwrap().roster_ids.len(), 5);
    // 预算扣减（薪资 + 转会费）
    assert!(world.team(to).unwrap().budget < 5_000_000);
    // 合同已签
    let career = world.player(p).unwrap().career.as_ref().unwrap();
    assert!(career.contract_years > 0);
    assert!(career.salary > 0);
}

#[test]
fn stay_decision_skips_transfer() {
    let mut world = World::new();
    let to = world.create_team("Astralis", 1, 2000);
    for i in 0..5 {
        world
            .create_npc(
                Tier::Tier1,
                Some(to),
                None,
                Some(&format!("n{i}")),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
    }
    world.team_mut(to).unwrap().budget = 5_000_000;
    let p = world.create_player(
        Tier::Tier1,
        Some("P"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    boost_power(&mut world, p);
    let mut vrs = vrs_with_teams(&[("Astralis", 1, 2000, vec!["n0", "n1", "n2", "n3", "n4"])]);
    let clock = SimClock::of(2026, 6, 1);
    let offers = TransferEngine::generate_offers(&world, &vrs, &clock.date_label());
    let decision = PlayerDecision::new(offers[0].point_id.clone(), TransferEngine::STAY);
    let events = TransferEngine::execute_choices(
        &mut world,
        &mut vrs,
        &clock,
        &offers,
        &[decision],
        &mut Xoshiro256StarStar::seed(7),
    )
    .expect("STAY 决策必成功");
    assert!(events.is_empty());
    assert_eq!(world.player(p).unwrap().team, None);
}

#[test]
fn contract_summary_format() {
    let mut world = World::new();
    let t = world.create_team("Vitality", 2, 1800);
    let p = world.create_player(
        Tier::Tier1,
        Some("P"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    world.assign_player_to_team(p, t).unwrap();
    {
        let c = world.player_mut(p).unwrap().career.as_mut().unwrap();
        c.salary = 500_000;
        c.contract_years = 2;
    }
    let s = TransferEngine::contract_summary(&world, p).expect("合法玩家必成功");
    assert!(s.contains("Vitality"));
    assert!(s.contains("年薪 500000"));
    assert!(s.contains("剩余合同 2 年"));
    assert!(TransferEngine::contract_summary(&world, PlayerId(999)).is_err());
}

/// 合同到期留队（STAY）→ 按市场价续约（薪资解冻，2026 平衡性修订）。
#[test]
fn stay_re_signs_expired_contract_at_market_rate() {
    let mut world = World::new();
    // 玩家当前队（T4 弱队）
    let own = world.create_team("OwnTeam", 100, 500);
    for i in 0..4 {
        world
            .create_npc(
                Tier::Tier4,
                Some(own),
                None,
                Some(&format!("o{i}")),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
    }
    world.team_mut(own).unwrap().budget = 3_000_000;
    let p = world.create_player(
        Tier::Tier1,
        Some("P"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    boost_power(&mut world, p);
    world.assign_player_to_team(p, own).unwrap();
    {
        let c = world.player_mut(p).unwrap().career.as_mut().unwrap();
        c.contract_years = 0; // 合同已到期
        c.salary = 10_000; // 旧低价合同（冻结态）
        c.reputation = 80;
    }
    // 另一支可负担的强队（供候选生成；新规则下任何付得起的队都行）
    let other = world.create_team("OtherTeam", 5, 1800);
    for i in 0..5 {
        world
            .create_npc(
                Tier::Tier4,
                Some(other),
                None,
                Some(&format!("t{i}")),
                &mut Xoshiro256StarStar::seed(3),
                None,
            )
            .unwrap();
    }
    world.team_mut(other).unwrap().budget = 10_000_000;

    let mut vrs = vrs_with_teams(&[
        ("OwnTeam", 100, 500, vec!["P", "o0", "o1", "o2", "o3"]),
        ("OtherTeam", 5, 1800, vec!["t0", "t1", "t2", "t3", "t4"]),
    ]);
    let clock = SimClock::of(2026, 6, 1);

    let offers = TransferEngine::generate_offers(&world, &vrs, &clock.date_label());
    assert!(!offers.is_empty(), "到期球员应有候选（不再限定排名更高）");
    let decision = PlayerDecision::new(offers[0].point_id.clone(), TransferEngine::STAY);
    let events = TransferEngine::execute_choices(
        &mut world,
        &mut vrs,
        &clock,
        &offers,
        &[decision],
        &mut Xoshiro256StarStar::seed(7),
    )
    .expect("STAY 必成功");
    assert!(events.is_empty(), "留队不产生转会事件");
    let career = world.player(p).unwrap().career.as_ref().unwrap();
    assert!(
        career.contract_years > 0,
        "续约后合同年限重置: {}",
        career.contract_years
    );
    assert!(career.salary > 10_000, "薪资解冻并上涨: {}", career.salary);
    assert_eq!(world.player(p).unwrap().team, Some(own), "续约后仍在原队");
}

// 2026 复审新增：买断自己换队 —— 违约金扣款 + 离队自由市场 + 原队补位 + VRS 迁移。
#[test]
fn buyout_releases_player_to_free_agency() {
    let mut world = World::new();
    let own = world.create_team("OwnTeam", 100, 2000);
    // 4 名 NPC 队友（5 人阵容不变式）
    for i in 0..4 {
        world
            .create_npc(
                Tier::Tier1,
                Some(own),
                None,
                Some(&format!("o{i}")),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
    }
    let p = world.create_player(
        Tier::Tier1,
        Some("BuyoutPro"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    world.assign_player_to_team(p, own).expect("入队成功");
    {
        let pc = world.player_mut(p).unwrap();
        let career = pc.career_mut().unwrap();
        career.salary = 80_000;
        career.contract_years = 2;
        career.finance.cash = 500_000; // 足够支付 160k 买断费
    }
    let mut vrs = vrs_with_teams(&[(
        "OwnTeam",
        100,
        500,
        vec!["BuyoutPro", "o0", "o1", "o2", "o3"],
    )]);

    // 现金不足 → 拒绝且状态不变
    {
        let pc = world.player_mut(p).unwrap();
        pc.career_mut().unwrap().finance.cash = 10_000;
    }
    let err = TransferEngine::buyout(&mut world, &mut vrs, p, &mut Xoshiro256StarStar::seed(3))
        .expect_err("现金不足必须拒绝");
    assert!(err.contains("现金不足"));
    assert_eq!(world.player(p).unwrap().team, Some(own), "拒绝时不得离队");

    // 现金充足 → 买断成功：自由身 + 合同清零 + 扣款 + 原队补位
    {
        let pc = world.player_mut(p).unwrap();
        pc.career_mut().unwrap().finance.cash = 500_000;
    }
    let cost = TransferEngine::buyout(&mut world, &mut vrs, p, &mut Xoshiro256StarStar::seed(3))
        .expect("买断成功");
    assert_eq!(cost, 160_000, "买断费 = 年薪 × 2");
    let pc = world.player(p).unwrap();
    assert_eq!(pc.team, None, "买断后进入自由市场");
    let career = pc.career.as_ref().unwrap();
    assert_eq!(career.contract_years, 0, "合同清零");
    assert_eq!(career.finance.cash, 500_000 - 160_000, "扣款正确");
    assert_eq!(world.roster(own).len(), 5, "原队保持 5 人不变式");
    // 自由身后再买断 → 拒绝（无合同）
    let err2 = TransferEngine::buyout(&mut world, &mut vrs, p, &mut Xoshiro256StarStar::seed(4))
        .expect_err("自由身无需买断");
    assert!(err2.contains("合同"));
}

/// P0 回归（t13）：报价生成时 VRS 快照与 world roster 一致（候选生成），但**生成后、执行前**
/// world roster 变动（模拟同窗内 NPC 转会/阵容漂移），offer 签名变陈旧——执行仍按 **team_id**
/// 正确转会（不再因签名不匹配被误判「同窗被占用」而静默失败）。
#[test]
fn transfer_executes_when_offer_signature_stale_vs_world() {
    let mut world = World::new();
    // 目标队：T1 级（4 NPC + 1 弱 NPC），world 当前 roster 稳定 5 人。
    let to = world.create_team("Drifters", 1, 2000);
    for name in ["dev1ce", "npcA", "npcB", "npcC", "npcWeak"] {
        world
            .create_npc(
                Tier::Tier1,
                Some(to),
                None,
                Some(name),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
    }
    world.team_mut(to).unwrap().budget = 5_000_000;
    // 自由身玩家（拉高实力到 T1 门槛）。
    let p = world.create_player(
        Tier::Tier1,
        Some("MyPlayer"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    boost_power(&mut world, p);

    // VRS 与 world **一致**（候选正常生成）。
    let mut vrs = vrs_with_teams(&[(
        "Drifters",
        1,
        2000,
        vec!["dev1ce", "npcA", "npcB", "npcC", "npcWeak"],
    )]);
    let clock = SimClock::of(2026, 6, 1);

    let offers = TransferEngine::generate_offers(&world, &vrs, &clock.date_label());
    assert_eq!(offers.len(), 1, "自由身玩家应有转会候选");
    let candidate = offers[0].candidates[0].clone();
    assert_eq!(candidate.team_id, to);

    // 报价签名在生成时与 world 一致。
    assert_eq!(candidate.team_signature, world.signature_of(to));

    // 模拟「生成后、执行前」world 阵容漂移：把 npcB 换走（加入一个外部 NPC），
    // 使 world 当前签名与 offer 签名不再一致（复现同窗内 NPC 转会漂移）。
    let npc_b = world.player_by_name("npcB").unwrap();
    let ext_team = world.create_team("Elsewhere", 50, 500);
    world
        .release_player(npc_b)
        .expect("内部不变量：npcB 必在 arena");
    world
        .assign_player_to_team(npc_b, ext_team)
        .expect("内部不变量：目标队存在");
    // 用新 NPC 补回 Drifters 保持 5 人不变式（避免 execute 断言 roster 空/替换到刚走者）。
    let filler = world
        .create_npc(
            Tier::Tier2,
            Some(to),
            None,
            Some("npcDrift"),
            &mut Xoshiro256StarStar::seed(9),
            None,
        )
        .unwrap();
    let _ = filler;
    // 现在 world 签名已漂移（npcB 走、npcDrift 入），offer 签名成陈旧快照。
    assert_ne!(
        candidate.team_signature,
        world.signature_of(to),
        "前提：执行时 world 签名已与 offer 签名漂移（否则不构成回归）"
    );

    // 玩家接受报价（用 offer 下发的签名作 option_id，前端即如此）。
    let decision =
        PlayerDecision::new(offers[0].point_id.clone(), candidate.team_signature.clone());
    let events = TransferEngine::execute_choices(
        &mut world,
        &mut vrs,
        &clock,
        &offers,
        &[decision],
        &mut Xoshiro256StarStar::seed(7),
    )
    .expect("合法决策必成功");
    // 修复后：即使执行时签名已漂移，转会仍按 team_id 执行成功（不静默失败）。
    assert_eq!(
        events.len(),
        1,
        "漂移场景下转会必须执行（不再被误判占用跳过）"
    );
    assert_eq!(events[0].to_team, "Drifters");
    assert_eq!(
        world.player(p).unwrap().team,
        Some(to),
        "玩家应已实际转会到目标队"
    );
    assert_eq!(
        world.team(to).unwrap().roster_ids.len(),
        5,
        "目标队保持 5 人不变式"
    );
}

/// 同窗互斥保真（t13）：同一批内先转会改动目标队后，后一笔指向同一队应被互斥跳过
/// （保留原「防两笔转会抢同一队」语义，与快照过期区分开）。
#[test]
fn same_window_mutual_exclusion_skips_second_claim_on_same_team() {
    let mut world = World::new();
    let to = world.create_team("OnlyTeam", 1, 2000);
    for name in ["dev1ce", "npcA", "npcB", "npcC", "npcWeak"] {
        world
            .create_npc(
                Tier::Tier1,
                Some(to),
                None,
                Some(name),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
    }
    world.team_mut(to).unwrap().budget = 5_000_000;
    // 两名自由身玩家（同一批都想转会到 OnlyTeam）。
    let p1 = world.create_player(
        Tier::Tier1,
        Some("P1"),
        &mut Xoshiro256StarStar::seed(2),
        None,
        None,
    );
    let p2 = world.create_player(
        Tier::Tier1,
        Some("P2"),
        &mut Xoshiro256StarStar::seed(3),
        None,
        None,
    );
    boost_power(&mut world, p1);
    boost_power(&mut world, p2);

    let mut vrs = vrs_with_teams(&[(
        "OnlyTeam",
        1,
        2000,
        vec!["dev1ce", "npcA", "npcB", "npcC", "npcWeak"],
    )]);
    let clock = SimClock::of(2026, 6, 1);
    let offers = TransferEngine::generate_offers(&world, &vrs, &clock.date_label());

    // 两名玩家都决策转会到 OnlyTeam（同一批两个决策）。
    let d1 = PlayerDecision::new(
        offers[0].point_id.clone(),
        offers[0].candidates[0].team_signature.clone(),
    );
    let d2 = PlayerDecision::new(
        offers[0].point_id.clone(),
        offers[0].candidates[0].team_signature.clone(),
    );
    let events = TransferEngine::execute_choices(
        &mut world,
        &mut vrs,
        &clock,
        &offers,
        &[d1, d2],
        &mut Xoshiro256StarStar::seed(7),
    )
    .expect("合法决策必成功");
    // 同队互斥：只有第一笔执行成功，第二笔被跳过（保留原语义）。
    assert_eq!(events.len(), 1, "同窗同队应只有一笔转会执行");
}
