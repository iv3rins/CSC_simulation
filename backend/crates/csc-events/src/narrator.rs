//! 叙事层（Narrator）——把结构化 [`WorldEvent`] 渲染为**有代入感的第二人称叙事文本**。
//!
//! 定位（架构上的一块「产品差距」补全）：模拟引擎只产出冷数据（事件 struct），
//! 竞品文字游戏的护城河在「把冷数据翻译成人话/故事」。本模块是**纯函数叙事层**：
//! 输入事件 + 玩家视角（主角名/队伍名），输出 1~3 段中文叙事。
//!
//! 设计要点：
//! - **纯函数、无状态、无随机**：同输入同输出（不破坏可复现性——叙事是表现层
//!   派生，不进入决策/状态变更）；
//! - **第二人称主角视角**：涉及主角的事件用「你」，第三人用名字；
//! - **情感语调分区**：胜/负/伤/荣/退 各有专属措辞池，避免机械复述字段；
//! - **出口给 UI**：服务端把叙事文本随事件下发，前端不必再写 describe 逻辑。
//!
//! 文案外置（2026 结构拆分）：全部措辞收敛到 `assets/text/zh-CN.json`
//! （`narrator.*` 键），经 [`csc_text::TextBundle`] 注入——本模块不再持有
//! 任何硬编码中文文案；缺键回退键名便于发现缺失。

use csc_text::TextBundle;

use crate::event::WorldEvent;

/// 一段叙事（段落标题 + 正文；`aside` 为可选的补注/背景句）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narration {
    /// 一句话标题（日亮点）
    pub title: String,
    /// 主体叙事正文（可多句；用 \n 分段）
    pub body: String,
    /// 可选的背景/评注句（台侧解说视角，可空）
    pub aside: String,
}

/// 玩家视角上下文（Narrator 用其判断「是否主角」以选择人称）。
#[derive(Debug, Clone)]
pub struct NarratorView<'a> {
    /// 主角昵称（None = 无主角，全程第三人称）
    pub protagonist: Option<&'a str>,
    /// 主角当前队伍名（用于判断队伍级事件的「我方」归属）
    pub team: Option<&'a str>,
}

/// 叙事层级联：`Narrator::render(view, event) -> Option<Narration>`。
///
/// 返回 `None` 表示该事件无叙事价值（如纯内部决策镜像），UI 可回退到
/// 结构化字段展示或直接跳过。
pub struct Narrator;

impl Narrator {
    /// 把一条世界事件渲染为叙事文本（文案来自 `bundle`）。
    pub fn render(
        view: &NarratorView<'_>,
        event: &WorldEvent,
        bundle: &TextBundle,
    ) -> Option<Narration> {
        let pro = view.protagonist;
        let team = view.team;
        match event {
            WorldEvent::MatchPlayed {
                event_name,
                winner,
                winner_id: _,
                loser,
                loser_id: _,
                score,
                maps,
                tier,
                ..
            } => {
                // 主角视角：我方是否参赛
                let mine = team
                    .map(|t| sig_team(winner) == t || sig_team(loser) == t)
                    .unwrap_or(false);
                let i_won = team.map(|t| sig_team(winner) == t).unwrap_or(false);
                // 赛场瞬间（确定性：由比分悬殊度派生——同输入同输出，不破坏可复现性）
                let moment = bundle.get(match_moment_key(score));
                let (title, mut body) = match (mine, i_won) {
                    (true, true) => (
                        bundle.format("narrator.match.win.title", &[&strip(sig_team(loser))]),
                        bundle.format(
                            "narrator.match.win.body",
                            &[
                                event_name,
                                &maps.to_string(),
                                &strip(sig_team(winner)),
                                score,
                            ],
                        ),
                    ),
                    (true, false) => (
                        bundle.format("narrator.match.lose.title", &[&strip(sig_team(winner))]),
                        // score 为「胜者:败者」顺序——败者视角翻转成本方在前
                        bundle.format(
                            "narrator.match.lose.body",
                            &[event_name, &strip(sig_team(loser)), &flip_score(score)],
                        ),
                    ),
                    _ => (
                        bundle.format(
                            "narrator.match.neutral.title",
                            &[&strip(sig_team(winner)), &strip(sig_team(loser))],
                        ),
                        bundle.format(
                            "narrator.match.neutral.body",
                            &[
                                event_name,
                                &strip(sig_team(winner)),
                                score,
                                bundle.get(if *maps > 1 {
                                    "narrator.match.multi_map"
                                } else {
                                    "narrator.match.single_map"
                                }),
                                &maps.to_string(),
                            ],
                        ),
                    ),
                };
                if !moment.is_empty() {
                    body.push_str(moment);
                }
                let aside = bundle.format("narrator.match.aside", &[tier_name(*tier)]);
                Some(Narration { title, body, aside })
            }
            WorldEvent::Championship {
                event_name,
                champion,
                mvp,
                tier,
                ..
            } => {
                let mine = team.map(|t| champion == t).unwrap_or(false);
                let mvp_txt = match mvp {
                    Some(m) if Some(m.as_str()) == pro => {
                        bundle.get("narrator.champ.mvp.self").to_string()
                    }
                    Some(m) => bundle.format("narrator.champ.mvp.other", &[m]),
                    None => String::new(),
                };
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.champ.mine.title").to_string(),
                        bundle.format(
                            "narrator.champ.mine.body",
                            &[event_name, champion, &mvp_txt],
                        ),
                    )
                } else {
                    (
                        bundle.format("narrator.champ.other.title", &[champion, event_name]),
                        bundle.format(
                            "narrator.champ.other.body",
                            &[champion, event_name, &mvp_txt],
                        ),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: bundle.format("narrator.champ.aside", &[tier_name(*tier)]),
                })
            }
            WorldEvent::TournamentStage {
                event_name,
                stage,
                detail,
                ..
            } => Some(Narration {
                title: bundle.format("narrator.stage.title", &[event_name, stage]),
                body: detail.clone(),
                aside: String::new(),
            }),
            WorldEvent::TournamentCancelled {
                event_name,
                tier,
                reason,
                ..
            } => Some(Narration {
                title: bundle.format("narrator.cancelled.title", &[event_name]),
                body: bundle.format("narrator.cancelled.body", &[reason]),
                aside: tier_name(*tier).to_string(),
            }),
            WorldEvent::TransferDone {
                player_name,
                from_team,
                to_team,
                fee,
                ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let from = from_team
                    .as_deref()
                    .unwrap_or_else(|| bundle.get("narrator.transfer.free_market"));
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.transfer.mine.title").to_string(),
                        bundle.format("narrator.transfer.mine.body", &[from, to_team]),
                    )
                } else {
                    (
                        bundle.format("narrator.transfer.other.title", &[player_name, to_team]),
                        bundle.format(
                            "narrator.transfer.other.body",
                            &[player_name, from, to_team],
                        ),
                    )
                };
                let aside = if *fee > 0 {
                    bundle.format("narrator.transfer.aside.fee", &[&money_text(*fee)])
                } else {
                    bundle.get("narrator.transfer.aside.free").to_string()
                };
                Some(Narration { title, body, aside })
            }
            WorldEvent::Retirement {
                player_name,
                age,
                from_team,
                ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.retire.mine.title").to_string(),
                        bundle.format("narrator.retire.mine.body", &[&age.to_string()]),
                    )
                } else {
                    (
                        bundle.format("narrator.retire.other.title", &[player_name]),
                        bundle.format(
                            "narrator.retire.other.body",
                            &[player_name, &age.to_string()],
                        ),
                    )
                };
                let aside = from_team
                    .as_ref()
                    .map(|t| bundle.format("narrator.retire.aside", &[t]))
                    .unwrap_or_default();
                Some(Narration { title, body, aside })
            }
            WorldEvent::RookieIntake {
                player_name,
                age,
                into_team,
                ..
            } => Some(Narration {
                title: bundle.format("narrator.rookie.title", &[player_name]),
                body: bundle.format(
                    "narrator.rookie.body",
                    &[&age.to_string(), player_name, into_team],
                ),
                aside: String::new(),
            }),
            WorldEvent::InjuryOccurred {
                player_name,
                kind,
                severity,
                ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.injury.mine.title").to_string(),
                        bundle.format(
                            "narrator.injury.mine.body",
                            &[kind.label(), severity.label_cn()],
                        ),
                    )
                } else {
                    (
                        bundle.format("narrator.injury.other.title", &[player_name]),
                        bundle.format(
                            "narrator.injury.other.body",
                            &[player_name, kind.label(), severity.label_cn()],
                        ),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: bundle.get("narrator.injury.aside").to_string(),
                })
            }
            WorldEvent::InjuryRecovered {
                player_name, kind, ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.recover.mine.title").to_string(),
                        bundle.format("narrator.recover.mine.body", &[kind.label()]),
                    )
                } else {
                    (
                        bundle.format("narrator.recover.other.title", &[player_name]),
                        bundle.format("narrator.recover.other.body", &[player_name, kind.label()]),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: String::new(),
                })
            }
            WorldEvent::Conflict {
                player_name,
                teammate_name,
                severity,
                ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let (title, body) = if mine {
                    (
                        bundle.format("narrator.conflict.mine.title", &[teammate_name]),
                        bundle.format("narrator.conflict.mine.body", &[teammate_name]),
                    )
                } else {
                    (
                        bundle.format(
                            "narrator.conflict.other.title",
                            &[player_name, teammate_name],
                        ),
                        bundle.format(
                            "narrator.conflict.other.body",
                            &[player_name, teammate_name],
                        ),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: bundle.format("narrator.conflict.aside", &[&format!("{severity:.0}")]),
                })
            }
            WorldEvent::DecisionMade { .. } => {
                // 决策镜像本身叙事价值低 → 跳过（UI 已有决策面板）
                None
            }
            WorldEvent::TrainingDone {
                player_name, focus, ..
            } => {
                let mine = pro == Some(player_name.as_str());
                // 叙事专用短标签（散文句式用；`TrainingFocus::label()` 的完整标签如
                // "道具训练"与"完成{label}训练计划"的句子结构重复）。
                // 未知 focus **不再静默冒充"枪法"**——文案缺键回退键名，且键名
                // 就是训练枚举名，避免新增训练类型时叙事撒谎。
                let label_key = match focus.as_str() {
                    "AIM" => "narrator.training.label.AIM",
                    "UTILITY" => "narrator.training.label.UTILITY",
                    "CLUTCH" => "narrator.training.label.CLUTCH",
                    "PHYSICAL" => "narrator.training.label.PHYSICAL",
                    "MENTAL" => "narrator.training.label.MENTAL",
                    "COMMUNICATION" => "narrator.training.label.COMMUNICATION",
                    "REST" => "narrator.training.label.REST",
                    other => other,
                };
                let (title, body) = if mine {
                    (
                        bundle.format("narrator.training.mine.title", &[bundle.get(label_key)]),
                        bundle.format("narrator.training.mine.body", &[bundle.get(label_key)]),
                    )
                } else {
                    (
                        bundle.format(
                            "narrator.training.other.title",
                            &[player_name, bundle.get(label_key)],
                        ),
                        bundle.format(
                            "narrator.training.other.body",
                            &[player_name, bundle.get(label_key)],
                        ),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: bundle.get("narrator.training.aside").to_string(),
                })
            }
            WorldEvent::HonourAwarded {
                player_name,
                honour,
                ..
            } => {
                let mine = pro == Some(player_name.as_str());
                let (title, body) = if mine {
                    (
                        bundle.get("narrator.honour.mine.title").to_string(),
                        bundle.format("narrator.honour.mine.body", &[honour]),
                    )
                } else {
                    (
                        bundle.format("narrator.honour.other.title", &[player_name, honour]),
                        bundle.format("narrator.honour.other.body", &[honour, player_name]),
                    )
                };
                Some(Narration {
                    title,
                    body,
                    aside: String::new(),
                })
            }
            WorldEvent::LiveUpdate {
                headline, detail, ..
            } => Some(Narration {
                title: headline.clone(),
                body: detail.clone(),
                aside: String::new(),
            }),
        }
    }

    /// 便捷：配套 `NarratorView` 从引用构造。
    pub fn view<'a>(protagonist: Option<&'a str>, team: Option<&'a str>) -> NarratorView<'a> {
        NarratorView { protagonist, team }
    }
}

/// 从「队名|选手,选手」签名里剥离队名（签名首段）。
fn sig_team(sig: &str) -> &str {
    sig.split('|').next().unwrap_or(sig)
}

/// 剥离可能的签名尾巴（防御：若已传纯队名则原样）。
fn strip(s: &str) -> String {
    s.split('|').next().unwrap_or(s).to_string()
}

/// 赛事等级文案键（文案本体在 `assets/text/zh-CN.json` 的 `narrator.tier.*`）。
fn tier_name(t: csc_domain::tourney_tier::TourneyTier) -> &'static str {
    use csc_domain::tourney_tier::TourneyTier as T;
    match t {
        T::Major => "narrator.tier.MAJOR",
        T::SuperElite => "narrator.tier.SUPERELITE",
        T::Elite => "narrator.tier.ELITE",
        T::T1 => "narrator.tier.T1",
        T::T2 => "narrator.tier.T2",
        T::Qualify => "narrator.tier.QUALIFY",
    }
}

/// 金额中文简写（万）。
fn money_text(v: i64) -> String {
    if v >= 10_000 {
        format!("{}万", v / 10_000)
    } else {
        v.to_string()
    }
}

/// 翻转「胜者:败者」比分为「败者:胜者」（败者视角叙事用；多图逗号分隔各翻转）。
fn flip_score(score: &str) -> String {
    score
        .split(',')
        .map(|seg| {
            let mut it = seg.split(':');
            match (it.next(), it.next()) {
                (Some(a), Some(b)) => format!("{b}:{a}"),
                _ => seg.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 赛场瞬间（确定性叙事调味）：由比分的悬殊度派生一句话「赛场视角」评注。
/// 同输入同输出（纯函数），不消费随机——叙事层不破坏可复现性。
fn match_moment_key(score: &str) -> &'static str {
    // score 形如 "13:5,13:9"（多图），取首图比分判悬殊度
    let first = score.split(',').next().unwrap_or(score);
    let mut nums = first
        .split(':')
        .filter_map(|s| s.trim().parse::<i32>().ok());
    let (Some(a), Some(b)) = (nums.next(), nums.next()) else {
        return "";
    };
    let diff = (a - b).abs();
    // 加时（≥13 且差 1~2）→ 焦灼
    if a >= 13 && b >= 13 && diff <= 2 {
        "narrator.match.moment.ot"
    } else if diff >= 7 {
        "narrator.match.moment.blowout"
    } else if diff >= 4 {
        "narrator.match.moment.early"
    } else if diff <= 1 {
        "narrator.match.moment.tight"
    } else {
        "narrator.match.moment.even"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tourney_tier::TourneyTier;
    use csc_util::id::{PlayerId, TeamId};

    fn match_played(winner: &str, loser: &str) -> WorldEvent {
        WorldEvent::MatchPlayed {
            date: "2026-06-08".into(),
            seq: 0,
            event_name: "IEM Katowice".into(),
            tier: TourneyTier::T1,
            winner: winner.into(),
            winner_id: TeamId(1),
            loser: loser.into(),
            loser_id: TeamId(2),
            score: "13:9".into(),
            maps: 3,
        }
    }

    #[test]
    fn protagonist_perspective_uses_second_person() {
        let view = Narrator::view(Some("MyPlayer"), Some("Vitality"));
        let n = Narrator::render(
            &view,
            &match_played("Vitality|a,b,c,d,e", "FaZe|f,g,h,i,j"),
            &TextBundle::default(),
        )
        .unwrap();
        assert!(
            n.title.contains("你击败了"),
            "主角视角用第二人称: {}",
            n.title
        );
        assert!(n.body.contains("IEM Katowice"));
    }

    #[test]
    fn neutral_perspective_uses_third_person() {
        let view = Narrator::view(None, None);
        let n = Narrator::render(
            &view,
            &match_played("Vitality|a", "FaZe|b"),
            &TextBundle::default(),
        )
        .unwrap();
        assert!(!n.title.contains('你'), "无主角时全程第三人称: {}", n.title);
    }

    #[test]
    fn decision_mirror_has_no_narration() {
        let e = WorldEvent::DecisionMade {
            date: "d".into(),
            seq: 0,
            player_name: "P".into(),
            player_id: PlayerId(1),
            point_id: "p".into(),
            option_id: "AIM".into(),
        };
        assert!(
            Narrator::render(&Narrator::view(None, None), &e, &TextBundle::default()).is_none()
        );
    }

    #[test]
    fn retirement_emotional_tone() {
        let e = WorldEvent::Retirement {
            date: "d".into(),
            seq: 0,
            player_name: "MyPlayer".into(),
            player_id: PlayerId(1),
            age: 35,
            from_team: Some("Vitality".into()),
            from_team_id: Some(TeamId(1)),
        };
        let n = Narrator::render(
            &Narrator::view(Some("MyPlayer"), None),
            &e,
            &TextBundle::default(),
        )
        .unwrap();
        assert!(
            n.title.contains("挂起了鼠标"),
            "退役有专属措辞: {}",
            n.title
        );
    }

    #[test]
    fn tournament_cancelled_has_explanatory_narration() {
        // ROUND11 可观察契约：取消赛事必须渲染出解释性叙述（玩家可见原因）。
        let e = WorldEvent::TournamentCancelled {
            date: "2026-06-20".into(),
            seq: 0,
            event_name: "T3 Fallback Cup".into(),
            tier: TourneyTier::Qualify,
            reason: "参赛不足 / 未凑满 8 队".into(),
        };
        let n = Narrator::render(&Narrator::view(None, None), &e, &TextBundle::default())
            .expect("取消事件应有叙述");
        assert!(
            n.title.contains("T3 Fallback Cup") && n.title.contains("取消"),
            "取消标题应含赛事名与取消语义: {}",
            n.title
        );
        assert!(n.body.contains("参赛不足"), "取消正文应含原因: {}", n.body);
    }

    #[test]
    fn loser_perspective_flips_score() {
        // score 为胜者在前（13:9）；败者视角叙事应翻转为 9:13
        let view = Narrator::view(Some("MyPlayer"), Some("Vitality"));
        let e = match_played("FaZe|a,b,c,d,e", "Vitality|f,g,h,i,j");
        let n = Narrator::render(&view, &e, &TextBundle::default()).unwrap();
        assert!(n.title.contains("你输给了"));
        assert!(n.body.contains("9:13"), "败者视角比分应翻转: {}", n.body);
    }
}
