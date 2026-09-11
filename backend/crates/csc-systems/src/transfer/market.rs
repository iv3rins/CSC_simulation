//! 转会市场情报与报价生成（transfer 子模块：只读/报价侧）。
//!
//! 与执行侧（execution.rs）分离：读市场/生成候选与写操作各占一个文件，
//! 降低单文件规模（结构拆分前 transfer.rs 1239 行）。

use csc_decision::offer::{TransferOffer, TransferTarget};
use csc_domain::team_tier::TeamTier;
use csc_entities::power::PowerCalculator;
use csc_entities::transfer_contact::TeamContact;
use csc_entities::world::World;
use csc_simulation::finance::FinanceModel;
use csc_simulation::transfer_rules::TransferRules;
use csc_util::SimError;
use csc_util::id::{PlayerId, TeamId};
use csc_vrs::engine::VrsEngine;

use super::{TransferInsight, sorted_names_of};

impl super::TransferEngine {
    /// 刷新「队伍主动接触」情报（2026 转会市场感知层）：
    /// 每月由世界批次调用，为合同剩余 ≤2 年的玩家生成表达招募兴趣的队伍列表。
    /// 这是**非决策点市场情报**（写入 `CareerInfo.transfer_contacts`），真正签约
    /// 仍在合同到期后的 `TransferWindow` 决策里完成。
    pub fn refresh_contacts(world: &mut World, vrs: &VrsEngine, date: &str) {
        let entries: Vec<csc_vrs::entry::VrsEntry> = vrs
            .all_teams()
            .iter()
            .map(|t| csc_vrs::entry::VrsEntry {
                ranking: t.ranking,
                points: t.points,
                team_name: t.team_name.clone(),
                roster: t.roster.clone(),
            })
            .collect();
        let player_ids: Vec<PlayerId> = world.all_players_only().iter().map(|p| p.id).collect();
        for pid in player_ids {
            let Some(pc) = world.player(pid) else {
                continue;
            };
            let Some(career) = pc.career.as_ref() else {
                continue;
            };
            // 合同还长（>2 年）不制造市场噪音；自由身/临到期球员开放接触
            if career.contract_years > 2 {
                continue;
            }
            let power = PowerCalculator::player_power(pc);
            let reputation = career.reputation;
            let market = TransferRules::salary_of(power, reputation);
            let current_team = pc.team;
            let current_ranking = current_team.and_then(|tid| {
                let sig = world.signature_of(tid);
                entries
                    .iter()
                    .find(|e| VrsEngine::signature_of(&e.team_name, &e.roster) == sig)
                    .map(|e| e.ranking)
            });
            // 接触情报不套用正式报价的预算/转会费过滤——预算不够的队也会先来接触。
            let current_team_name = current_team
                .and_then(|tid| world.team(tid))
                .map(|t| t.name.clone());
            let mut contacts: Vec<TeamContact> = Vec::new();
            for e in TransferRules::eligible_teams(pc, &entries) {
                if contacts.len() >= 3 {
                    break;
                }
                if current_team_name.as_deref() == Some(e.team_name.as_str()) {
                    continue;
                }
                let Some(tid) = world.team_by_name(&e.team_name) else {
                    continue;
                };
                let tier = TeamTier::from_ranking(e.ranking);
                let reason = if current_ranking.is_none_or(|cur| e.ranking < cur) {
                    "你的能力符合我们的首发规划，希望签下你补强阵容".to_string()
                } else {
                    "我们有薪资空间和稳定出场时间，正在物色即战力".to_string()
                };
                contacts.push(TeamContact {
                    team_id: tid,
                    team_name: e.team_name.clone(),
                    ranking: e.ranking,
                    salary_offer: market,
                    reason,
                    date: date.to_string(),
                    tier,
                });
            }
            if let Some(career) = world.player_mut(pid).and_then(|p| p.career_mut()) {
                career.transfer_contacts = contacts;
            }
        }
    }
    /// 生成转会候选（决策批次输入）：为每个自由身/合同到期的玩家生成
    /// [`TransferOffer`]（含按 VRS 排名升序的候选清单），offer 携带确定性
    /// pointId（`日期|transfer|玩家ID`）。
    ///
    /// 候选过滤（经济闭环）：目标队预算 ≥ 薪资 + 转会费。
    /// **2026 合同状态机**：合同到期 = 自由市场——候选不再限定「排名更高」；
    /// 全价无队可签时**降薪 40%**（[`csc_decision::offer::SALARY_CUT_FACTOR`]）重入市场；
    /// 降薪后仍无候选也出决策点（STAY 续约 / LEAVE 赋闲）。
    /// 纯确定性（与 Kotlin 一致，不消费随机——随机源只属于 NPC 市场与赛事模拟）。
    pub fn generate_offers(world: &World, vrs: &VrsEngine, date: &str) -> Vec<TransferOffer> {
        let mut offers = Vec::new();
        // 候选快照：同一转会窗内多笔转会互不基于过期 roster 决策
        let entries: Vec<csc_vrs::entry::VrsEntry> = vrs
            .all_teams()
            .iter()
            .map(|t| csc_vrs::entry::VrsEntry {
                ranking: t.ranking,
                points: t.points,
                team_name: t.team_name.clone(),
                roster: t.roster.clone(),
            })
            .collect();
        for pc in world.players.iter().filter(|p| p.is_player() && !p.retired) {
            let career = pc.career.as_ref().unwrap();
            if career.contract_years > 0 {
                continue; // 合同期内不转会
            }
            let current_team = pc.team;
            // 全价 → 无候选则降薪 40% 重试（合同状态机第一层）
            let full = Self::candidates_for(world, pc, &entries, current_team, 1.0);
            let (candidates, discounted) = if full.is_empty() {
                (
                    Self::candidates_for(
                        world,
                        pc,
                        &entries,
                        current_team,
                        csc_decision::offer::SALARY_CUT_FACTOR,
                    ),
                    true,
                )
            } else {
                (full, false)
            };
            offers.push(TransferOffer {
                point_id: format!("{date}|transfer|{}", pc.id),
                player_id: pc.id,
                player_name: pc.name.clone(),
                from_team_id: current_team,
                from_team_signature: current_team.map(|tid| world.signature_of(tid)),
                candidates,
                discounted,
                // 当前队 VRS 排名（auto 决策判断候选是否更强的依据；自由身 None）
                current_ranking: current_team.and_then(|tid| {
                    let sig = world.signature_of(tid);
                    entries
                        .iter()
                        .find(|e| {
                            csc_vrs::engine::VrsEngine::signature_of(&e.team_name, &e.roster) == sig
                        })
                        .map(|e| e.ranking)
                }),
            });
        }
        offers
    }

    /// 按薪资系数生成候选（`salary_factor` = 1.0 全价 / 0.6 降薪）。
    fn candidates_for(
        world: &World,
        pc: &csc_entities::character::PlayerCharacter,
        entries: &[csc_vrs::entry::VrsEntry],
        current_team: Option<TeamId>,
        salary_factor: f64,
    ) -> Vec<TransferTarget> {
        let career = pc.career.as_ref().unwrap();
        TransferRules::eligible_teams(pc, entries)
            .iter()
            .filter_map(|entry| {
                let sig = VrsEngine::signature_of(&entry.team_name, &entry.roster);
                // 实体按签名反查（签名可能含未登记队伍）
                world
                    .teams
                    .iter()
                    .find(|t| t.signature(&sorted_names_of(world, t.id)) == sig)
                    .map(|t| (entry, t.id))
            })
            .filter(|(_, tid)| {
                // 非当前队 + **预算约束**（薪资 × 系数 + 转会费）；合同到期可去任何付得起的队
                let team = world.team(*tid).unwrap();
                let market =
                    TransferRules::salary_of(PowerCalculator::player_power(pc), career.reputation)
                        as f64;
                let salary = (market * salary_factor) as i64;
                *tid != current_team.unwrap_or(csc_util::id::TeamId::NONE)
                    && FinanceModel::can_afford(team.budget, salary, Self::transfer_fee_of(pc))
            })
            .map(|(entry, tid)| {
                // 最弱 NPC（限定 NPC：替换 Player 会破坏生涯归属一致性）
                let team = world.team(tid).unwrap();
                let weakest = team
                    .roster_ids
                    .iter()
                    .filter_map(|pid| world.player(*pid))
                    .filter(|p| p.is_npc())
                    .min_by(|a, b| {
                        PowerCalculator::player_power(a)
                            .total_cmp(&PowerCalculator::player_power(b))
                    });
                let power_delta = weakest
                    .map(|w| PowerCalculator::player_power(pc) - PowerCalculator::player_power(w))
                    .unwrap_or(0.0);
                // 报价薪资（确定性）：玩家市场价 × 系数（全价 1.0 / 降薪 0.6）。
                let salary =
                    (TransferRules::salary_of(PowerCalculator::player_power(pc), career.reputation)
                        as f64
                        * salary_factor) as i64;
                TransferTarget {
                    team_id: tid,
                    team_signature: VrsEngine::signature_of(&entry.team_name, &entry.roster),
                    ranking: entry.ranking,
                    power_delta,
                    salary,
                }
            })
            .collect()
    }
    /// 玩家合同摘要（可随时查看队伍/薪资/剩余合同时长）。
    /// `player_id` 是外部输入（UI 查询）——反解失败返回 `Err` 而非 panic。
    pub fn contract_summary(world: &World, player_id: PlayerId) -> Result<String, SimError> {
        let pc = world
            .player(player_id)
            .ok_or_else(|| SimError::protocol(format!("玩家 {:?} 不存在", player_id)))?;
        let career = pc
            .career
            .as_ref()
            .ok_or_else(|| SimError::protocol(format!("「{}」不是玩家角色（无生涯）", pc.name)))?;
        let team_name = pc
            .team
            .map(|tid| world.team(tid).map(|t| t.name.clone()).unwrap_or_default())
            .unwrap_or_else(|| "自由身".into());
        Ok(format!(
            "{}：{} | 年薪 {} | 剩余合同 {} 年 | 实力 {}",
            pc.name,
            team_name,
            career.salary,
            career.contract_years,
            PowerCalculator::player_power(pc).round() as i32
        ))
    }
    /// 转会市场洞察：从当前状态推导「为什么现在能/不能收到报价」。
    /// 数据全部来自 world/vrs 的实际规则执行，前端只展示不猜测。
    pub fn market_insight(
        world: &World,
        vrs: &VrsEngine,
        player_id: PlayerId,
    ) -> Result<TransferInsight, SimError> {
        let pc = world
            .player(player_id)
            .ok_or_else(|| SimError::protocol(format!("玩家 {:?} 不存在", player_id)))?;
        let career = pc
            .career
            .as_ref()
            .ok_or_else(|| SimError::protocol(format!("「{}」不是玩家角色（无生涯）", pc.name)))?;
        let entries: Vec<csc_vrs::entry::VrsEntry> = vrs
            .all_teams()
            .iter()
            .map(|t| csc_vrs::entry::VrsEntry {
                ranking: t.ranking,
                points: t.points,
                team_name: t.team_name.clone(),
                roster: t.roster.clone(),
            })
            .collect();
        let current_team = pc.team;
        let current_rank = current_team.and_then(|tid| {
            let sig = world.signature_of(tid);
            entries
                .iter()
                .find(|e| VrsEngine::signature_of(&e.team_name, &e.roster) == sig)
                .map(|e| e.ranking)
        });
        let full = Self::candidates_for(world, pc, &entries, current_team, 1.0);
        let discounted = Self::candidates_for(
            world,
            pc,
            &entries,
            current_team,
            csc_decision::offer::SALARY_CUT_FACTOR,
        );
        let explanation = if career.contract_years > 0 {
            format!(
                "合同还剩 {} 年：转会窗不会对合同期内选手生成报价。",
                career.contract_years
            )
        } else if full.is_empty() && discounted.is_empty() {
            "当前没有队伍能同时满足你的薪资预期与培养补偿预算；可以考虑留队或等待市场变化。"
                .to_string()
        } else if full.is_empty() {
            format!(
                "全价薪资下无队可签，但有 {} 支队伍接受降薪方案——降薪会增加选择，但会减少收入。",
                discounted.len()
            )
        } else {
            format!(
                "当前有 {} 支队伍愿意按市场价签约，另有 {} 支接受降薪方案。",
                full.len(),
                discounted.len()
            )
        };
        Ok(TransferInsight {
            player_name: pc.name.clone(),
            contract_years: career.contract_years,
            salary: career.salary,
            reputation: career.reputation,
            current_team: current_team
                .map(|tid| world.team(tid).map(|t| t.name.clone()).unwrap_or_default()),
            current_team_rank: current_rank,
            months_unsigned: career.months_unsigned,
            eligible_teams_full_price: full.len(),
            eligible_teams_discounted: discounted.len(),
            explanation,
        })
    }
}
