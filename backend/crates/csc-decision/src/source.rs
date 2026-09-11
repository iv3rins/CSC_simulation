//! 决策源（Kotlin `DecisionSource.kt` 转写）——玩家意图层抽象（服务器协议原型）。

use crate::point::{DecisionPoint, PlayerDecision};

/// 决策源——引擎产出 [`DecisionPoint`] 列表 → `decide` 返回决策列表（按 pointId 对应）。
///
/// 实现方：真实玩家（UI/服务端注入）、AI、测试替身、全自动默认。
/// 本接口是"玩家层"与"模拟层"的**唯一耦合点**：双方只交换纯值。
pub trait DecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision>;
}

/// 全自动默认决策源：保持历史全自动行为（= Kotlin `AutoDecisionSource`）。
///
/// 比赛干预选第 0 项（玩家当前打法风格基线）；转会选最优候选；伤病带伤也打；
/// 代言接受——等价旧版硬编码选择。
#[derive(Debug, Clone, Copy, Default)]
pub struct AutoDecisionSource;

impl AutoDecisionSource {
    /// 单个决策点的权威默认决策（**唯一实现**）。
    ///
    /// 所有"默认决策"消费方（auto 引擎、CLI 空输入、集成测试、前端 TS 的
    /// 语义镜像）都必须与本实现保持一致——历史上 csc-app / 集成测试各自
    /// 复制过一份并发生漂移（CLI 曾无条件跳槽最强队，破坏可复现性契约），
    /// 因此本函数是 Rust 侧的唯一事实源。
    pub fn default_decision_for(point: &DecisionPoint) -> PlayerDecision {
        match point {
            DecisionPoint::TransferWindow { id, offers, .. } => {
                // 只跳槽到**明显更强**的队伍（2026 试玩修复：原"只要有候选就签
                // 最优队"导致 auto 模式每 4 个月合同到期无脑跳槽最强队——"随意
                // 变更"。现候选排名 < 当前队排名才转会，否则续约留队保持忠诚）。
                let offer = offers.first();
                let best = offer.and_then(|o| o.candidates.first());
                match best {
                    Some(target)
                        if target.ranking
                            < offer.and_then(|o| o.current_ranking).unwrap_or(i32::MAX) =>
                    {
                        PlayerDecision::new(id, &target.team_signature)
                    }
                    _ => PlayerDecision::new(id, "STAY"),
                }
            }
            DecisionPoint::TrainingFocus { id, options, .. } => {
                PlayerDecision::new(id, options.first().map(|o| o.focus.name()).unwrap_or("AIM"))
            }
            DecisionPoint::MatchIntervention { id, options, .. } => PlayerDecision::new(
                id,
                options.first().map(|o| o.id.as_str()).unwrap_or("DEFAULT"),
            ),
            DecisionPoint::TeammateBlunder { id, .. } => PlayerDecision::new(id, "IGNORE"),
            DecisionPoint::InjuryDecision { id, .. } => {
                // 历史行为：带伤也打（职业态度，但累积 INJURY_PRONE 风险）
                PlayerDecision::new(id, "PLAY_THROUGH")
            }
            DecisionPoint::SponsorshipOffer { id, .. } => PlayerDecision::new(id, "ACCEPT"),
            DecisionPoint::LifeEvent {
                id, kind, options, ..
            } => {
                let safe = match kind {
                    crate::point::LifeEventKind::MatchFixerContact => "REFUSE",
                    crate::point::LifeEventKind::TeammatePowerStruggle => "MEDIATE",
                    // 队友邀请（双排/直播/团建）：auto/快进不静默承接（B-4）——
                    // 旧行为取 options.first() = "JOIN"，快进时把社交邀约自动当成已接受，
                    // 破坏「快进不替玩家做有承诺的选择」语义。统一 DECLINE（不承诺）。
                    crate::point::LifeEventKind::TeammateInvite => "DECLINE",
                    _ => options.first().map(|o| o.id.as_str()).unwrap_or("DECLINE"),
                };
                PlayerDecision::new(id, safe)
            }
        }
    }
}

impl DecisionSource for AutoDecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> {
        points
            .iter()
            .map(AutoDecisionSource::default_decision_for)
            .collect()
    }
}

/// 赛季模拟决策源（FIFA 生涯模式）：赛季推进期间**不向玩家弹事件决策**，
/// 全部采用「职业球员会做的稳妥选择」：
/// - 转会：STAY——换队由玩家在赛季间经主动转会界面决定；
/// - 伤病：REST——不再出现自动模式带伤硬打的夸张表现；
/// - 人生/代言事件：保守默认；
/// - 场内干预：走默认基线（玩家赛前 BP 若已设置，由 conductor 作为基线生效）。
#[derive(Debug, Clone, Copy, Default)]
pub struct CareerDecisionSource;

impl DecisionSource for CareerDecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> {
        points
            .iter()
            .map(|point| match point {
                DecisionPoint::TransferWindow { id, .. } => PlayerDecision::new(id, "STAY"),
                DecisionPoint::InjuryDecision { id, .. } => PlayerDecision::new(id, "REST"),
                _ => AutoDecisionSource::default_decision_for(point),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point::TrainingOption;
    use csc_simulation::training::TrainingFocus;

    #[test]
    fn auto_defaults_match_kotlin() {
        let mut src = AutoDecisionSource;
        let points = vec![
            DecisionPoint::TrainingFocus {
                id: "t1".into(),
                date: "d".into(),
                player_id: csc_util::id::PlayerId(1),
                player_name: "p".into(),
                options: vec![TrainingOption {
                    focus: TrainingFocus::Aim,
                    label: "瞄准特训".into(),
                    description: "提升瞄准".into(),
                }],
            },
            DecisionPoint::InjuryDecision {
                id: "i1".into(),
                date: "d".into(),
                player_id: csc_util::id::PlayerId(1),
                player_name: "p".into(),
                injury: csc_entities::injury::Injury {
                    kind: csc_entities::injury::InjuryKind::Wrist,
                    severity: csc_entities::injury::InjurySeverity::Minor,
                    days_left: 1,
                    sustained_date: "d".into(),
                    source: "s".into(),
                    ticked: false,
                },
                options: Vec::new(),
            },
        ];
        let decisions = src.decide(&points);
        assert_eq!(decisions[0].option_id, "AIM");
        assert_eq!(decisions[1].option_id, "PLAY_THROUGH");
    }

    /// B-4：快进/auto 不得静默承接队友邀请（旧行为取 options.first()="JOIN"）。
    #[test]
    fn auto_teammate_invite_declines() {
        use crate::point::LifeOption;
        let mut src = AutoDecisionSource;
        let points = vec![DecisionPoint::LifeEvent {
            id: "li".into(),
            date: "d".into(),
            player_id: csc_util::id::PlayerId(1),
            player_name: "p".into(),
            kind: crate::point::LifeEventKind::TeammateInvite,
            actor: "m1key".into(),
            detail: "邀请双排".into(),
            options: vec![
                LifeOption {
                    id: "JOIN".into(),
                    label: "接受".into(),
                    description: "join".into(),
                },
                LifeOption {
                    id: "LATER".into(),
                    label: "稍后".into(),
                    description: "later".into(),
                },
                LifeOption {
                    id: "DECLINE".into(),
                    label: "拒绝".into(),
                    description: "decline".into(),
                },
            ],
            contact_team_id: None,
        }];
        let decisions = src.decide(&points);
        assert_eq!(decisions[0].option_id, "DECLINE", "快进不静默承接队友邀请");
    }
}
