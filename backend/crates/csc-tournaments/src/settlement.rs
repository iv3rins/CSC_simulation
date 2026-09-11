//! 赛事结算层（Series Settlement，Kotlin `SeriesSettlement.kt` 转写）——
//! 从赛事引擎拆出的**结算职责单点**。
//!
//! 赛事引擎只负责「跑」（排程/分派/推进），全部「算账」收敛在这里：
//! - 系列赛结算：生涯回写（击杀/死亡/胜场/士气/声誉）+ 疲劳消耗 + 比赛日志 + VRS 积分提交；
//! - 赛事结算：奖金发放（队伍预算 + 玩家分成）、荣誉颁发（团队/个人/MVP）、参赛记录；
//! - 年度结算：逐图 Rating 累计（`YearlyRatingTracker`）+ TOP20 颁奖 + 年度统计采集。
//!
//! 转写差异：Kotlin 持 `EntityEngine`/`VrsEngine`/`SimClock`/`EconomyEngine`/
//! `WorldJournal` 引用 → Rust 只持有**跨调用状态**（`YearlyRatingTracker` +
//! `EconomyEngine` 无状态标记），其余依赖全部方法参数化（world/vrs/clock/journal）。

use crate::yearly_rating::YearlyRatingTracker;
use csc_domain::tier_profile::{tier_lan, tier_prize_pool};
use csc_entities::career::{IndividualHonour, IndividualHonourType, TeamHonour};
use csc_entities::world::World;
use csc_events::event::WorldEvent;
use csc_events::journal::WorldJournal;
use csc_simulation::condition::ConditionModel;
use csc_simulation::form::FormModel;
use csc_simulation::rating::RatingCalculator;
use csc_simulation::series::{SeriesResult, SeriesStage};
use csc_systems::economy::EconomyEngine;
use csc_time::clock::SimClock;
use csc_util::id::{PlayerId, TeamId};
use csc_vrs::engine::VrsEngine;

use crate::scheduled::TournamentResult;

/// 亚军判定（决赛败者）——荣誉颁发与奖金发放共用（结构拆分收敛：
/// 此前 award_event_honours 与 award_prize_money 各写一份）。
///
/// 依赖隐式契约「最后一场 series = 决赛」（各赛制引擎收官场）；加**守卫**：
/// 最后一场必须包含冠军，否则契约破坏时返回 `None`（宁可少发，不可错发——
/// 避免把亚军奖/荣誉误判给非决赛队伍）。
fn runner_up_of(result: &TournamentResult) -> Option<TeamId> {
    result
        .series
        .last()
        .and_then(|s| {
            if s.team_a_id == result.champion {
                Some(s.team_b_id)
            } else if s.team_b_id == result.champion {
                Some(s.team_a_id)
            } else {
                None // 最后一场不是决赛（契约破坏）
            }
        })
        .filter(|ru| *ru != result.champion)
}

/// 赛事结算层：持有跨调用状态（年度 Rating 累计器 + 历届 TOP20 榜单快照）。
///
/// **封装边界**：`yearly_rating`/`top20_history` 私有——外部（编排层）只能经
/// 访问器读写，防止越层直接 `clone` 私有状态破坏封装（P1 收窄）。
#[derive(Default)]
pub struct SeriesSettlement {
    /// 年度 Rating 累计器（跨赛事累计逐图 rating/图数，供 TOP20 结算）
    yearly_rating: YearlyRatingTracker,
    /// 历届年度 TOP20 榜单快照（颁奖后持久化——含 NPC 名次与入选依据，
    /// 解决「历史榜单不可复盘」缺口；结算后累计器清空不影响历史）
    top20_history: Vec<crate::top20::Top20YearBoard>,
}

impl SeriesSettlement {
    /// 年度 Rating 累计器只读快照（存档/查询边界；P1 越层访问收口后唯一读取口）。
    pub fn yearly_rating_snapshot(&self) -> YearlyRatingTracker {
        self.yearly_rating.clone()
    }

    /// 年度 Rating 累计器只读引用（内部/同 crate 结算用；避免防御性 clone）。
    pub fn yearly_rating(&self) -> &YearlyRatingTracker {
        &self.yearly_rating
    }

    /// 年度 Rating 累计器可变引用（内部结算推进用）。
    pub fn yearly_rating_mut(&mut self) -> &mut YearlyRatingTracker {
        &mut self.yearly_rating
    }

    /// 从存档恢复年度累计器（读档边界；P1 越层访问收口后唯一写入口）。
    pub fn restore_yearly_rating(&mut self, tracker: YearlyRatingTracker) {
        self.yearly_rating = tracker;
    }

    /// 历届 TOP20 榜单快照（只读；查询/存档边界）。
    pub fn top20_history(&self) -> &[crate::top20::Top20YearBoard] {
        &self.top20_history
    }

    /// 从存档恢复历届 TOP20 榜单快照（读档边界）。
    pub fn restore_top20_history(&mut self, history: Vec<crate::top20::Top20YearBoard>) {
        self.top20_history = history;
    }

    // —— 系列赛结算 ——

    /// 精确路径结算：生涯回写（K/D/胜场/士气/声誉）→ 疲劳 → 比赛日志 → VRS 积分。
    ///
    /// @param event_name 赛事名（比赛日志的叙事归属）
    pub fn settle_series(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        series: &SeriesResult,
        event_name: &str,
    ) {
        self.apply_career_stats(world, clock, series); // 个体击杀/死亡回写玩家生涯
        self.apply_fatigue(world, series); // 体况：疲劳消耗（图数 × 场地）
        self.record_match_played(journal, clock, series, event_name, true); // 世界事件日志（玩家队比赛）
        // 带模拟时钟时间戳结算：比赛发生在「当下」，ELO 结算无衰减（info=1）；
        // 时间衰减/历史筛选由 reseed 的滚动窗口（Engine 传入）作用于历史比赛生效。
        vrs.apply_match_result(
            &series.to_match_result(),
            Some(clock.now_epoch_seconds()),
            None,
        );
    }

    /// 粗略路径结算：士气反馈（NPC 对局同样有状态演化）+ 疲劳 + 比赛日志 +
    /// **年度 Rating 累计（2026 修订：NPC 对局也必须累计——否则 TOP20 榜上
    /// 只有主角，竞争真空导致主角无脑高排名）** + VRS 积分。
    pub fn settle_quick_series(
        &mut self,
        world: &mut World,
        vrs: &mut VrsEngine,
        clock: &SimClock,
        journal: &mut WorldJournal,
        series: &SeriesResult,
        event_name: &str,
    ) {
        // 士气反馈（粗略路径同样生效：NPC 无 CareerInfo，声誉自然跳过，只动 morale——
        // 胜率计算消费 morale，必须与回写对称）
        let a_won = series.winner_sig == series.team_a_sig;
        let b_won = series.winner_sig == series.team_b_sig;
        if a_won || b_won {
            let roster_a: Vec<PlayerId> = world
                .roster(series.team_a_id)
                .iter()
                .map(|p| p.id)
                .collect();
            let roster_b: Vec<PlayerId> = world
                .roster(series.team_b_id)
                .iter()
                .map(|p| p.id)
                .collect();
            for pid in roster_a {
                if let Some(pc) = world.player_mut(pid) {
                    FormModel::apply_series_outcome(pc, a_won);
                }
            }
            for pid in roster_b {
                if let Some(pc) = world.player_mut(pid) {
                    FormModel::apply_series_outcome(pc, b_won);
                }
            }
        }
        self.record_yearly_and_career(world, clock, series); // 年度累计：NPC 也上榜竞争
        self.apply_fatigue(world, series); // 体况：NPC 对局同样消耗疲劳
        // 事件流降噪（2026 试玩修订）：NPC 队之间的比赛不写 MatchPlayed 事件——
        // 世界赛事完整记录在 `TournamentResult`，journal 只保留玩家队比赛叙事。
        self.record_match_played(journal, clock, series, event_name, false);
        vrs.apply_match_result(
            &series.to_match_result(),
            Some(clock.now_epoch_seconds()),
            None,
        );
    }

    /// 赛事荣誉（生涯数据写入，仅作用于玩家）。
    ///
    /// @return 本赛事 MVP（玩家）：(昵称, PlayerId)；无玩家参赛/未达门槛 = None。
    ///         冠军事件日志的 MVP 字段由调用方消费本返回值写入。
    pub fn award_event_honours(
        &mut self,
        world: &mut World,
        clock: &SimClock,
        journal: &mut WorldJournal,
        result: &TournamentResult,
    ) -> Option<(String, PlayerId)> {
        let year = clock.year();

        // 1. 冠军团队荣誉 + 冠军声誉加成（仅玩家；NPC 无 CareerInfo 自然跳过）
        let champion_roster: Vec<PlayerId> =
            world.roster(result.champion).iter().map(|p| p.id).collect();
        let event_title = result.event.name.clone();
        for pid in &champion_roster {
            if let Some(pc) = world.player_mut(*pid) {
                if let Some(career) = pc.career_mut() {
                    career.honours.team.push(TeamHonour {
                        title: event_title.clone(),
                        year,
                        tier: result.event.tier,
                        detail: String::new(),
                    });
                }
                FormModel::apply_champion_bonus(pc);
            }
        }

        // 2. MVP / EVP：仅录入顶级赛事（Major/超级精英/Elite/T1；低级别不设荣誉）。
        //    按 PlayerId 键控累计（防同名不同队串数据，对齐 B3）；
        //    判定规则委托 [`crate::honours::HonourEvaluator`]（纯函数，独立单测）。
        //    2026 平衡性修订：MVP 只从冠军/亚军队伍产生（见 honours.rs）；
        //    **2026 修订（NPC 参与）**：评定收集全员（含 NPC）——此前 `is_player`
        //    过滤导致 MVP/EVP 永远只给主角、NPC 的 TOP20 荣誉维度恒 0。
        let mut overall: Vec<(PlayerId, TeamId, f64, i32)> = Vec::new(); // (pid, 队伍, Σrating, maps)
        let mut playoff: Vec<(PlayerId, TeamId, f64, i32)> = Vec::new(); // 仅淘汰赛
        let mvp_eligible = result.event.tier.is_elite();
        if mvp_eligible {
            for series in &result.series {
                for map in &series.maps {
                    let rounds = map.team_a_score + map.team_b_score;
                    for line in &map.lines {
                        let roster = if line.team_sig == series.team_a_sig {
                            series.team_a_id
                        } else {
                            series.team_b_id
                        };
                        let Some(pid) = world.player_by_name(&line.player_name) else {
                            continue;
                        };
                        if !world
                            .team(roster)
                            .map(|t| t.roster_ids.contains(&pid))
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        let rating = RatingCalculator::rating_of_counts(
                            line.kills,
                            line.deaths,
                            line.assists,
                            rounds,
                        );
                        match overall.iter_mut().find(|(p, _, _, _)| *p == pid) {
                            Some(e) => {
                                e.2 += rating;
                                e.3 += 1;
                            }
                            None => overall.push((pid, roster, rating, 1)),
                        }
                        if series.stage == SeriesStage::Playoff {
                            match playoff.iter_mut().find(|(p, _, _, _)| *p == pid) {
                                Some(e) => {
                                    e.2 += rating;
                                    e.3 += 1;
                                }
                                None => playoff.push((pid, roster, rating, 1)),
                            }
                        }
                    }
                }
            }
        }
        // 亚军判定（决赛败者；与 award_prize_money 同一契约，共用 runner_up_of）
        let runner_up: Option<TeamId> = runner_up_of(result);
        let decision = if mvp_eligible {
            crate::honours::HonourEvaluator::decide(&overall, &playoff, result.champion, runner_up)
        } else {
            crate::honours::HonourDecision {
                mvp: None,
                evps: Vec::new(),
            }
        };
        let mvp = decision.mvp;
        if let Some(mvp_id) = mvp {
            let mvp_name = world.player(mvp_id).expect("选手不存在").name.clone();
            if let Some(career) = world.player_mut(mvp_id).expect("选手不存在").career_mut() {
                career.honours.individual.push(IndividualHonour {
                    r#type: IndividualHonourType::Mvp,
                    year,
                    detail: format!("{} MVP", result.event.name),
                    tier: Some(result.event.tier),
                    event_name: Some(result.event.name.clone()),
                    rank: None,
                });
            }
            // 年度荣誉累计（全员含 NPC——TOP20 荣誉维度数据源）
            self.yearly_rating.record_honour(
                mvp_id,
                true,
                crate::top20::Top20Evaluator::mvp_score(result.event.tier),
            );
            FormModel::apply_mvp_bonus(world.player_mut(mvp_id).expect("选手不存在"));
            journal.record(WorldEvent::HonourAwarded {
                date: clock.date_label(),
                seq: -1,
                player_name: mvp_name,
                player_id: mvp_id,
                honour: format!("{} MVP", result.event.name),
            });
        }

        // 3. EVP 荣誉落地（判定已在 HonourEvaluator 完成）
        for evp_id in decision.evps {
            let evp_name = world.player(evp_id).expect("选手不存在").name.clone();
            if let Some(career) = world.player_mut(evp_id).expect("选手不存在").career_mut() {
                career.honours.individual.push(IndividualHonour {
                    r#type: IndividualHonourType::Evp,
                    year,
                    detail: format!("{} EVP", result.event.name),
                    tier: Some(result.event.tier),
                    event_name: Some(result.event.name.clone()),
                    rank: None,
                });
            }
            // 年度荣誉累计（全员含 NPC）
            self.yearly_rating.record_honour(
                evp_id,
                false,
                crate::top20::Top20Evaluator::evp_score(result.event.tier),
            );
            journal.record(WorldEvent::HonourAwarded {
                date: clock.date_label(),
                seq: -1,
                player_name: evp_name,
                player_id: evp_id,
                honour: format!("{} EVP", result.event.name),
            });
        }
        // 冠军荣誉日志（玩家）
        for pid in &champion_roster {
            if !world.player(*pid).map(|p| p.is_player()).unwrap_or(false) {
                continue;
            }
            journal.record(WorldEvent::HonourAwarded {
                date: clock.date_label(),
                seq: -1,
                player_name: career_player_name(world, *pid),
                player_id: *pid,
                honour: format!("{} 冠军", result.event.name),
            });
        }
        mvp.map(|pid| (world.player(pid).expect("选手不存在").name.clone(), pid))
    }

    /// 奖金发放（经济闭环的收入端）：冠军得奖池 60%、亚军（决赛败者）得 40% → 队伍预算；
    /// 玩家个人按名次获奖金分成（[`EconomyEngine::award_prize_share`]）。
    ///
    /// 亚军判定依赖隐式契约「最后一场 series = 决赛」（各赛制引擎收官场）。
    /// 加**守卫**：最后一场必须包含冠军，否则契约被破坏时不发放亚军奖金，
    /// 避免把亚军奖误发给非决赛队伍（宁可少发，不可错发）。
    pub fn award_prize_money(
        &mut self,
        world: &mut World,
        clock: &SimClock,
        result: &TournamentResult,
    ) {
        let pool = tier_prize_pool(result.event.tier) as i64;
        let year = clock.year();
        world
            .team_mut(result.champion)
            .expect("冠军队伍不存在")
            .budget += pool * 60 / 100;
        let runner_up: Option<TeamId> = runner_up_of(result);
        if let Some(ru) = runner_up.filter(|ru| *ru != result.champion) {
            world.team_mut(ru).expect("亚军队伍不存在").budget += pool * 40 / 100;
        }
        // 玩家奖金分成（经济闭环的玩家侧：冠军 15% / 亚军 8%）
        let champion_roster: Vec<PlayerId> =
            world.roster(result.champion).iter().map(|p| p.id).collect();
        for pid in champion_roster {
            let pc = world.player(pid).expect("选手不存在");
            if pc.is_player() {
                EconomyEngine::award_prize_share(world, pid, pool, 1, year);
            }
        }
        if let Some(ru) = runner_up.filter(|ru| *ru != result.champion) {
            let ru_roster: Vec<PlayerId> = world.roster(ru).iter().map(|p| p.id).collect();
            for pid in ru_roster {
                let pc = world.player(pid).expect("选手不存在");
                if pc.is_player() {
                    EconomyEngine::award_prize_share(world, pid, pool, 2, year);
                }
            }
        }
    }

    /// 把赛事对象记录到所有参赛玩家（非 NPC）的 `career.attended_tournaments`。
    pub fn record_participation(
        world: &mut World,
        teams: &[TeamId],
        event: &csc_domain::tournament::Tournament,
    ) {
        for tid in teams {
            let roster: Vec<PlayerId> = world.roster(*tid).iter().map(|p| p.id).collect();
            for pid in roster {
                if let Some(career) = world.player_mut(pid).expect("选手不存在").career_mut() {
                    career.attended_tournaments.push(event.clone());
                }
            }
        }
    }

    // —— 年度结算 ——

    /// 年度统计（生涯档案素材；供 Engine 跨年归档，随后 [`Self::year_end_awards`] 清空累计器）。
    pub fn yearly_stats(
        &self,
    ) -> std::collections::HashMap<csc_util::id::PlayerId, crate::yearly_rating::YearlyStat> {
        self.yearly_rating.stats()
    }

    /// 年度结算：按 [`crate::top20::Top20Evaluator`] 综合分排名（加权 Rating +
    /// MVP/EVP 荣誉 + 淘汰赛硬仗 + 样本惩罚 + **天才少年通配**），给前 `top` 名
    /// 玩家颁发 TOP20 荣誉（含名次），并清空年度累计器。
    pub fn year_end_awards(
        &mut self,
        world: &mut World,
        clock: &SimClock,
        journal: &mut WorldJournal,
        year: i32,
        top: usize,
    ) -> Vec<PlayerId> {
        // 1. 综合评分（不可变借用 world + tracker；清空前完成；含通配外卡）
        let ranked: Vec<crate::top20::Top20Score> =
            crate::top20::Top20Evaluator::evaluate_with_wildcards(world, &self.yearly_rating, year)
                .into_iter()
                .take(top)
                .collect();

        let mut awarded: Vec<PlayerId> = Vec::new();
        let mut board_entries: Vec<crate::top20::Top20BoardEntry> =
            Vec::with_capacity(ranked.len());
        for (index, s) in ranked.iter().enumerate() {
            // 直接按 ID 反解实体（退役/不存在 → 防御跳过）
            let Some(pc) = world.player_mut(s.player_id) else {
                continue;
            };
            // 榜单快照（先于荣誉回写采集名字——含 NPC：历届榜单可复盘）
            board_entries.push(crate::top20::Top20BoardEntry {
                rank: index + 1,
                player_id: s.player_id,
                player_name: pc.name.clone(),
                score: s.score,
                weighted_rating: s.weighted_rating,
                honor_points: s.honor_points,
                playoff_rating: s.playoff_rating,
                maps: s.maps,
                mvp_count: s.mvp_count,
                evp_count: s.evp_count,
                wildcard: s.wildcard,
            });
            let Some(career) = pc.career_mut() else {
                continue; // 无生涯（NPC——荣誉不入 career，但名次已进榜单快照）
            };
            career.honours.individual.push(IndividualHonour {
                r#type: IndividualHonourType::Top20,
                year,
                detail: format!("{year} 年度 TOP20 第 {} 名", index + 1),
                tier: None,
                event_name: None,
                rank: Some(index as i32 + 1),
            });
            journal.record(WorldEvent::HonourAwarded {
                date: clock.date_label(),
                seq: -1,
                player_name: pc.name.clone(),
                player_id: s.player_id,
                honour: format!("{year} 年度 TOP20 第 {} 名", index + 1),
            });
            awarded.push(s.player_id);
        }
        // 3. 榜单快照持久化（先于清空累计器；NPC 名次由此可复盘）
        self.top20_history.push(crate::top20::Top20YearBoard {
            year,
            entries: board_entries,
        });
        // 4. 清空年度累计器（结算后复位）
        self.yearly_rating.clear();
        awarded
    }

    // —— 内部实现 ——

    /// 生涯回写：把系列赛里玩家（非 NPC）的击杀/死亡累计进其生涯，
    /// 胜方玩家加胜场；年度 Rating 累计委托 `YearlyRatingTracker`。
    /// NPC 不携带 CareerInfo，其战绩只保留在 SeriesResult 比赛日志中。
    fn apply_career_stats(&mut self, world: &mut World, clock: &SimClock, result: &SeriesResult) {
        // 系列赛胜场（series 粒度，三态匹配防签名错乱）：胜方队伍里每位玩家 +1
        let winner_roster: Vec<PlayerId> = if result.winner_sig == result.team_a_sig {
            world
                .roster(result.team_a_id)
                .iter()
                .map(|p| p.id)
                .collect()
        } else if result.winner_sig == result.team_b_sig {
            world
                .roster(result.team_b_id)
                .iter()
                .map(|p| p.id)
                .collect()
        } else {
            Vec::new() // 胜者签名与双方均不匹配（防御）：无人加胜场
        };
        for pid in &winner_roster {
            if let Some(career) = world.player_mut(*pid).expect("选手不存在").career_mut() {
                career.career_wins += 1;
            }
        }

        // 士气/声誉随胜负反馈（闭环：士气 → 下场胜率）
        let team_a_won = result.winner_sig == result.team_a_sig;
        let team_b_won = result.winner_sig == result.team_b_sig;
        if team_a_won || team_b_won {
            let roster_a: Vec<PlayerId> = world
                .roster(result.team_a_id)
                .iter()
                .map(|p| p.id)
                .collect();
            let roster_b: Vec<PlayerId> = world
                .roster(result.team_b_id)
                .iter()
                .map(|p| p.id)
                .collect();
            for pid in roster_a {
                if let Some(pc) = world.player_mut(pid) {
                    FormModel::apply_series_outcome(pc, team_a_won);
                }
            }
            for pid in roster_b {
                if let Some(pc) = world.player_mut(pid) {
                    FormModel::apply_series_outcome(pc, team_b_won);
                }
            }
        }

        // 击杀/死亡回写 + 年度 Rating 累计（每图）
        self.record_yearly_and_career(world, clock, result);
    }

    /// 年度 Rating 累计（**全员含 NPC**——TOP20 同榜竞争的数据源；2026 修订：
    /// 此前 NPC 对局走 quick 路径漏记，榜上只有主角）+ 玩家击杀/死亡生涯回写
    /// （NPC 无生涯自然跳过）。
    fn record_yearly_and_career(
        &mut self,
        world: &mut World,
        clock: &SimClock,
        result: &SeriesResult,
    ) {
        let roster_a_refs = world.roster(result.team_a_id);
        let roster_b_refs = world.roster(result.team_b_id);
        let mut career_updates: Vec<(PlayerId, i32, i32)> = Vec::new();
        for map in &result.maps {
            for line in &map.lines {
                let roster = if line.team_sig == result.team_a_sig {
                    &roster_a_refs
                } else {
                    &roster_b_refs
                };
                let Some(pc) = roster.iter().find(|p| p.name == line.player_name) else {
                    continue;
                };
                if pc.is_player() {
                    career_updates.push((pc.id, line.kills, line.deaths));
                }
            }
            self.yearly_rating.record(
                clock.year(),
                map,
                result.tier,
                result.stage,
                &result.team_a_sig,
                &roster_a_refs,
                &result.team_b_sig,
                &roster_b_refs,
            );
        }
        for (pid, kills, deaths) in career_updates {
            if let Some(career) = world.player_mut(pid).expect("选手不存在").career_mut() {
                career.career_kills += kills;
                career.career_deaths += deaths;
            }
        }
    }

    /// 体况结算：系列赛疲劳消耗（按图数与场地）。
    fn apply_fatigue(&self, world: &mut World, series: &SeriesResult) {
        let cost = ConditionModel::fatigue_cost(series.maps.len() as i32, tier_lan(series.tier));
        let mut ids: Vec<PlayerId> = Vec::new();
        ids.extend(world.roster(series.team_a_id).iter().map(|p| p.id));
        ids.extend(world.roster(series.team_b_id).iter().map(|p| p.id));
        for pid in ids {
            if let Some(pc) = world.player_mut(pid) {
                pc.fatigue = (pc.fatigue + cost).min(ConditionModel::FATIGUE_MAX);
            }
        }
    }

    /// 世界事件日志：一场系列赛（胜者/败者 + 比分 + 图数）。
    ///
    /// 比分**胜者在前**（`winner_score:loser_score`）——若按 A/B 侧固定顺序，
    /// 当胜者为 B 侧时叙事层会打出「X 以 11:13 带走对手」的矛盾文本（试玩捕获）。
    fn record_match_played(
        &self,
        journal: &mut WorldJournal,
        clock: &SimClock,
        series: &SeriesResult,
        event_name: &str,
        to_journal: bool,
    ) {
        if !to_journal {
            return; // 事件流降噪：NPC 队之间的比赛不写事件流（赛事完整记录在 TournamentResult）
        }
        let score = series
            .maps
            .iter()
            .map(|m| {
                let (w, l) = if m.winner_sig == series.team_a_sig {
                    (m.team_a_score, m.team_b_score)
                } else {
                    (m.team_b_score, m.team_a_score)
                };
                format!("{w}:{l}")
            })
            .collect::<Vec<_>>()
            .join(",");
        let (winner_id, loser_id) = if series.winner_sig == series.team_a_sig {
            (series.team_a_id, series.team_b_id)
        } else {
            (series.team_b_id, series.team_a_id)
        };
        let loser = if series.winner_sig == series.team_a_sig {
            series.team_b_sig.clone()
        } else {
            series.team_a_sig.clone()
        };
        journal.record(WorldEvent::MatchPlayed {
            date: clock.date_label(),
            seq: -1,
            event_name: event_name.to_string(),
            tier: series.tier,
            winner: series.winner_sig.clone(),
            winner_id,
            loser,
            loser_id,
            score,
            maps: series.maps.len() as i32,
        });
    }
}

/// 生涯持有者昵称（荣誉日志用）。
fn career_player_name(world: &World, pid: PlayerId) -> String {
    world
        .player(pid)
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_empty_rating() {
        let s = SeriesSettlement::default();
        assert!(s.yearly_rating().stats().is_empty());
    }
}
