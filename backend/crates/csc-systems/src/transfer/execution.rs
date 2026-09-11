//! 转会执行侧（transfer 子模块）：决策消费/续约/离队/买断/实际转会。
//!
//! 与市场侧（market.rs）分离（结构拆分前 transfer.rs 1239 行）。

use csc_decision::offer::TransferOffer;
use csc_decision::point::PlayerDecision;
use csc_entities::career::{CareerMemory, CareerMemoryKind};
use csc_entities::power::PowerCalculator;
use csc_entities::world::World;
use csc_simulation::finance::FinanceModel;
use csc_simulation::transfer_rules::TransferRules;
use csc_time::clock::SimClock;
use csc_util::SimError;
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use super::TransferEvent;

impl super::TransferEngine {
    /// 执行决策列表（[`PlayerDecision`]：STAY = 续约留队；LEAVE = 离队赋闲；
    /// 否则 = 目标队签名），返回实际发生的转会事件。
    ///
    /// 候选外目标（协议错误）→ `Err(SimError::Protocol)`（D6：决策源是外部输入边界，
    /// 不 panic）；目标队已被同窗转会改动（旧签名被替换移除）→ 互斥跳过并告警。
    /// 纯确定性（与 Kotlin 一致，不消费随机）。
    pub fn execute_choices(
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        offers: &[TransferOffer],
        decisions: &[PlayerDecision],
        rng: &mut Xoshiro256StarStar,
    ) -> Result<Vec<TransferEvent>, SimError> {
        let mut events = Vec::new();
        // 同窗互斥：记录本批次已被转会改动（目标队/源队）的队伍——保留「防两笔转会抢
        // 同一队」语义；与快照过期无关（P0 修复：target.team_id 才是稳定真实队伍）。
        let mut occupied: std::collections::HashSet<TeamId> = std::collections::HashSet::new();
        for decision in decisions {
            let Some(offer) = offers.iter().find(|o| o.point_id == decision.point_id) else {
                continue; // 与本次批次无关的决策（防御）
            };
            if decision.option_id == Self::STAY {
                // 合同到期留队 = 续约（降薪预期按 offer.discounted）；自由身 STAY = 继续待业
                if offer.from_team_id.is_some() {
                    Self::re_sign_expired(
                        world,
                        clock,
                        offer.player_id,
                        offer.from_team_id,
                        offer.discounted,
                    );
                } else {
                    Self::tick_unsigned(world, offer.player_id);
                }
                continue;
            }
            if decision.option_id == Self::LEAVE {
                // 离队赋闲：合同到期主动离开 → 自由市场（原队补位保持 5 人）
                Self::leave_team(world, vrs, offer.player_id, offer.from_team_id, rng);
                continue;
            }
            let Some(target) = offer
                .candidates
                .iter()
                .find(|c| c.team_signature == decision.option_id)
            else {
                return Err(SimError::protocol(format!(
                    "决策源返回候选外目标「{}」——不在玩家「{}」的候选清单中",
                    decision.option_id, offer.player_name
                )));
            };
            // 执行时按 **team_id** 解析目标队（P0 修复）：`TransferTarget.team_id` 取自
            // world 队伍，稳定且确定性；`team_signature`（VRS 快照 roster）仅作展示/选项 id，
            // 与 world 当前 roster 可能存在正常时差（NPC 转会/阵容变动后 VRS 未同步）。
            // 用签名反查会在 VRS↔world 漂移时误判「同窗被占用」→ 转会永不生效。
            let fresh_id = target.team_id;
            // 同窗互斥（保真）：本批次已有转会改动该队（真被占用）→ 跳过并告警。
            if occupied.contains(&fresh_id) {
                eprintln!(
                    "[TransferEngine] 玩家「{}」候选目标「{}」已被同窗转会改动（该队已有人加入）——互斥跳过",
                    offer.player_name, target.team_signature
                );
                continue;
            }
            let ev = Self::execute_transfer(
                world,
                vrs,
                clock,
                offer.player_id,
                offer.from_team_id,
                fresh_id,
                Self::transfer_fee_of(world.player(offer.player_id).unwrap()),
                offer.discounted,
            );
            // 标记目标队与源队已被本批次改动（后续同窗互斥依据）。
            occupied.insert(fresh_id);
            if let Some(from) = offer.from_team_id {
                occupied.insert(from);
            }
            events.push(ev);
        }
        Ok(events)
    }

    /// 合同到期留队 = **续约**（2026 合同状态机）：按市场价重签——
    /// 薪资 = 实力+声誉定薪、年限 = 能力值合同期，并向队伍收取签约薪资；
    /// `discounted` = 全价无队可签后的降薪预期（×[`csc_decision::offer::SALARY_CUT_FACTOR`]）。
    /// 签约即清零待业月数。
    ///
    /// 仅对合同已到期（≤0 年）且仍有队伍的玩家生效；合同期内调用为 no-op。
    fn re_sign_expired(
        world: &mut World,
        clock: &SimClock,
        player_id: PlayerId,
        from_team: Option<TeamId>,
        discounted: bool,
    ) {
        let Some(team_id) = from_team else {
            return; // 自由身 STAY：无队伍可续约
        };
        let Some(pc) = world.player(player_id) else {
            return;
        };
        let Some(career) = pc.career.as_ref() else {
            return;
        };
        if career.contract_years > 0 || pc.team != Some(team_id) {
            return; // 合同期内 / 归属已变 → 非续约场景
        }
        let power = PowerCalculator::player_power(pc);
        let reputation = career.reputation;
        let new_years = TransferRules::contract_duration(power);
        let market = TransferRules::salary_of(power, reputation) as f64;
        let new_salary = if discounted {
            (market * csc_decision::offer::SALARY_CUT_FACTOR) as i64
        } else {
            market as i64
        };
        {
            let career = world
                .player_mut(player_id)
                .expect("玩家不存在")
                .career_mut()
                .expect("非玩家");
            career.contract_years = new_years;
            career.salary = new_salary;
            career.months_unsigned = 0;
        }
        // 经济闭环：签约薪资支出（与 execute_transfer 一致的入账口径，无转会费）
        world.team_mut(team_id).expect("队伍不存在").budget -= new_salary;
        eprintln!(
            "[TransferEngine] {} 续约 {}：年薪 {}{}（{} 年）",
            world
                .player(player_id)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            world
                .team(team_id)
                .map(|t| t.name.clone())
                .unwrap_or_default(),
            new_salary,
            if discounted { "（降薪）" } else { "" },
            new_years,
        );
        let _ = clock; // 续约本身无日期叙事（未来可加 journal 事件）
    }

    /// 自由身 STAY = 继续待业：待业月数 +4（转会窗间隔）。持续 ≥12 个月
    /// 强制「赋闲/待业」——薪资归零、不再发放原薪水（合同状态机第二层）。
    fn tick_unsigned(world: &mut World, player_id: PlayerId) {
        let Some(pc) = world.player_mut(player_id) else {
            return;
        };
        let name = pc.name.clone();
        let Some(career) = pc.career_mut() else {
            return;
        };
        career.months_unsigned = (career.months_unsigned + 4).min(99);
        if career.months_unsigned >= 12 && career.salary != 0 {
            eprintln!(
                "[TransferEngine] {name} 待业 {} 个月仍无签约 → 强制赋闲（薪资归零）",
                career.months_unsigned
            );
            career.salary = 0;
        }
    }
    /// 买断自己换队（2026 复审新增）：玩家主动支付违约金解除合同 → 自由市场，
    /// 下个转会窗即可自由签约。违约金 = 年薪 × 2（下限 100k），从个人现金扣除。
    /// 复用 `leave_team` 语义（离队 + 原队补位 + VRS 阵容迁移）。
    ///
    /// @return 实际支付的买断费
    pub fn buyout(
        world: &mut World,
        vrs: &mut VrsEngine,
        player_id: PlayerId,
        rng: &mut Xoshiro256StarStar,
    ) -> Result<i64, String> {
        let Some(pc) = world.player(player_id) else {
            return Err("玩家不存在".to_string());
        };
        let Some(career) = pc.career.as_ref() else {
            return Err("尚无生涯数据".to_string());
        };
        if career.contract_years <= 0 {
            return Err("当前没有在履行合同（自由身无需买断）".to_string());
        }
        let Some(team_id) = pc.team else {
            return Err("当前没有所属队伍".to_string());
        };
        let cost = (career.salary * 2).max(100_000);
        if career.finance.cash < cost {
            return Err(format!(
                "现金不足：买断费需要 {cost}（年薪 × 2，下限 100k）"
            ));
        }
        // 扣款 → 离队（leave_team 内部处理队伍补位与 VRS 迁移）
        world
            .player_mut(player_id)
            .expect("玩家已存在")
            .career_mut()
            .expect("生涯已存在")
            .finance
            .cash -= cost;
        Self::leave_team(world, vrs, player_id, Some(team_id), rng);
        Ok(cost)
    }
    /// 离队赋闲（LEAVE）：合同到期主动离开 → 自由市场。原队保持 5 人不变式：
    /// 优先签自由市场最强可负担 NPC 补位；自由市场为空时生成青训新秀（确定性 rng）。
    fn leave_team(
        world: &mut World,
        vrs: &mut VrsEngine,
        player_id: PlayerId,
        from_team: Option<TeamId>,
        rng: &mut Xoshiro256StarStar,
    ) {
        let Some(team_id) = from_team else {
            return; // 自由身 LEAVE = 继续赋闲，no-op
        };
        let Some(pc) = world.player(player_id) else {
            return;
        };
        if pc.team != Some(team_id) {
            return; // 归属已变（防御）
        }
        let old_sig = world.signature_of(team_id);
        world
            .release_player(player_id)
            .expect("内部不变量：玩家必存在于 roster");
        world
            .add_free_agent(player_id)
            .expect("内部不变量：玩家可进自由市场");
        // 主动离队：待业计时从零开始（赋闲状态机从下一个转会窗起算）
        if let Some(career) = world.player_mut(player_id).and_then(|p| p.career_mut()) {
            career.months_unsigned = 0;
            career.contract_years = 0;
        }

        // 补位：优先自由市场 NPC（最强者），否则青训新秀
        let power_of = |id: PlayerId| {
            world
                .player(id)
                .map(PowerCalculator::player_power)
                .unwrap_or(0.0)
        };
        let free_npc: Option<PlayerId> = world
            .players
            .iter()
            .filter(|p| p.is_npc() && !p.retired && p.team.is_none())
            .map(|p| p.id)
            .max_by(|a, b| power_of(*a).total_cmp(&power_of(*b)));
        if let Some(npc) = free_npc {
            world.remove_free_agent(npc);
            world
                .assign_player_to_team(npc, team_id)
                .expect("内部不变量：补位 NPC/原队必存在");
        } else {
            // 无自由市场 NPC → 生成青训新秀（唯一名）
            let base = csc_entities::generator::RandomPlayerGenerator::random_name(rng);
            let mut name = base.clone();
            let mut suffix = 2u32;
            while world.player_by_name(&name).is_some() {
                name = format!("{base}{suffix}");
                suffix += 1;
            }
            let age = crate::population::PopulationEngine::ROOKIE_MIN_AGE
                + rng.next_i32_bound(
                    crate::population::PopulationEngine::ROOKIE_MAX_AGE
                        - crate::population::PopulationEngine::ROOKIE_MIN_AGE
                        + 1,
                );
            world
                .create_npc(
                    csc_domain::tier::Tier::Tier4,
                    Some(team_id),
                    None,
                    Some(&name),
                    rng,
                    Some(age),
                )
                .expect("内部不变量：补位队伍必存在");
        }
        // VRS 阵容迁移
        let new_names: Vec<String> = world
            .roster(team_id)
            .iter()
            .map(|p| p.name.clone())
            .collect();
        vrs.apply_roster_change_verified(&old_sig, &new_names);
    }

    /// 转会费（培养补偿模型；自由身免收由调用方判定）。
    pub(crate) fn transfer_fee_of(pc: &csc_entities::character::PlayerCharacter) -> i64 {
        FinanceModel::transfer_fee(PowerCalculator::player_power(pc))
    }
    /// 执行一次转会：玩家替换目标队最弱 NPC；玩家原队由被换 NPC 补位
    /// （自由身首签则被换 NPC 进入自由市场）。同步队伍实体与 VRS 状态，
    /// 并结算转会费（原队收入 / 新队支出，自由身免收）。
    /// `discounted` = 降薪预期签约（×[`csc_decision::offer::SALARY_CUT_FACTOR`]）。
    /// 签约即清零待业月数（合同状态机）。
    #[allow(clippy::too_many_arguments)]
    fn execute_transfer(
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        player_id: PlayerId,
        from: Option<TeamId>,
        to: TeamId,
        fee: i64,
        discounted: bool,
    ) -> TransferEvent {
        let (player_name, power) = {
            let pc = world.player(player_id).expect("玩家不存在");
            (pc.name.clone(), PowerCalculator::player_power(pc))
        };
        let is_free_agent = from.is_none();
        let to_old_sig = world.signature_of(to);
        let from_old_sig = from.map(|tid| world.signature_of(tid));

        // 目标队：玩家替换最弱 NPC（限定 NPC）
        let to_team = world.team(to).expect("目标队伍不存在");
        assert!(!to_team.roster_ids.is_empty(), "目标队伍阵容为空，无法转会");
        let weakest = to_team
            .roster_ids
            .iter()
            .filter_map(|pid| world.player(*pid))
            .filter(|p| p.is_npc())
            .min_by(|a, b| {
                PowerCalculator::player_power(a).total_cmp(&PowerCalculator::player_power(b))
            })
            .map(|p| p.id)
            .expect("目标队伍无可替换的 NPC");
        // 不变式：非自由身时玩家必须位于原队 roster（防别名残留）
        if !is_free_agent {
            let from_id = from.unwrap();
            let in_roster = world
                .team(from_id)
                .map(|t| t.roster_ids.contains(&player_id))
                .unwrap_or(false);
            assert!(
                in_roster,
                "玩家「{player_name}」不在原队 roster，转会状态不一致"
            );
        }

        // —— 换人（World 原子操作，双端不变量自动维护）——
        // 先全部离队、再入队——`assign_player_to_team` 不清理旧归属，若先 assign
        // 玩家到目标队再 release，会把「目标队刚加入的玩家」误移除并让原队 roster
        // 残留玩家（双重归属）。正确顺序：玩家先离开原队 → 被换 NPC 离队 → 入队。
        if !is_free_agent {
            world
                .release_player(player_id)
                .expect("内部不变量：玩家必存在于 arena");
        }
        world
            .release_player(weakest)
            .expect("内部不变量：被换 NPC 必存在于 arena");
        // 玩家入目标队 + 被换 NPC 补原队 / 进自由市场
        world
            .assign_player_to_team(player_id, to)
            .expect("内部不变量：玩家/目标队必存在");
        if is_free_agent {
            world
                .add_free_agent(weakest)
                .expect("内部不变量：被换 NPC 必存在于 arena");
        } else {
            world
                .assign_player_to_team(weakest, from.unwrap())
                .expect("内部不变量：被换 NPC/原队必存在");
        }

        // VRS 阵容迁移（签名变化；出走 ≥ 3 人积分清零）
        let to_names: Vec<String> = world.roster(to).iter().map(|p| p.name.clone()).collect();
        vrs.apply_roster_change_verified(&to_old_sig, &to_names);
        if let Some(old_sig) = &from_old_sig {
            let from_id = from.unwrap();
            let from_names: Vec<String> = world
                .roster(from_id)
                .iter()
                .map(|p| p.name.clone())
                .collect();
            vrs.apply_roster_change_verified(old_sig, &from_names);
        }

        // 经济闭环：原队收取培养补偿（自由身免收）
        if let Some(from_id) = from {
            world.team_mut(from_id).expect("队伍不存在").budget += fee;
        }

        // 生涯数据：按能力值签新合同、按实力+声誉定薪（降薪预期 ×SALARY_CUT_FACTOR）；待业清零
        let joined_team_name = world
            .team(to)
            .map(|team| team.name.clone())
            .unwrap_or_default();
        let departed_team_name = from_old_sig
            .as_ref()
            .map(|sig| sig.split('|').next().unwrap_or("?").to_string());
        {
            let career = world
                .player_mut(player_id)
                .expect("玩家不存在")
                .career_mut()
                .expect("非玩家");
            let market = TransferRules::salary_of(power, career.reputation) as f64;
            career.contract_years = TransferRules::contract_duration(power);
            career.salary = if discounted {
                (market * csc_decision::offer::SALARY_CUT_FACTOR) as i64
            } else {
                market as i64
            };
            career.months_unsigned = 0;
            career.remember(CareerMemory {
                kind: CareerMemoryKind::TeamJoin,
                year: clock.year(),
                date: Some(clock.date_label()),
                event_name: Some(joined_team_name.clone()),
                detail: "你加入了一支新的职业队伍。".to_string(),
            });
            if from.is_some() {
                career.remember(CareerMemory {
                    kind: CareerMemoryKind::TeamDeparture,
                    year: clock.year(),
                    date: Some(clock.date_label()),
                    event_name: departed_team_name.clone(),
                    detail: "你离开了上一支队伍。".to_string(),
                });
            }
        }
        // 经济闭环：签约预算支出（薪资 + 转会费）
        let salary = world
            .player(player_id)
            .expect("玩家不存在")
            .career
            .as_ref()
            .unwrap()
            .salary;
        world.team_mut(to).expect("队伍不存在").budget -= salary + fee;

        TransferEvent {
            player_name,
            player_id,
            from_team: from_old_sig.map(|sig| sig.split('|').next().unwrap_or("?").to_string()),
            from_team_id: from,
            to_team: world.team(to).map(|t| t.name.clone()).unwrap_or_default(),
            to_team_id: to,
            contract_years: TransferRules::contract_duration(power),
            salary,
            fee,
            date: clock.date_label(),
        }
    }
}
