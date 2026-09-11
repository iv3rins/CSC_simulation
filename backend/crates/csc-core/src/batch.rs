//! 世界级决策批次编排（Kotlin `WorldDecisionBatch.kt` 转写）：每月收集全部
//! 待决策点 → 决策 → 记录 → 应用。
//!
//! 职责：
//! - 收集：伤病 tick（恢复/掷伤/决策点）、训练点、转会窗点（每 4 个月）、代言评估；
//! - 决策：`DecisionSource::decide`（玩家意图层，纯接口）；
//! - 记录：`DecisionRecorder::record` 单点（决策日志 + 事件镜像，可复现性）；
//! - 应用：按决策点类型分派到对应子系统（转会/训练/伤病/代言）。
//!
//! 比赛级决策（图间干预/队友失误）在 `csc-tournaments::conductor` 现场处理，
//! 与本模块共享同一决策源与日志（Engine 装配时注入同一实例）。

use csc_decision::point::{DecisionPoint, TrainingOption};
use csc_decision::recorder::DecisionRecorder;
use csc_decision::source::DecisionSource;
use csc_events::event::WorldEvent;
use csc_simulation::training::{TrainingFocus, TrainingModel};
use csc_systems::economy::EconomyEngine;
use csc_systems::injury::InjuryEngine;
use csc_systems::life_events::LifeEventsEngine;
use csc_systems::transfer::TransferEngine;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;

use crate::context::WorldContext;

/// 收集完成后的真实世界批次。调用方必须连同收集后的世界/RNG 保存；
/// 此值不代表赛事/月步骤 continuation，也不包含传输层幂等身份。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingWorldBatch {
    pub date: String,
    pub points: Vec<DecisionPoint>,
    pub transfer_window_due: bool,
}

/// 世界级决策批次编排：无跨调用状态（全部依赖参数化）。
pub struct WorldDecisionBatch;

impl WorldDecisionBatch {
    /// 执行一次月度世界级决策批次。
    ///
    /// 决策源返回非法选项（候选外转会目标）→ `Err(SimError::Protocol)` 向上传播。
    ///
    /// @param month_idx 模拟月计数（转会窗判定：每 4 个月一次）
    /// @param transfer_window_due 本批次是否轮到转会窗
    pub fn run(
        ctx: &mut WorldContext<'_>,
        decision: &mut dyn DecisionSource,
        _month_idx: i32,
        rng: &mut Xoshiro256StarStar,
        transfer_window_due: bool,
    ) -> Result<(), csc_util::SimError> {
        let pending = Self::collect(ctx, rng, transfer_window_due);
        let decisions = if pending.points.is_empty() {
            Vec::new()
        } else {
            decision.decide(&pending.points)
        };
        Self::submit(ctx, &pending, &decisions, rng)
    }

    /// 只运行一次收集阶段，然后返回。所有随机抽样/世界 tick 已反映在 ctx 与 rng；
    /// 恢复应直接 submit 保存的批次，禁止重新 collect。
    pub fn collect(
        ctx: &mut WorldContext<'_>,
        rng: &mut Xoshiro256StarStar,
        transfer_window_due: bool,
    ) -> PendingWorldBatch {
        let date = ctx.clock.date_label();
        let mut points: Vec<DecisionPoint> = Vec::new();

        // 1. 伤病/体能 tick（恢复 + 掷伤；玩家新伤 → 决策点进批次）
        InjuryEngine::monthly_tick(ctx.world, rng, &mut points, &date, Some(ctx.journal));

        // 1.5 转会市场感知：每月刷新「谁在招人」主动接触情报（非决策点）
        TransferEngine::refresh_contacts(ctx.world, ctx.vrs, &date);

        // 1.6 场外人生事件（指挥中心个人决策）：私下接触/假赛/宫斗/采访/队友邀请
        points.extend(LifeEventsEngine::collect(
            ctx.world, ctx.vrs, &date, rng, ctx.text,
        ));

        // 2. 场外养成：**不再是每月强制决策**——训练由玩家在任意时间主动排定
        //    （`POST /games/{id}/training` → `CareerInfo.pending_training`），
        //    在下一月 `season::apply_pending_training` 中应用（见 season.rs）。
        let player_ids: Vec<PlayerId> = ctx.world.all_players_only().iter().map(|p| p.id).collect();

        // 3. 转会窗（每 4 个月一次；每玩家一个决策点）
        if transfer_window_due {
            let npc_events = TransferEngine::run_npc_market(ctx.world, ctx.vrs, ctx.clock, rng);
            for e in npc_events {
                ctx.journal.record(WorldEvent::TransferDone {
                    date: e.date.clone(),
                    seq: -1,
                    player_name: e.player_name.clone(),
                    player_id: e.player_id,
                    from_team: e.from_team.clone(),
                    from_team_id: e.from_team_id,
                    to_team: e.to_team.clone(),
                    to_team_id: e.to_team_id,
                    fee: e.fee,
                });
            }
            for offer in TransferEngine::generate_offers(ctx.world, ctx.vrs, &date) {
                points.push(DecisionPoint::TransferWindow {
                    id: offer.point_id.clone(),
                    date: date.clone(),
                    player_id: offer.player_id,
                    player_name: offer.player_name.clone(),
                    offers: vec![offer],
                });
            }
        }

        // 4. 代言评估（跨年后的第一个月批次）
        for pid in &player_ids {
            EconomyEngine::monthly_tick(ctx.world, *pid, &date, &mut points, rng);
        }

        PendingWorldBatch {
            date,
            points,
            transfer_window_due,
        }
    }

    /// 在收集后的同一权威状态上提交。协议/业务 Err 保留世界、VRS、RNG、
    /// 事件与决策日志；成功后批次生命周期/幂等仍由上层单一 owner 管理。
    pub fn submit(
        ctx: &mut WorldContext<'_>,
        pending: &PendingWorldBatch,
        decisions: &[csc_decision::point::PlayerDecision],
        rng: &mut Xoshiro256StarStar,
    ) -> Result<(), csc_util::SimError> {
        if pending.date != ctx.clock.date_label() {
            return Err(csc_util::SimError::protocol("世界批次日期已过期"));
        }
        csc_decision::validate_submission(&pending.points, decisions)
            .map_err(|e| csc_util::SimError::protocol(format!("决策提交校验失败：{e}")))?;
        // 受限事务副本：批次只修改这五个聚合，不复制赛事/档案/整个 GameState。
        let mut world = ctx.world.clone();
        let mut vrs = csc_vrs::engine::VrsEngine::from_database(ctx.vrs.database().clone());
        let mut journal = ctx.journal.clone();
        let mut log = ctx.decision_log.clone();
        let mut next_rng = *rng;
        let mut candidate = WorldContext {
            world: &mut world,
            vrs: &mut vrs,
            journal: &mut journal,
            decision_log: &mut log,
            tournaments: ctx.tournaments,
            clock: ctx.clock,
            archive: ctx.archive,
            rating_profile: ctx.rating_profile,
            text: ctx.text,
        };
        DecisionRecorder::record(
            candidate.decision_log,
            candidate.journal,
            &pending.date,
            &pending.points,
            decisions,
        );
        Self::apply_world_decisions(&mut candidate, &pending.points, decisions, &mut next_rng)?;
        if pending.transfer_window_due {
            candidate.vrs.settle_roster_window();
        }
        *ctx.world = world;
        *ctx.vrs = vrs;
        *ctx.journal = journal;
        *ctx.decision_log = log;
        *rng = next_rng;
        Ok(())
    }

    /// 应用世界级决策（按决策点类型分派到对应子系统）。
    fn apply_world_decisions(
        ctx: &mut WorldContext<'_>,
        points: &[DecisionPoint],
        decisions: &[csc_decision::point::PlayerDecision],
        rng: &mut Xoshiro256StarStar,
    ) -> Result<(), csc_util::SimError> {
        for point in points {
            if ctx.world.player(point.player_id()).is_none() {
                return Err(csc_util::SimError::protocol(format!(
                    "决策主体 {:?} 已不存在",
                    point.player_id()
                )));
            }
            let Some(d) = decisions.iter().find(|d| d.point_id == point.id()) else {
                continue;
            };
            match point {
                DecisionPoint::TransferWindow { offers, .. } => {
                    let offer = offers
                        .iter()
                        .find(|offer| offer.point_id == d.point_id)
                        .ok_or_else(|| csc_util::SimError::protocol("转会报价与决策身份不匹配"))?;
                    if offer.player_id != point.player_id()
                        || ctx
                            .world
                            .player(offer.player_id)
                            .and_then(|p| p.career.as_ref())
                            .is_none()
                        || offer
                            .from_team_id
                            .is_some_and(|id| ctx.world.team(id).is_none())
                    {
                        return Err(csc_util::SimError::protocol("转会报价主体或源队已失效"));
                    }
                    if let Some(target) = offer
                        .candidates
                        .iter()
                        .find(|c| c.team_signature == d.option_id)
                        && ctx.world.team(target.team_id).is_none()
                    {
                        return Err(csc_util::SimError::protocol("转会报价目标队已失效"));
                    }
                    let events = TransferEngine::execute_choices(
                        ctx.world,
                        ctx.vrs,
                        ctx.clock,
                        offers,
                        std::slice::from_ref(d),
                        rng,
                    )?;
                    for e in events {
                        ctx.journal.record(WorldEvent::TransferDone {
                            date: ctx.clock.date_label(),
                            seq: -1,
                            player_name: e.player_name,
                            player_id: e.player_id,
                            from_team: e.from_team,
                            from_team_id: e.from_team_id,
                            to_team: e.to_team,
                            to_team_id: e.to_team_id,
                            fee: e.fee,
                        });
                    }
                }
                DecisionPoint::TrainingFocus {
                    player_id, options, ..
                } => {
                    let pid = *player_id;
                    let focus = TrainingFocus::from_name(&d.option_id);
                    let focus = if options.iter().any(|o| o.focus.name() == d.option_id) {
                        focus
                    } else {
                        options
                            .first()
                            .map(|o| o.focus)
                            .unwrap_or(TrainingFocus::Aim)
                    };
                    // 环境上下文：团队凝聚力（训练氛围加成；自由身 = 中性）——在可变借用前读
                    let cohesion = ctx
                        .world
                        .player(pid)
                        .and_then(|pc| pc.team)
                        .and_then(|tid| ctx.world.team(tid))
                        .map(|t| t.chemistry.cohesion)
                        .unwrap_or(50.0);
                    if let Some(pc) = ctx.world.player_mut(pid) {
                        TrainingModel::apply_training(
                            pc,
                            focus,
                            rng,
                            csc_simulation::training::TrainingContext {
                                team_cohesion: cohesion,
                                season_goal: None,
                            },
                        );
                    }
                }
                DecisionPoint::InjuryDecision { player_id, .. } => {
                    InjuryEngine::apply_decision(ctx.world, *player_id, &d.option_id);
                }
                DecisionPoint::SponsorshipOffer { player_id, .. } => {
                    EconomyEngine::apply_sponsor_decision(
                        ctx.world,
                        *player_id,
                        &d.option_id,
                        ctx.clock.year(),
                        point,
                    );
                }
                DecisionPoint::LifeEvent { .. } => {
                    LifeEventsEngine::apply(ctx.world, point, &d.option_id, ctx.journal);
                }
                DecisionPoint::MatchIntervention { .. } | DecisionPoint::TeammateBlunder { .. } => {
                    return Err(csc_util::SimError::protocol("比赛决策不能通过世界批次提交"));
                }
            }
        }
        Ok(())
    }
}

/// 全部训练选项（训练页/协议展示；= Kotlin `TrainingFocus.entries.map`；
/// 选项 id = focus 枚举名，如 "AIM"）。
pub fn training_options() -> Vec<TrainingOption> {
    [
        TrainingFocus::Aim,
        TrainingFocus::Utility,
        TrainingFocus::Clutch,
        TrainingFocus::Physical,
        TrainingFocus::Mental,
        TrainingFocus::Communication,
        TrainingFocus::Rest,
    ]
    .iter()
    .map(|f| TrainingOption {
        focus: *f,
        label: f.label().to_string(),
        description: f.description().to_string(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_decision::log::DecisionLog;
    use csc_decision::source::AutoDecisionSource;
    use csc_entities::world::World;
    use csc_events::journal::WorldJournal;
    use csc_time::clock::SimClock;
    use csc_vrs::engine::VrsEngine;

    fn fixture<T>(
        f: impl FnOnce(&mut WorldContext<'_>, &mut Xoshiro256StarStar, PlayerId) -> T,
    ) -> T {
        let mut world = World::new();
        let pid = world.create_player(
            csc_domain::tier::Tier::Tier4,
            Some("P"),
            &mut Xoshiro256StarStar::seed(17),
            None,
            None,
        );
        let mut vrs = VrsEngine::from_json_files(&[]).unwrap();
        let mut clock = SimClock::of(2026, 6, 1);
        let mut log = DecisionLog::default();
        let mut journal = WorldJournal::default();
        let mut tournaments = csc_tournaments::engine::TournamentEngine::new();
        let mut archive = csc_career::archive::CareerArchive::default();
        let text = csc_text::TextBundle::default();
        let mut ctx = WorldContext {
            world: &mut world,
            vrs: &mut vrs,
            tournaments: &mut tournaments,
            clock: &mut clock,
            decision_log: &mut log,
            journal: &mut journal,
            archive: &mut archive,
            rating_profile: None,
            text: &text,
        };
        f(&mut ctx, &mut Xoshiro256StarStar::seed(99), pid)
    }

    fn canonical(ctx: &WorldContext<'_>, rng: &Xoshiro256StarStar) -> serde_json::Value {
        serde_json::to_value((
            &*ctx.world,
            ctx.vrs.database(),
            &*ctx.journal,
            &*ctx.decision_log,
            rng.snapshot(),
        ))
        .unwrap()
    }

    #[test]
    fn collected_batch_roundtrip_matches_synchronous_driver() {
        let expected = fixture(|ctx, rng, _| {
            WorldDecisionBatch::run(ctx, &mut AutoDecisionSource, 4, rng, true).unwrap();
            canonical(ctx, rng)
        });
        let actual = fixture(|ctx, rng, _| {
            let pending = WorldDecisionBatch::collect(ctx, rng, true);
            let after_collect = canonical(ctx, rng);
            let json = serde_json::to_string(&pending).unwrap();
            let restored: PendingWorldBatch = serde_json::from_str(&json).unwrap();
            assert_eq!(pending, restored);
            assert_eq!(
                after_collect,
                canonical(ctx, rng),
                "查询/序列化不能消耗随机或重做tick"
            );
            let decisions = AutoDecisionSource.decide(&restored.points);
            WorldDecisionBatch::submit(ctx, &restored, &decisions, rng).unwrap();
            canonical(ctx, rng)
        });
        assert_eq!(expected, actual);
    }

    #[test]
    fn failed_later_effect_does_not_commit_earlier_training_or_logs() {
        fixture(|ctx, rng, pid| {
            let point = |id: &str, player_id| DecisionPoint::TrainingFocus {
                id: id.into(),
                date: ctx.clock.date_label(),
                player_id,
                player_name: "P".into(),
                options: training_options(),
            };
            let pending = PendingWorldBatch {
                date: ctx.clock.date_label(),
                points: vec![point("first", pid), point("missing", PlayerId(99999))],
                transfer_window_due: false,
            };
            let before = canonical(ctx, rng);
            let decisions = vec![
                csc_decision::point::PlayerDecision::new("first", "AIM"),
                csc_decision::point::PlayerDecision::new("missing", "AIM"),
            ];
            // 形状完整合法；第一次训练已作用于候选状态，第二个主体失效才返回业务错误。
            assert!(csc_decision::validate_submission(&pending.points, &decisions).is_ok());
            assert!(WorldDecisionBatch::submit(ctx, &pending, &decisions, rng).is_err());
            assert_eq!(before, canonical(ctx, rng));
        });
    }

    #[test]
    fn rejected_input_keeps_pending_and_collected_state() {
        fixture(|ctx, rng, pid| {
            let pending = PendingWorldBatch {
                date: ctx.clock.date_label(),
                points: vec![DecisionPoint::TrainingFocus {
                    id: "train".into(),
                    date: ctx.clock.date_label(),
                    player_id: pid,
                    player_name: "P".into(),
                    options: training_options(),
                }],
                transfer_window_due: false,
            };
            let before = canonical(ctx, rng);
            assert!(WorldDecisionBatch::submit(ctx, &pending, &[], rng).is_err());
            assert_eq!(before, canonical(ctx, rng));
            WorldDecisionBatch::submit(
                ctx,
                &pending,
                &[csc_decision::point::PlayerDecision::new("train", "AIM")],
                rng,
            )
            .unwrap();
            assert_eq!(ctx.decision_log.count(), 1);
        });
    }

    #[test]
    fn training_focus_from_name_and_options() {
        assert_eq!(TrainingFocus::from_name("AIM"), TrainingFocus::Aim);
        assert_eq!(TrainingFocus::from_name("UTILITY"), TrainingFocus::Utility);
        assert_eq!(
            TrainingFocus::from_name("UNKNOWN"),
            TrainingFocus::Aim,
            "未知回退 Aim"
        );
        let options = training_options();
        assert_eq!(options.len(), 7, "6 项专项训练 + 1 项休整恢复");
        assert_eq!(options[0].focus.name(), "AIM");
        assert!(!options[0].label.is_empty());
        // 休整恢复是最后一个选项（不挤占自动模式的默认专项训练）
        assert_eq!(options[6].focus, TrainingFocus::Rest);
    }

    #[test]
    fn batch_does_not_force_monthly_training_decision() {
        let mut world = World::new();
        let pid = world.create_player(
            csc_domain::tier::Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(1),
            None,
            None,
        );
        let _ = pid;
        let mut vrs = VrsEngine::from_database(
            csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
        );
        let mut clock = SimClock::of(2026, 6, 1);
        let mut log = DecisionLog::default();
        let mut journal = WorldJournal::default();
        let mut auto = AutoDecisionSource;
        let mut ctx = WorldContext {
            world: &mut world,
            vrs: &mut vrs,
            tournaments: &mut csc_tournaments::engine::TournamentEngine::new(),
            clock: &mut clock,
            decision_log: &mut log,
            journal: &mut journal,
            archive: &mut csc_career::archive::CareerArchive::default(),
            rating_profile: None,
            text: &csc_text::TextBundle::default(),
        };
        WorldDecisionBatch::run(
            &mut ctx,
            &mut auto,
            5,
            &mut Xoshiro256StarStar::seed(1),
            false,
        )
        .expect("auto 批次必成功");
        // 训练已改为主动触发：月度批次不得再产出训练决策点
        let entries = log.entries();
        assert!(
            !entries.iter().any(|e| e.point_id.contains("|train|")),
            "月度批次不应再有训练决策"
        );
    }
}
