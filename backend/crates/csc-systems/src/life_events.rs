//! 场外人生事件子系统（2026 指挥中心个人决策）：
//! 私下战队接触 / 假赛联系者 / 宫斗队友 / 战队正式接触 / 媒体采访 / 队友邀请。
//!
//! 设计边界：只做**决策点生成**与**决策效果应用**；公式零持有、
//! 无跨调用状态。决策经既有 `DecisionRecorder` 进决策日志，可复现。

use csc_decision::point::{DecisionPoint, LifeEventKind, LifeOption};
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::chemistry_model::BlunderReaction;
use csc_simulation::directives::BlunderKind;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;
use csc_vrs::engine::VrsEngine;

use crate::chemistry::ChemistryEngine;

pub struct LifeEventsEngine;

impl LifeEventsEngine {
    /// 每月生成概率（%）：不是每月必刷，避免决策疲劳。
    pub const MONTHLY_CHANCE_PCT: i32 = 72;

    /// 同类型事件的冷却月数（Sprint 5 重复治理）。
    pub const KIND_COOLDOWN_MONTHS: i32 = 3;

    /// 月序（year*12+month），用于冷却窗口比较。
    pub fn month_index(date: &str) -> i32 {
        let mut parts = date.split('-');
        let year = parts
            .next()
            .and_then(|y| y.parse::<i32>().ok())
            .unwrap_or(0);
        let month = parts
            .next()
            .and_then(|m| m.parse::<i32>().ok())
            .unwrap_or(1);
        year * 12 + month
    }

    /// 是否仍处于同类型冷却期。
    pub fn cooling_down(
        last_kind: Option<&str>,
        last_month: i32,
        kind: LifeEventKind,
        month: i32,
    ) -> bool {
        last_kind == Some(kind.name()) && month - last_month < Self::KIND_COOLDOWN_MONTHS
    }

    /// 生成本月场外人生事件决策点（0 或 1 个；确定性 RNG；文案来自 `text` 包）。
    ///
    /// `vrs` 保留在签名中（batch.rs 调用点传 `ctx.vrs`）：TeamId 化（P2-4）后
    /// 接触方优先从 `transfer_contacts`（已含 team_id）反查；`transfer_contacts`
    /// 为空时**恢复 VRS 排名兜底**（经 `world.team_by_name` 反查 TeamId，零新 API）
    /// ——R1 返工：兜底分支必须保留，否则无 contacts 的主角 kinds 长度不同 →
    /// `rng.next_i32_bound(kinds.len())` 消费分叉 → 世界轨迹漂移（RNG 消费序
    /// 稳定性契约，见 P2-PLAN §2.1 风险①）。
    pub fn collect(
        world: &mut World,
        vrs: &VrsEngine,
        date: &str,
        rng: &mut Xoshiro256StarStar,
        text: &csc_text::TextBundle,
    ) -> Vec<DecisionPoint> {
        if rng.next_i32_bound(100) >= Self::MONTHLY_CHANCE_PCT {
            return Vec::new();
        }
        let Some(pc) = world.players.iter().find(|p| p.is_player() && !p.retired) else {
            return Vec::new();
        };
        let player_id = pc.id;
        let player_name = pc.name.clone();
        let current_team = pc
            .team
            .and_then(|tid| world.team(tid))
            .map(|t| t.name.clone());
        let teammate = pc.team.and_then(|tid| {
            world
                .roster(tid)
                .into_iter()
                .find(|p| p.id != player_id)
                .map(|p| (p.id, p.name.clone()))
        });
        // P2-4 TeamId 化：接触方以稳定 TeamId 键控（消除同名不同队/队改名的
        // point_id 歧义，收敛 SAVE-FORMAT §5.2 已知残留）。
        // ① 优先 transfer_contacts（TeamContact 已含 team_id）；
        // ② **恢复 VRS 排名兜底**（R1）：`transfer_contacts` 为空时从 VRS 排名
        //    取第一个非当前队条目，经 `world.team_by_name` 反查稳定 TeamId——
        //    **保持 RNG 消费序稳定**（kinds 长度不因 contacts 有无而分叉，见
        //    P2-PLAN §2.1 风险①）；查不到才跳过（kinds.retain 判据
        //    contact_team_id.is_none()）。
        let contact_team_id = pc
            .career
            .as_ref()
            .and_then(|c| c.transfer_contacts.first())
            .map(|c| c.team_id)
            .or_else(|| {
                vrs.ranking_entries()
                    .into_iter()
                    .find(|e| Some(e.team_name.as_str()) != current_team.as_deref())
                    .and_then(|e| world.team_by_name(&e.team_name))
            });
        let contact_team = contact_team_id
            .and_then(|tid| world.team(tid))
            .map(|t| t.name.clone());

        // 职业阶段事件池（Sprint 5）：不同年龄/声望段面对不同人生问题，
        // 避免 20 年生涯重复同一套事件。
        let mut kinds = Self::kinds_for_phase(Self::career_phase(
            pc.age,
            pc.career.as_ref().map(|c| c.reputation).unwrap_or(50),
        ));
        if teammate.is_none() {
            kinds.retain(|k| {
                *k != LifeEventKind::TeammatePowerStruggle && *k != LifeEventKind::TeammateInvite
            });
        }
        if contact_team_id.is_none() {
            kinds.retain(|k| {
                *k != LifeEventKind::TeamContact && *k != LifeEventKind::SecretTeamContact
            });
        }
        if kinds.is_empty() {
            kinds.push(LifeEventKind::Interview);
        }
        let month = Self::month_index(date);
        let last_kind = pc
            .career
            .as_ref()
            .and_then(|c| c.life_event_last_kind.clone());
        let last_month = pc
            .career
            .as_ref()
            .map(|c| c.life_event_last_month)
            .unwrap_or(0);
        let mut kind = kinds[rng.next_i32_bound(kinds.len() as i32) as usize];
        if Self::cooling_down(last_kind.as_deref(), last_month, kind, month) && kinds.len() > 1 {
            let alternatives: Vec<LifeEventKind> =
                kinds.iter().copied().filter(|k| *k != kind).collect();
            kind = alternatives[rng.next_i32_bound(alternatives.len() as i32) as usize];
        }

        let actor = match kind {
            LifeEventKind::Interview => text.get("life.actor.media").to_string(),
            LifeEventKind::MatchFixerContact => text.get("life.actor.fixer").to_string(),
            LifeEventKind::TeammateInvite | LifeEventKind::TeammatePowerStruggle => teammate
                .as_ref()
                .map(|t| t.1.clone())
                .unwrap_or_else(|| text.get("life.actor.teammate").to_string()),
            LifeEventKind::SecretTeamContact | LifeEventKind::TeamContact => contact_team
                .clone()
                .unwrap_or_else(|| text.get("life.actor.team").to_string()),
        };
        // 决策点 ID 的 actor 键（M1：PlayerId 键控；不得含 `|`——队名资产契约：
        // 当前资产队名无 `|`。SecretTeamContact/TeamContact 已 TeamId 化（P2-4）：
        // actor_key = `T:{team_id}`（稳定 ID，队名仅作展示字段）；`T:`/`A:` 前缀
        // 标记串类型防 `|` 歧义。
        let actor_key = match kind {
            LifeEventKind::TeammateInvite | LifeEventKind::TeammatePowerStruggle => teammate
                .as_ref()
                .map(|t| t.0.to_string())
                .unwrap_or_else(|| text.get("life.actor.teammate").to_string()),
            LifeEventKind::SecretTeamContact | LifeEventKind::TeamContact => {
                format!(
                    "T:{}",
                    contact_team_id
                        .map(|tid| tid.to_string())
                        .unwrap_or_else(|| text.get("life.actor.team").to_string())
                )
            }
            LifeEventKind::Interview | LifeEventKind::MatchFixerContact => format!("A:{}", actor),
        };

        let (detail, options) = match kind {
            LifeEventKind::SecretTeamContact => (
                text.format("life.secret.detail", &[&actor]),
                vec![
                    LifeOption {
                        id: "MEET".into(),
                        label: text.get("life.secret.opt.meet").into(),
                        description: text.get("life.secret.opt.meet.desc").into(),
                    },
                    LifeOption {
                        id: "DECLINE".into(),
                        label: text.get("life.secret.opt.decline").into(),
                        description: text.get("life.secret.opt.decline.desc").into(),
                    },
                    LifeOption {
                        id: "REPORT".into(),
                        label: text.get("life.secret.opt.report").into(),
                        description: text.get("life.secret.opt.report.desc").into(),
                    },
                ],
            ),
            LifeEventKind::MatchFixerContact => (
                text.get("life.fixer.detail").to_string(),
                vec![
                    LifeOption {
                        id: "REFUSE".into(),
                        label: text.get("life.fixer.opt.refuse").into(),
                        description: text.get("life.fixer.opt.refuse.desc").into(),
                    },
                    LifeOption {
                        id: "REPORT".into(),
                        label: text.get("life.fixer.opt.report").into(),
                        description: text.get("life.fixer.opt.report.desc").into(),
                    },
                    LifeOption {
                        id: "ACCEPT".into(),
                        label: text.get("life.fixer.opt.accept").into(),
                        description: text.get("life.fixer.opt.accept.desc").into(),
                    },
                ],
            ),
            LifeEventKind::TeammatePowerStruggle => (
                text.format("life.struggle.detail", &[&actor]),
                vec![
                    LifeOption {
                        id: "MEDIATE".into(),
                        label: text.get("life.struggle.opt.mediate").into(),
                        description: text.get("life.struggle.opt.mediate.desc").into(),
                    },
                    LifeOption {
                        id: "BACK_ACTOR".into(),
                        label: text.get("life.struggle.opt.back").into(),
                        description: text.format("life.struggle.opt.back.desc", &[&actor]),
                    },
                    LifeOption {
                        id: "SIDELINE".into(),
                        label: text.get("life.struggle.opt.sideline").into(),
                        description: text.get("life.struggle.opt.sideline.desc").into(),
                    },
                ],
            ),
            LifeEventKind::TeamContact => (
                text.format("life.contact.detail", &[&actor]),
                vec![
                    LifeOption {
                        id: "INTERESTED".into(),
                        label: text.get("life.contact.opt.interested").into(),
                        description: text.get("life.contact.opt.interested.desc").into(),
                    },
                    LifeOption {
                        id: "NOT_NOW".into(),
                        label: text.get("life.contact.opt.not_now").into(),
                        description: text.get("life.contact.opt.not_now.desc").into(),
                    },
                ],
            ),
            LifeEventKind::Interview => (
                text.get("life.interview.detail").to_string(),
                vec![
                    LifeOption {
                        id: "SPOTLIGHT".into(),
                        label: text.get("life.interview.opt.spotlight").into(),
                        description: text.get("life.interview.opt.spotlight.desc").into(),
                    },
                    LifeOption {
                        id: "HUMBLE".into(),
                        label: text.get("life.interview.opt.humble").into(),
                        description: text.get("life.interview.opt.humble.desc").into(),
                    },
                    LifeOption {
                        id: "DECLINE".into(),
                        label: text.get("life.interview.opt.decline").into(),
                        description: text.get("life.interview.opt.decline.desc").into(),
                    },
                ],
            ),
            LifeEventKind::TeammateInvite => (
                text.format("life.invite.detail", &[&actor]),
                vec![
                    LifeOption {
                        id: "JOIN".into(),
                        label: text.get("life.invite.opt.join").into(),
                        description: text.get("life.invite.opt.join.desc").into(),
                    },
                    LifeOption {
                        id: "LATER".into(),
                        label: text.get("life.invite.opt.later").into(),
                        description: text.get("life.invite.opt.later.desc").into(),
                    },
                    LifeOption {
                        id: "DECLINE".into(),
                        label: text.get("life.invite.opt.decline").into(),
                        description: text.get("life.invite.opt.decline.desc").into(),
                    },
                ],
            ),
        };

        if let Some(pc) = world.player_mut(player_id)
            && let Some(career) = pc.career_mut()
        {
            career.life_event_last_kind = Some(kind.name().to_string());
            career.life_event_last_month = month;
        }

        vec![DecisionPoint::LifeEvent {
            id: format!("{date}|life|{player_id}|{}|{}", kind.name(), actor_key),
            date: date.to_string(),
            player_id,
            player_name,
            kind,
            actor,
            detail,
            options,
            contact_team_id,
        }]
    }

    /// 职业阶段：新秀 / 成长 / 巅峰 / 衰退 / 末期。
    pub fn career_phase(age: i32, _reputation: i32) -> &'static str {
        if age <= 20 {
            "ROOKIE"
        } else if age <= 24 {
            "GROWTH"
        } else if age <= 29 {
            "PEAK"
        } else if age <= 33 {
            "DECLINE"
        } else {
            "LATE"
        }
    }

    /// 各职业阶段的事件池（Sprint 5）：
    /// - 新秀：机会 / 适应 / 定位；
    /// - 成长：首发竞争、第一次大赛压力、合同接触；
    /// - 巅峰：冠军压力、明星权责、商业与诚信风险；
    /// - 衰退：转型、替补竞争、恢复；
    /// - 末期：职业遗产、退役时机、更衣室传承。
    pub fn kinds_for_phase(phase: &str) -> Vec<LifeEventKind> {
        match phase {
            "ROOKIE" => vec![
                LifeEventKind::Interview,
                LifeEventKind::TeammateInvite,
                LifeEventKind::TeamContact,
            ],
            "GROWTH" => vec![
                LifeEventKind::Interview,
                LifeEventKind::SecretTeamContact,
                LifeEventKind::TeamContact,
                LifeEventKind::TeammatePowerStruggle,
            ],
            "PEAK" => vec![
                LifeEventKind::Interview,
                LifeEventKind::SecretTeamContact,
                LifeEventKind::MatchFixerContact,
                LifeEventKind::TeammatePowerStruggle,
            ],
            "DECLINE" => vec![
                LifeEventKind::Interview,
                LifeEventKind::TeamContact,
                LifeEventKind::TeammatePowerStruggle,
                LifeEventKind::TeammateInvite,
            ],
            _ => vec![
                LifeEventKind::Interview,
                LifeEventKind::TeammateInvite,
                LifeEventKind::SecretTeamContact,
            ],
        }
    }

    /// 应用人生事件决策（效果集中于此；point_id 供关系调整的可追溯来源）。
    pub fn apply(
        world: &mut World,
        point: &DecisionPoint,
        option_id: &str,
        journal: &mut WorldJournal,
    ) {
        let DecisionPoint::LifeEvent {
            player_id,
            player_name,
            kind,
            actor,
            contact_team_id,
            ..
        } = point
        else {
            return;
        };
        let player_id = *player_id;
        let year = point
            .date()
            .split('-')
            .next()
            .and_then(|y| y.parse::<i32>().ok())
            .unwrap_or(2026); // 印记归属年份
        let date = point.date().to_string();
        let team_id = world.player(player_id).and_then(|p| p.team);
        // P2-4：TeamContact/SecretTeamContact 的接触方按稳定 ID 反查展示名——
        // 消除「同名不同队/队改名」下按名匹配的歧义；`contact_team_id` 为 None
        // （旧档/兜底降级）时回退到 point.actor 展示串。
        let contact_name = contact_team_id
            .and_then(|tid| world.team(tid))
            .map(|t| t.name.clone())
            .unwrap_or_else(|| actor.clone());
        let teammate_id = team_id.and_then(|tid| {
            world
                .roster(tid)
                .into_iter()
                .find(|p| p.name == *actor)
                .map(|p| p.id)
        });

        let mut headline = format!("你处理了「{actor}」的事件");
        match kind {
            LifeEventKind::SecretTeamContact => match option_id {
                "MEET" => {
                    rep_delta(world, player_id, 1);
                    morale_delta(world, player_id, -1);
                    headline = "你私下会见了招募方".into();
                }
                "REPORT" => {
                    rep_delta(world, player_id, 3);
                    headline = "你把私下接触报告给了俱乐部".into();
                }
                _ => headline = "你婉拒了私下接触".into(),
            },
            LifeEventKind::MatchFixerContact => match option_id {
                "ACCEPT" => {
                    if let Some(career) =
                        world.player_mut(player_id).expect("玩家存在").career_mut()
                    {
                        career.finance.cash += 100_000;
                    }
                    rep_delta(world, player_id, -10);
                    morale_delta(world, player_id, -5);
                    headline = "你收下了那笔来路不明的钱".into();
                }
                "REPORT" => {
                    rep_delta(world, player_id, 4);
                    headline = "你向赛事方举报了假赛联系者".into();
                }
                _ => {
                    rep_delta(world, player_id, 2);
                    headline = "你拒绝并拉黑了假赛联系者".into();
                }
            },
            LifeEventKind::TeammatePowerStruggle => {
                if let (Some(pid), Some(tid)) = (teammate_id, team_id) {
                    let reaction = match option_id {
                        "MEDIATE" | "BACK_ACTOR" => BlunderReaction::Support,
                        _ => BlunderReaction::Ignore,
                    };
                    ChemistryEngine::apply_blunder_reaction(
                        world,
                        player_id,
                        pid,
                        BlunderKind::Tilt,
                        reaction,
                        tid,
                        year,
                        point.id(),
                        Some(journal),
                    );
                }
                headline = match option_id {
                    "MEDIATE" => "你出面调停了更衣室矛盾".into(),
                    "BACK_ACTOR" => "你选择站到了队友一边".into(),
                    _ => "你没有介入这场宫斗".into(),
                };
            }
            LifeEventKind::TeamContact => match option_id {
                "INTERESTED" => {
                    rep_delta(world, player_id, 1);
                    headline = "你向引援方表达了兴趣".into();
                }
                _ => headline = "你暂不考虑转会".into(),
            },
            LifeEventKind::Interview => match option_id {
                "SPOTLIGHT" => {
                    rep_delta(world, player_id, 3);
                    if let Some(tid) = team_id {
                        let team = world.team_mut(tid).expect("队伍存在");
                        team.chemistry.cohesion = (team.chemistry.cohesion - 1.0).max(0.0);
                    }
                    headline = "你在发布会上自信放话".into();
                }
                "HUMBLE" => {
                    if let Some(tid) = team_id {
                        for pid in world.roster(tid).iter().map(|p| p.id).collect::<Vec<_>>() {
                            morale_delta(world, pid, 2);
                        }
                    }
                    headline = "你把功劳归给了团队".into();
                }
                _ => headline = "你跳过了采访话题".into(),
            },
            LifeEventKind::TeammateInvite => match option_id {
                "JOIN" => {
                    relation_delta(world, team_id, player_id, teammate_id, 5.0);
                    morale_delta(world, player_id, 3);
                    if let Some(pid) = teammate_id {
                        morale_delta(world, pid, 3);
                    }
                    headline = "你赴约了队友的双排邀请".into();
                }
                "DECLINE" => {
                    relation_delta(world, team_id, player_id, teammate_id, -2.0);
                    headline = "你婉拒了队友的邀请".into();
                }
                _ => headline = "你答应下次再聚".into(),
            },
        }
        journal.record(WorldEvent::LiveUpdate {
            date,
            seq: -1,
            headline,
            detail: format!("{player_name} 对「{contact_name}」事件做出了选择：{option_id}"),
        });
    }
}

fn rep_delta(world: &mut World, player_id: PlayerId, delta: i32) {
    if let Some(career) = world
        .player_mut(player_id)
        .expect("玩家不存在")
        .career_mut()
    {
        career.reputation = (career.reputation + delta).clamp(0, 100);
    }
}

fn morale_delta(world: &mut World, player_id: PlayerId, delta: i32) {
    if let Some(pc) = world.player_mut(player_id) {
        pc.pro.morale = (pc.pro.morale + delta).clamp(0, 100);
    }
}

fn relation_delta(
    world: &mut World,
    team_id: Option<csc_util::id::TeamId>,
    player_id: PlayerId,
    teammate_id: Option<PlayerId>,
    delta: f64,
) {
    let (Some(tid), Some(pid)) = (team_id, teammate_id) else {
        return;
    };
    let (a, b) = {
        let p = world.player(player_id).expect("玩家存在");
        let t = world.player(pid);
        (
            p.name.clone(),
            t.map(|x| x.name.clone()).unwrap_or_default(),
        )
    };
    let roster: Vec<String> = world.roster(tid).iter().map(|p| p.name.clone()).collect();
    let team = world.team_mut(tid).expect("队伍存在");
    team.chemistry.adjust(&a, &b, delta);
    team.chemistry.cohesion =
        csc_simulation::chemistry_model::ChemistryModel::cohesion_of(&team.chemistry, &roster);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_window_and_month_index() {
        assert_eq!(LifeEventsEngine::month_index("2026-01-05"), 2026 * 12 + 1);
        assert_eq!(LifeEventsEngine::month_index("2027-02-28"), 2027 * 12 + 2);
        assert!(LifeEventsEngine::cooling_down(
            Some("INTERVIEW"),
            2026 * 12 + 1,
            LifeEventKind::Interview,
            2026 * 12 + 3
        ));
        assert!(!LifeEventsEngine::cooling_down(
            Some("INTERVIEW"),
            2026 * 12 + 1,
            LifeEventKind::Interview,
            2026 * 12 + 4
        ));
        assert!(!LifeEventsEngine::cooling_down(
            Some("INTERVIEW"),
            2026 * 12 + 1,
            LifeEventKind::TeamContact,
            2026 * 12 + 2
        ));
    }

    #[test]
    fn phase_boundaries_and_pools() {
        assert_eq!(LifeEventsEngine::career_phase(16, 50), "ROOKIE");
        assert_eq!(LifeEventsEngine::career_phase(20, 50), "ROOKIE");
        assert_eq!(LifeEventsEngine::career_phase(21, 50), "GROWTH");
        assert_eq!(LifeEventsEngine::career_phase(24, 50), "GROWTH");
        assert_eq!(LifeEventsEngine::career_phase(25, 50), "PEAK");
        assert_eq!(LifeEventsEngine::career_phase(29, 50), "PEAK");
        assert_eq!(LifeEventsEngine::career_phase(30, 50), "DECLINE");
        assert_eq!(LifeEventsEngine::career_phase(34, 50), "LATE");

        // 新秀期不出现假赛接触；巅峰期不再出现队友宵夜。
        assert!(
            !LifeEventsEngine::kinds_for_phase("ROOKIE")
                .contains(&LifeEventKind::MatchFixerContact)
        );
        assert!(
            !LifeEventsEngine::kinds_for_phase("PEAK").contains(&LifeEventKind::TeammateInvite)
        );
        assert!(
            LifeEventsEngine::kinds_for_phase("PEAK").contains(&LifeEventKind::MatchFixerContact)
        );
        assert!(LifeEventsEngine::kinds_for_phase("LATE").contains(&LifeEventKind::Interview));
    }

    /// P2-4：TeamContact/SecretTeamContact 的 actor_key 用稳定 TeamId（`T:{id}`），
    /// 不再用队名串——消除同名不同队/队改名的 point_id 歧义；contact_team_id
    /// 同步写入决策点（serde(default) 兼容旧档）。
    #[test]
    fn team_contact_actor_key_is_team_id_based() {
        use csc_decision::point::LifeEventKind as K;
        use csc_domain::team_tier::TeamTier;
        use csc_entities::transfer_contact::TeamContact as Contact;

        // 世界：三支队伍（队名用普通名；目标队 ID = 2）
        let mut world = World::new();
        let _t0 = world.create_team("Alpha", 1, 2000);
        let _t1 = world.create_team("Beta", 2, 1800);
        let target = world.create_team("Gamma", 3, 1600);
        let pid = world.create_player(
            csc_domain::tier::Tier::Tier4,
            Some("MyPlayer"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        // 主角带 transfer_contact（含 team_id = target）
        let career = world.player_mut(pid).unwrap().career_mut().unwrap();
        career.transfer_contacts.push(Contact {
            team_id: target,
            team_name: "Gamma".into(),
            ranking: 3,
            salary_offer: 100_000,
            reason: "x".into(),
            date: "2026-02-01".into(),
            tier: TeamTier::from_ranking(3),
        });

        // 强制命中 TeamContact：月序/冷却让候选稳定，直接调用 collect；
        // 断言产出的事件（若为 TeamContact/SecretTeamContact）id 含 `T:{target}`。
        let mut rng = Xoshiro256StarStar::seed(7);
        let vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let text = csc_text::TextBundle::default();
        let points = LifeEventsEngine::collect(&mut world, &vrs, "2026-02-01", &mut rng, &text);
        let contact = points.into_iter().find(|p| {
            matches!(
                p,
                DecisionPoint::LifeEvent {
                    kind: K::TeamContact | K::SecretTeamContact,
                    ..
                }
            )
        });
        if let Some(DecisionPoint::LifeEvent {
            id,
            contact_team_id,
            ..
        }) = contact
        {
            assert_eq!(
                contact_team_id,
                Some(target),
                "contact_team_id 应为接触方 TeamId"
            );
            assert!(
                id.ends_with(&format!("|T:{target}")),
                "actor_key 应为 T:{{TeamId}}：{id}"
            );
            assert!(
                !id.contains("Gamma"),
                "actor_key 不得含队名（TeamId 化）：{id}"
            );
        }
        // 无接触事件也可接受（RNG 未命中 TeamContact）——本测试只约束「命中时格式正确」。
    }
}
