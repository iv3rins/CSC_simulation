//! 伤病/体能子系统（Kotlin `InjuryEngine.kt` 转写）：疲劳恢复、伤病触发、伤病决策应用。
//!
//! 分层：
//! - 规则：`csc-simulation::ConditionModel`（纯函数：消耗/恢复/概率）；
//! - 状态：`PlayerCharacter.fatigue` / `injury`（实体携带）；
//! - 决策：玩家伤病 → `DecisionPoint::InjuryDecision`（带伤上阵/休养），
//!   NPC 伤病直接生效（无决策，纯模拟）；
//! - 本引擎：月度 tick（恢复 + 倒计时 + 掷伤）+ 决策应用。
//!
//! 比赛消耗（fatigueCost）由赛事引擎赛后结算——本引擎不触碰比赛。

use csc_decision::point::{DecisionPoint, InjuryOption};
use csc_entities::character::PlayerCharacter;
use csc_entities::injury::Injury;
use csc_entities::mark::{CareerMarkType, CareerMarks};
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::condition::ConditionModel;
use csc_simulation::mark_effects::MarkEffects;
use csc_util::id::PlayerId;
use csc_util::rng::Xoshiro256StarStar;

/// 伤病/体能子系统：无跨调用状态。
pub struct InjuryEngine;

impl InjuryEngine {
    /// 伤病决策选项 id（与决策点选项对齐；= Kotlin companion 常量）。
    pub const PLAY_THROUGH: &'static str = "PLAY_THROUGH";
    pub const REST: &'static str = "REST";

    /// 月度 tick：
    /// 1. 疲劳自然恢复（全员）；
    /// 2. 伤病倒计时（受伤当月不恢复；休养中的玩家恢复双倍）；
    /// 3. 掷新伤（NPC 直接生效；玩家进入 `batch` 等待决策）；
    /// 4. 伤病缺勤日累计（生涯档案素材）。
    ///
    /// **事件流降噪（2026 试玩修订）**：伤病世界事件只记录「主角本人 + 主角队友」
    /// ——此前全世界 NPC 的伤病每月刷屏 journal（12 个月 1,486 条事件中伤病占
    /// 过半），淹没对玩家有意义的信息。NPC 伤病仍影响世界（状态照常演化），
    /// 只是不进事件流。
    ///
    /// @param batch 决策点收集器（玩家伤病点）
    /// @param date 当前日期（dateLabel）
    pub fn monthly_tick(
        world: &mut World,
        rng: &mut Xoshiro256StarStar,
        batch: &mut Vec<DecisionPoint>,
        date: &str,
        mut journal: Option<&mut WorldJournal>,
    ) {
        let year = date
            .get(..4)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
        let ids: Vec<PlayerId> = world
            .players
            .iter()
            .filter(|p| !p.retired)
            .map(|p| p.id)
            .collect();
        // 事件流相关性：主角本人 + 主角所在队的队友（玩家关心范围）
        let protagonist_team = world.all_players_only().first().and_then(|p| p.team);
        let relevant = |pc: &PlayerCharacter| {
            pc.is_player() || (pc.team.is_some() && pc.team == protagonist_team)
        };

        // —— 第一遍：疲劳恢复 + 伤病倒计时 ——
        for id in &ids {
            let pc = world.player_mut(*id).expect("选手不存在");
            pc.fatigue = (pc.fatigue - ConditionModel::monthly_recovery()).max(0.0);

            let Some(injury) = pc.injury.clone() else {
                continue;
            };
            let resting = pc.career.as_ref().map(|c| c.resting).unwrap_or(false);
            let ticked = if !injury.ticked {
                Injury {
                    ticked: true,
                    ..injury.clone()
                } // 受伤当月：只标记，不扣减
            } else if resting {
                injury.tick_resting() // 休养：双倍恢复
            } else {
                injury.tick() // 带伤：常规恢复
            };
            if pc.is_player() {
                pc.career.as_mut().unwrap().injury_days_this_year += 1;
            }
            pc.injury = Some(ticked.clone());

            if ticked.healed() {
                pc.injury = None;
                if pc.is_player() {
                    pc.career.as_mut().unwrap().resting = false;
                }
                if relevant(pc)
                    && let Some(j) = journal.as_deref_mut()
                {
                    j.record(WorldEvent::InjuryRecovered {
                        date: date.to_string(),
                        seq: -1,
                        player_name: pc.name.clone(),
                        player_id: *id,
                        kind: ticked.kind,
                    });
                }
                continue;
            }
        }

        // —— 第一遍后：有伤未愈的玩家 → 进决策批次（每月重新选择）——
        for id in &ids {
            let pc = world.player(*id).expect("选手不存在");
            if !pc.is_player() || pc.injury.is_none() {
                continue;
            }
            let point = Self::player_injury_point(world, *id, date);
            batch.push(point);
        }

        // —— 第二遍：无伤者掷新伤 ——
        for id in &ids {
            let pc = world.player(*id).expect("选手不存在");
            if pc.injury.is_some() {
                continue;
            }
            let prone = if pc.is_player() {
                MarkEffects::injury_prone_strength(&pc.career.as_ref().unwrap().marks)
            } else {
                0
            };
            let chance = ConditionModel::injury_chance(pc.age, pc.base.health, pc.fatigue, prone);
            if !rng.roll_bp(chance) {
                continue;
            }

            let kind = ConditionModel::roll_kind(rng);
            let severity = ConditionModel::roll_severity(rng);
            let injury = Injury {
                kind,
                severity,
                days_left: severity.recovery_days(),
                sustained_date: date.to_string(),
                source: "monthly".into(),
                ticked: false,
            };
            world.player_mut(*id).expect("选手不存在").injury = Some(injury.clone());
            if relevant(world.player(*id).expect("选手不存在"))
                && let Some(j) = journal.as_deref_mut()
            {
                j.record(WorldEvent::InjuryOccurred {
                    date: date.to_string(),
                    seq: -1,
                    player_name: world.player(*id).expect("选手不存在").name.clone(),
                    player_id: *id,
                    kind,
                    severity,
                });
            }
            // 玻璃人印记累积（玩家多次受伤 → INJURY_PRONE，影响未来概率）
            if world.player(*id).expect("选手不存在").is_player() {
                if let Some(career) = world.player_mut(*id).expect("选手不存在").career_mut() {
                    CareerMarks::apply(
                        &mut career.marks,
                        CareerMarkType::InjuryProne,
                        year,
                        "injury",
                        1,
                    );
                    // 伤病史归档（2026 试玩修复：字段此前从未写入——退役履历缺伤病记录）
                    career
                        .injury_history
                        .push(format!("{date} {:?} {:?}", injury.kind, injury.severity));
                }
                let point = Self::player_injury_point(world, *id, date);
                batch.push(point);
            }
        }
    }

    /// 玩家伤病决策点（选项：带伤上阵 / 休养）。
    fn player_injury_point(world: &World, player_id: PlayerId, date: &str) -> DecisionPoint {
        let pc = world.player(player_id).expect("玩家不存在");
        let injury = pc
            .injury
            .clone()
            .expect("玩家无伤病却生成伤病决策点（调用时序错误）");
        DecisionPoint::InjuryDecision {
            id: format!("{date}|injury|{}", pc.id),
            date: date.to_string(),
            player_id,
            player_name: pc.name.clone(),
            injury,
            options: vec![
                InjuryOption {
                    id: Self::PLAY_THROUGH.into(),
                    label: "带伤上阵".into(),
                    description: "实力受损，恢复变慢，二次受伤风险".into(),
                },
                InjuryOption {
                    id: Self::REST.into(),
                    label: "休养".into(),
                    description: "恢复加速，本月状态打折（半罚），减少缺勤".into(),
                },
            ],
        }
    }

    /// 应用玩家的伤病决策：
    /// - PLAY_THROUGH：保持全罚 + 恢复不变（mark 累积 INJURY_PRONE 已在掷伤处处理）；
    /// - REST：标记休养（恢复双倍、罚分半效）。
    pub fn apply_decision(world: &mut World, player_id: PlayerId, option_id: &str) {
        let Some(career) = world
            .player_mut(player_id)
            .expect("玩家不存在")
            .career_mut()
        else {
            return;
        };
        career.resting = option_id == Self::REST;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;
    use csc_entities::injury::{InjuryKind, InjurySeverity};

    fn world_with_player() -> (World, PlayerId) {
        let mut w = World::new();
        let p = w.create_player(
            Tier::Tier1,
            Some("P"),
            &mut Xoshiro256StarStar::seed(3),
            None,
            None,
        );
        (w, p)
    }

    #[test]
    fn rest_flag_and_tick_double_recovery() {
        let (mut w, p) = world_with_player();
        // 直接给玩家造一个伤（轻伤：10 天）
        let injury = Injury {
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Minor,
            days_left: 10,
            sustained_date: "2026-06-08".into(),
            source: "test".into(),
            ticked: false,
        };
        w.player_mut(p).unwrap().injury = Some(injury.clone());
        // 玩家选 REST
        InjuryEngine::apply_decision(&mut w, p, InjuryEngine::REST);
        assert!(w.player(p).unwrap().career.as_ref().unwrap().resting);

        let mut batch = Vec::new();
        let mut rng = Xoshiro256StarStar::seed(3);
        // 受伤当月只标记不扣减（Kotlin 语义）
        InjuryEngine::monthly_tick(&mut w, &mut rng, &mut batch, "2026-07-01", None);
        let left = w.player(p).unwrap().injury.as_ref().unwrap().days_left;
        assert_eq!(left, 10, "受伤当月只标记，不扣减");
        assert!(w.player(p).unwrap().injury.as_ref().unwrap().ticked);
        // 第二月：resting 双倍恢复（10 - 2 = 8）
        InjuryEngine::monthly_tick(&mut w, &mut rng, &mut batch, "2026-08-01", None);
        let left = w.player(p).unwrap().injury.as_ref().unwrap().days_left;
        assert_eq!(left, 8, "休养双倍恢复: {left}");
        // 未痊愈 → 每月重新决策
        assert!(!batch.is_empty());
        assert!(matches!(batch[0], DecisionPoint::InjuryDecision { .. }));
    }

    #[test]
    fn recovery_clears_injury_and_resting() {
        let (mut w, p) = world_with_player();
        // 已过受伤保护月（ticked=true），只剩 1 天 → 本月扣减后痊愈
        let injury = Injury {
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Minor,
            days_left: 1,
            sustained_date: "2026-06-08".into(),
            source: "test".into(),
            ticked: true,
        };
        w.player_mut(p).unwrap().injury = Some(injury);
        w.player_mut(p).unwrap().career.as_mut().unwrap().resting = false;
        let mut batch = Vec::new();
        let mut journal = WorldJournal::default();
        let mut rng = Xoshiro256StarStar::seed(3);
        InjuryEngine::monthly_tick(
            &mut w,
            &mut rng,
            &mut batch,
            "2026-07-01",
            Some(&mut journal),
        );
        assert!(w.player(p).unwrap().injury.is_none(), "痊愈后 injury 清空");
        assert!(
            !w.player(p).unwrap().career.as_ref().unwrap().resting,
            "痊愈后 resting 复位"
        );
        assert!(!journal.is_empty(), "痊愈事件入日志");
        assert!(batch.is_empty(), "痊愈后无伤病决策点");
    }

    #[test]
    fn injury_days_accumulate_while_injured() {
        let (mut w, p) = world_with_player();
        let injury = Injury {
            kind: InjuryKind::Wrist,
            severity: InjurySeverity::Minor,
            days_left: 5,
            sustained_date: "2026-06-08".into(),
            source: "test".into(),
            ticked: false,
        };
        w.player_mut(p).unwrap().injury = Some(injury);
        let mut batch = Vec::new();
        let mut rng = Xoshiro256StarStar::seed(3);
        InjuryEngine::monthly_tick(&mut w, &mut rng, &mut batch, "2026-07-01", None);
        assert_eq!(
            w.player(p)
                .unwrap()
                .career
                .as_ref()
                .unwrap()
                .injury_days_this_year,
            1
        );
    }

    #[test]
    fn healthy_player_gets_injury_point_when_struck() {
        let (mut w, p) = world_with_player();
        let mut batch = Vec::new();
        let mut journal = WorldJournal::default();
        // 固定种子必中伤（chance 与疲劳相关；直接把疲劳拉满；整数化判定后
        // seed=5 的首次基点骰 < 800 命中 8% 概率）
        w.player_mut(p).unwrap().fatigue = 100.0;
        let mut rng = Xoshiro256StarStar::seed(5);
        InjuryEngine::monthly_tick(
            &mut w,
            &mut rng,
            &mut batch,
            "2026-07-01",
            Some(&mut journal),
        );
        assert!(w.player(p).unwrap().injury.is_some(), "高疲劳应触发伤病");
        assert!(!batch.is_empty(), "玩家受伤应产生决策点");
        assert!(!journal.is_empty(), "伤病事件入日志");
    }
}
