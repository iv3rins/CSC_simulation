//! NPC 转会市场（从 [`super::transfer`] 拆出的**对手 AI 转会**职责单点）。
//!
//! 完整转会市场的另一半（玩家转会走决策管线，见 [`super::transfer::TransferEngine`]）：
//! NPC 队伍间的「低就挖角」——一名实力显著高于其当前队伍层级的 NPC，会被排名更靠前、
//! 且预算足以支付培养补偿费的队伍挖走（与目标队最弱 NPC 互换，保持 roster 5 人不变式）。
//!
//! 规则：
//! - **低就判定**：NPC 实力（`PowerCalculator::player_power`）对应的
//!   `TransferRules::max_joinable_tier` 高于其当前队伍的层级；
//! - **买家筛选**：候选队中「排名更靠前 + 预算 ≥ 转会费 + 存在比该 NPC 更弱的
//!   可替换 NPC」的最优一队；
//! - **成交**：以 [`TransferRules::NPC_MARKET_DEAL_PROBABILITY`] 掷骰（消费 rng，
//!   可复现），成交即与买家最弱 NPC 互换并结算转会费（买家支出 / 卖家收入）。
//!
//! 与玩家转会共享经济闭环（转会费 `FinanceModel::transfer_fee`）。

use csc_domain::team_tier::TeamTier;
use csc_entities::power::PowerCalculator;
use csc_entities::world::World;
use csc_simulation::finance::FinanceModel;
use csc_simulation::transfer_rules::TransferRules;
use csc_time::clock::SimClock;
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::transfer::{TransferEngine, TransferEvent, tier_strictly_higher};

impl TransferEngine {
    /// NPC 转会市场（对手 AI 模拟转会）——每转会窗执行一次，模拟 NPC 队伍间
    /// 的「低就挖角」。这是完整转会市场的另一半——玩家转会走决策管线，NPC 转会
    /// 由本方法按确定性规则 + 消费 rng 自动完成，两者共享经济闭环（转会费）。
    ///
    /// @param rng 主 RNG（消费随机，保证跨语言可复现）
    pub fn run_npc_market(
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        rng: &mut Xoshiro256StarStar,
    ) -> Vec<TransferEvent> {
        let mut events = Vec::new();
        // 快照全部 NPC（有队伍的），避免迭代中可变借用冲突
        let npc_ids: Vec<PlayerId> = world
            .players
            .iter()
            .filter(|p| p.is_npc() && p.team.is_some() && !p.retired)
            .map(|p| p.id)
            .collect();

        // 阵容连续性（2026-08 修复）：一支队伍每个转会窗最多参与 1 笔 NPC 交易。
        // 旧实现允许同一支 T1 队在单窗内连续吃进多名低级别明星——账面变强了，
        // 但 VRS 会判「3 人离队 → 积分清零」，王朝队反而被自己的补强拆散。
        let mut involved: std::collections::HashSet<TeamId> = std::collections::HashSet::new();

        // 当前队伍层级（按 VRS 排名；无 VRS 状态时用队伍缓存排名）
        let team_tier_of = |world: &World, vrs: &VrsEngine, tid: TeamId| -> TeamTier {
            let sig = world.signature_of(tid);
            let rank = vrs
                .team_of(&sig)
                .map(|t| t.ranking)
                .unwrap_or_else(|| world.team(tid).map(|t| t.vrs_ranking).unwrap_or(i32::MAX));
            TeamTier::from_ranking(rank)
        };

        for npc_id in npc_ids {
            let Some(pc) = world.player(npc_id) else {
                continue;
            };
            if pc.retired || pc.team.is_none() {
                continue;
            }
            let from_id = pc.team.unwrap();
            if involved.contains(&from_id) {
                continue; // 卖家本窗已参与交易：保护阵容连续性
            }
            let power = PowerCalculator::player_power(pc);
            let from_tier = team_tier_of(world, vrs, from_id);
            // 低就：NPC 能进更高层级，但困在低层级队伍
            if !tier_strictly_higher(TransferRules::max_joinable_tier(power), from_tier) {
                continue;
            }
            let fee = FinanceModel::transfer_fee(power);

            // 找最优买家：排名更靠前 + 预算够 + 有更弱可替换 NPC
            let mut best_buyer: Option<(TeamId, i32)> = None; // (买家, 排名)
            for team in world.teams.iter() {
                if team.id == from_id || involved.contains(&team.id) {
                    continue; // 买家本窗已参与交易：一支队伍一个窗只动一次阵容
                }
                let buyer_tier = team_tier_of(world, vrs, team.id);
                // 买家层级必须高于（严格更强）当前队
                if !tier_strictly_higher(buyer_tier, from_tier) {
                    continue;
                }
                if team.budget < fee {
                    continue;
                }
                // 买家必须存在比该 NPC 更弱的 NPC（可替换，保持 5 人）
                let has_weaker = team
                    .roster_ids
                    .iter()
                    .filter_map(|pid| world.player(*pid))
                    .filter(|p| p.is_npc())
                    .any(|p| PowerCalculator::player_power(p) < power);
                if !has_weaker {
                    continue;
                }
                let buyer_rank = vrs
                    .team_of(&world.signature_of(team.id))
                    .map(|t| t.ranking)
                    .unwrap_or(team.vrs_ranking);
                match best_buyer {
                    Some((_, best_rank)) if buyer_rank >= best_rank => {}
                    _ => best_buyer = Some((team.id, buyer_rank)),
                }
            }

            let Some((buyer_id, _)) = best_buyer else {
                continue;
            };
            // 掷骰成交（消费 rng，可复现；整数化判定 D2）
            if !rng.roll_bp(TransferRules::NPC_MARKET_DEAL_PROBABILITY) {
                continue;
            }
            if let Some(ev) =
                Self::execute_npc_swap(world, vrs, clock, npc_id, from_id, buyer_id, fee)
            {
                involved.insert(from_id);
                involved.insert(buyer_id);
                events.push(ev);
            }
        }
        events
    }

    /// 执行一次 NPC 互换：`npc_id`（卖家队）与买家最弱 NPC 互换，保持双方 roster
    /// 5 人不变式，并结算转会费（买家支出 / 卖家收入）。NPC 无个人薪资/合同，
    /// 转会费即培养补偿费（[`FinanceModel::transfer_fee`]）。
    ///
    /// 返回 `None` 仅当买家已无可替换 NPC（同窗前序交易改动了该队）。
    fn execute_npc_swap(
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        npc_id: PlayerId,
        from_id: TeamId,
        to_id: TeamId,
        fee: i64,
    ) -> Option<TransferEvent> {
        let npc_name = {
            let pc = world.player(npc_id)?;
            if pc.retired || pc.team != Some(from_id) {
                return None; // 前序交易已改动，互斥跳过
            }
            pc.name.clone()
        };

        let from_old_sig = world.signature_of(from_id);
        let to_old_sig = world.signature_of(to_id);

        // 买家最弱 NPC（限定 NPC；替换 Player 会破坏生涯归属一致性）
        let to_team = world.team(to_id)?;
        let weakest = to_team
            .roster_ids
            .iter()
            .filter_map(|pid| world.player(*pid))
            .filter(|p| p.is_npc())
            .min_by(|a, b| {
                PowerCalculator::player_power(a).total_cmp(&PowerCalculator::player_power(b))
            })
            .map(|p| p.id)?;

        // 互换（先全部离队、再入队，避免双重归属残留）
        world.release_player(npc_id).ok()?;
        world.release_player(weakest).ok()?;
        world.assign_player_to_team(npc_id, to_id).ok()?;
        world.assign_player_to_team(weakest, from_id).ok()?;

        // VRS 阵容迁移（双方签名均变化）
        let to_names: Vec<String> = world.roster(to_id).iter().map(|p| p.name.clone()).collect();
        vrs.apply_roster_change_verified(&to_old_sig, &to_names);
        let from_names: Vec<String> = world
            .roster(from_id)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        vrs.apply_roster_change_verified(&from_old_sig, &from_names);

        // 经济闭环：买家付转会费、卖家收转会费
        world.team_mut(to_id).expect("队伍不存在").budget -= fee;
        world.team_mut(from_id).expect("队伍不存在").budget += fee;

        Some(TransferEvent {
            player_name: npc_name,
            player_id: npc_id,
            from_team: Some(from_old_sig.split('|').next().unwrap_or("?").to_string()),
            from_team_id: Some(from_id),
            to_team: world
                .team(to_id)
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            to_team_id: to_id,
            contract_years: 0, // NPC 无个人合同
            salary: 0,         // NPC 无个人薪资（年度成本按人头计，见 settlement）
            fee,
            date: clock.date_label(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{boost_power, vrs_with_teams};
    use csc_domain::tier::Tier;

    /// 完整转会市场：低就 NPC 被更强队伍挖角。
    #[test]
    fn npc_market_poaches_underplaced_npc() {
        let mut world = World::new();
        // 弱队 T4（排名 200）：4 弱 NPC + 1 名"低就"强 NPC
        let weak_team = world.create_team("T4Team", 200, 100);
        for i in 0..4 {
            world
                .create_npc(
                    Tier::Tier4,
                    Some(weak_team),
                    None,
                    Some(&format!("weak{i}")),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                )
                .unwrap();
        }
        let star = world
            .create_npc(
                Tier::Tier4,
                Some(weak_team),
                None,
                Some("star"),
                &mut Xoshiro256StarStar::seed(1),
                None,
            )
            .unwrap();
        boost_power(&mut world, star); // power 拉满 → max_joinable_tier = T1

        // 强队 T1（排名 3）：预算充足，5 弱 NPC（最弱可被 star 替换）
        let strong_team = world.create_team("T1Team", 3, 1800);
        for i in 0..5 {
            world
                .create_npc(
                    Tier::Tier4,
                    Some(strong_team),
                    None,
                    Some(&format!("strong{i}")),
                    &mut Xoshiro256StarStar::seed(2),
                    None,
                )
                .unwrap();
        }
        world.team_mut(strong_team).unwrap().budget = 10_000_000;

        // VRS 登记两支队伍（签名匹配实体）
        let mut vrs = vrs_with_teams(&[
            (
                "T1Team",
                3,
                1800,
                vec!["strong0", "strong1", "strong2", "strong3", "strong4"],
            ),
            (
                "T4Team",
                200,
                100,
                vec!["star", "weak0", "weak1", "weak2", "weak3"],
            ),
        ]);
        let clock = SimClock::of(2026, 6, 1);

        let before_buyer = world.team(strong_team).unwrap().budget;
        let before_seller = world.team(weak_team).unwrap().budget;
        let events = TransferEngine::run_npc_market(
            &mut world,
            &mut vrs,
            &clock,
            &mut Xoshiro256StarStar::seed(9),
        );

        // star 低就 T4，唯一更高层买家是 T1Team；seed=9 首次基点骰 3840 < 5000（概率 0.5）必然成交
        assert_eq!(events.len(), 1, "seed=9 应恰好成交一笔: {events:?}");
        let ev = &events[0];
        assert_eq!(ev.player_name, "star");
        assert_eq!(ev.from_team.as_deref(), Some("T4Team"));
        assert_eq!(ev.to_team, "T1Team");
        // roster 保持 5 人
        assert_eq!(world.team(strong_team).unwrap().roster_ids.len(), 5);
        assert_eq!(world.team(weak_team).unwrap().roster_ids.len(), 5);
        // star 已入强队
        assert_eq!(world.player(star).unwrap().team, Some(strong_team));
        // 经济闭环：买家支出、卖家收入
        assert!(world.team(strong_team).unwrap().budget < before_buyer);
        assert!(world.team(weak_team).unwrap().budget > before_seller);
    }

    /// 无低就 NPC 时市场不产生任何转会（低就判定失败）。
    #[test]
    fn npc_market_no_underplaced_no_deals() {
        let mut world = World::new();
        // T1 强队：5 名实力匹配的 NPC（不低就）
        let t1 = world.create_team("T1Team", 3, 1800);
        for i in 0..5 {
            let n = world
                .create_npc(
                    Tier::Tier1,
                    Some(t1),
                    None,
                    Some(&format!("a{i}")),
                    &mut Xoshiro256StarStar::seed(1),
                    None,
                )
                .unwrap();
            boost_power(&mut world, n);
        }
        // T4 弱队：5 名弱 NPC
        let t4 = world.create_team("T4Team", 200, 100);
        for i in 0..5 {
            world
                .create_npc(
                    Tier::Tier4,
                    Some(t4),
                    None,
                    Some(&format!("b{i}")),
                    &mut Xoshiro256StarStar::seed(2),
                    None,
                )
                .unwrap();
        }
        let mut vrs = vrs_with_teams(&[
            ("T1Team", 3, 1800, vec!["a0", "a1", "a2", "a3", "a4"]),
            ("T4Team", 200, 100, vec!["b0", "b1", "b2", "b3", "b4"]),
        ]);
        let clock = SimClock::of(2026, 6, 1);
        let events = TransferEngine::run_npc_market(
            &mut world,
            &mut vrs,
            &clock,
            &mut Xoshiro256StarStar::seed(42),
        );
        // T4 弱 NPC 实力低 → max_joinable_tier 不高于 T4，不低就；无成交
        assert!(events.is_empty(), "无低就 NPC 不应有成交: {events:?}");
    }

    /// 阵容连续性：同一支队伍每窗最多参与 1 笔 NPC 交易。
    /// 旧实现会让第一名 T1 买家在同一个窗内连吃两名低级别明星，
    /// VRS 按 3 人离队清零——王朝队被自己的补强拆散。
    #[test]
    fn npc_market_caps_one_deal_per_team_per_window() {
        let mut world = World::new();
        // 两个 T1 买家（排名 1/2），各有 5 名可替换弱 NPC
        let buyer_a = world.create_team("BuyerA", 1, 2000);
        let buyer_b = world.create_team("BuyerB", 2, 1900);
        for (tid, prefix, seed) in [(buyer_a, "ba", 1u64), (buyer_b, "bb", 2u64)] {
            for i in 0..5 {
                world
                    .create_npc(
                        Tier::Tier4,
                        Some(tid),
                        None,
                        Some(&format!("{prefix}{i}")),
                        &mut Xoshiro256StarStar::seed(seed),
                        None,
                    )
                    .unwrap();
            }
            world.team_mut(tid).unwrap().budget = 10_000_000;
        }
        // 两个 T4 卖家，各有一名低就明星 + 4 名弱 NPC
        let seller_a = world.create_team("SellerA", 120, 100);
        let seller_b = world.create_team("SellerB", 121, 90);
        for (tid, star_name, weak_prefix, seed) in [
            (seller_a, "starA", "sa", 3u64),
            (seller_b, "starB", "sb", 4u64),
        ] {
            for i in 0..4 {
                world
                    .create_npc(
                        Tier::Tier4,
                        Some(tid),
                        None,
                        Some(&format!("{weak_prefix}{i}")),
                        &mut Xoshiro256StarStar::seed(seed),
                        None,
                    )
                    .unwrap();
            }
            let star = world
                .create_npc(
                    Tier::Tier4,
                    Some(tid),
                    None,
                    Some(star_name),
                    &mut Xoshiro256StarStar::seed(seed),
                    None,
                )
                .unwrap();
            boost_power(&mut world, star);
        }

        let mut vrs = vrs_with_teams(&[
            ("BuyerA", 1, 2000, vec!["ba0", "ba1", "ba2", "ba3", "ba4"]),
            ("BuyerB", 2, 1900, vec!["bb0", "bb1", "bb2", "bb3", "bb4"]),
            (
                "SellerA",
                120,
                100,
                vec!["starA", "sa0", "sa1", "sa2", "sa3"],
            ),
            (
                "SellerB",
                121,
                90,
                vec!["starB", "sb0", "sb1", "sb2", "sb3"],
            ),
        ]);
        let clock = SimClock::of(2026, 6, 1);
        // seed=8：前两笔掷骰均成交（0.5 概率序列 True,True）
        let events = TransferEngine::run_npc_market(
            &mut world,
            &mut vrs,
            &clock,
            &mut Xoshiro256StarStar::seed(8),
        );
        assert_eq!(events.len(), 2, "两名低就明星都应在市场成交: {events:?}");
        let mut from_ids = std::collections::HashSet::new();
        let mut to_ids = std::collections::HashSet::new();
        for e in &events {
            assert!(
                from_ids.insert(e.from_team_id.unwrap()),
                "同一卖家不应参与两笔 NPC 交易"
            );
            assert!(to_ids.insert(e.to_team_id), "同一买家不应参与两笔 NPC 交易");
        }
        assert!(
            from_ids.contains(&seller_a) && from_ids.contains(&seller_b),
            "两名低就明星都应离开原队"
        );
        assert!(
            to_ids.contains(&buyer_a) && to_ids.contains(&buyer_b),
            "两名 T1 买家应各收一名新援——而不是同一队连吃两笔"
        );
    }
}
