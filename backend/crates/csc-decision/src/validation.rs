//! 决策提交校验（22 号审查 R2 修复）——**类型化全集校验器**（唯一实现）。
//!
//! 背景：旧流程 `WorldDecisionBatch::run` 先 `DecisionRecorder::record` 再 apply，
//! 且不校验提交集合——空数组/漏项/重复 point/未知 point/候选外 option 都会进入
//! 应用阶段：轻则「记录了成功选择却未完整执行」，重则部分副作用已产生后报错。
//!
//! 本模块提供**唯一**的共享校验器：在「实际消费决策」的状态拥有者处调用，
//! 先完整验证（pending 与提交点集合一一对应、选项确属候选、无重复/未知/漏项），
//! 通过后才允许记录与原子应用。REST、WS、比赛 conductor、CLI/自动源共用同一规则。
//!
//! 校验只做**纯值检查**（不读世界、不消费 RNG），失败无副作用。

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::point::{DecisionPoint, PlayerDecision};

/// 决策提交校验失败（类型化；可直接映射为 422 业务拒绝）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// 提交为空但存在待决策点（漏项：未对任何点作答）。
    EmptyWhenPointsExpected { expected: usize },
    /// 提交中出现本批次不存在的决策点 id（未知 point）。
    UnknownPoint { point_id: String },
    /// 同一决策点被提交多次（重复 point）。
    DuplicatePoint { point_id: String },
    /// 待决策点未在提交中出现（漏项）。
    MissingPoint { point_id: String },
    /// 选项不在该决策点的候选集合内（候选外 option）。
    InvalidOption { point_id: String, option_id: String },
    /// 决策主体无效（如 PlayerId::NONE 哨兵）。
    InvalidSubject { point_id: String },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWhenPointsExpected { expected } => {
                write!(f, "提交的决策为空，但本批次有 {expected} 个待决策点")
            }
            Self::UnknownPoint { point_id } => {
                write!(f, "提交了本批次不存在的决策点「{point_id}」")
            }
            Self::DuplicatePoint { point_id } => {
                write!(f, "决策点「{point_id}」被重复提交")
            }
            Self::MissingPoint { point_id } => {
                write!(f, "待决策点「{point_id}」未在提交中出现")
            }
            Self::InvalidOption {
                point_id,
                option_id,
            } => write!(f, "决策点「{point_id}」的选项「{option_id}」不在候选列表内"),
            Self::InvalidSubject { point_id } => {
                write!(f, "决策点「{point_id}」的决策主体无效")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl ValidationError {
    /// 机器可读错误码（供服务端 422 body 的 `code` 字段）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyWhenPointsExpected { .. } => "EMPTY_SUBMISSION",
            Self::UnknownPoint { .. } => "UNKNOWN_POINT",
            Self::DuplicatePoint { .. } => "DUPLICATE_POINT",
            Self::MissingPoint { .. } => "MISSING_POINT",
            Self::InvalidOption { .. } => "INVALID_OPTION",
            Self::InvalidSubject { .. } => "INVALID_SUBJECT",
        }
    }
}

/// 固定候选集合常量（避免魔法字符串散落）。
mod ids {
    /// 转会窗：留队（续约/待业）。
    pub const STAY: &str = "STAY";
    /// 转会窗：合同到期主动离队赋闲。
    pub const LEAVE: &str = "LEAVE";
    /// 队友失误反应（= `ChemistryModel::REACTION_*`）。
    pub const REACTION_SUPPORT: &str = "SUPPORT";
    pub const REACTION_CONFRONT: &str = "CONFRONT";
    pub const REACTION_IGNORE: &str = "IGNORE";
    /// 伤病决策（= `InjuryEngine::PLAY_THROUGH / REST`）。
    pub const PLAY_THROUGH: &str = "PLAY_THROUGH";
    pub const REST: &str = "REST";
    /// 代言（= `EconomyEngine::ACCEPT / DECLINE`）。
    pub const ACCEPT: &str = "ACCEPT";
    pub const DECLINE: &str = "DECLINE";
}

/// 计算某决策点的**候选选项 id 集合**（唯一口径；与各子系统消费逻辑对齐）。
///
/// 若选项来自决策点自身携带的 `options`（训练/干预/伤病/代言/人生事件），
/// 则以 `options` 为准；`options` 为空（旧档/测试替身）时回退到固定集合。
pub fn candidate_option_ids(point: &DecisionPoint) -> Vec<String> {
    match point {
        DecisionPoint::TransferWindow { offers, .. } => {
            let mut out = vec![ids::STAY.to_string(), ids::LEAVE.to_string()];
            for offer in offers {
                for c in &offer.candidates {
                    out.push(c.team_signature.clone());
                }
            }
            out
        }
        DecisionPoint::TrainingFocus { options, .. } => {
            if options.is_empty() {
                // 旧档/测试替身可能携带空 options；回退到全部训练重点名
                // （与 `AutoDecisionSource` 的 "AIM" 默认及 `TrainingFocus::name()` 对齐）。
                vec![
                    "AIM".to_string(),
                    "UTILITY".to_string(),
                    "CLUTCH".to_string(),
                    "PHYSICAL".to_string(),
                    "MENTAL".to_string(),
                    "COMMUNICATION".to_string(),
                    "REST".to_string(),
                ]
            } else {
                options.iter().map(|o| o.focus.name().to_string()).collect()
            }
        }
        DecisionPoint::MatchIntervention { options, .. } => {
            options.iter().map(|o| o.id.clone()).collect()
        }
        DecisionPoint::TeammateBlunder { .. } => vec![
            ids::REACTION_SUPPORT.to_string(),
            ids::REACTION_CONFRONT.to_string(),
            ids::REACTION_IGNORE.to_string(),
        ],
        DecisionPoint::InjuryDecision { options, .. } => {
            if options.is_empty() {
                vec![ids::PLAY_THROUGH.to_string(), ids::REST.to_string()]
            } else {
                options.iter().map(|o| o.id.clone()).collect()
            }
        }
        DecisionPoint::SponsorshipOffer { options, .. } => {
            if options.is_empty() {
                vec![ids::ACCEPT.to_string(), ids::DECLINE.to_string()]
            } else {
                options.iter().map(|o| o.id.clone()).collect()
            }
        }
        DecisionPoint::LifeEvent { options, .. } => options.iter().map(|o| o.id.clone()).collect(),
    }
}

/// 完整校验一批提交：`pending`（待决策点）与 `decisions`（提交）必须一一对应。
///
/// 规则（22 R2）：
/// - 提交非空要求：`pending` 非空时提交不得为空；
/// - 未知 point：提交中出现 `pending` 没有的 id → 拒绝；
/// - 重复 point：同一 id 提交多次 → 拒绝；
/// - 漏项：`pending` 中某 id 未提交 → 拒绝；
/// - 候选外 option：选项不在该点候选集合内 → 拒绝；
/// - 非法主体：决策主体为 `PlayerId::NONE` 哨兵 → 拒绝。
///
/// 通过即返回 `Ok(())`；任何失败**无副作用**（纯值检查，不触碰世界/RNG/journal）。
pub fn validate_submission(
    points: &[DecisionPoint],
    decisions: &[PlayerDecision],
) -> Result<(), ValidationError> {
    if points.is_empty() {
        // 无待决策：只允许空提交（多余提交视为未知 point）。
        if let Some(d) = decisions.first() {
            return Err(ValidationError::UnknownPoint {
                point_id: d.point_id.clone(),
            });
        }
        return Ok(());
    }
    if decisions.is_empty() {
        return Err(ValidationError::EmptyWhenPointsExpected {
            expected: points.len(),
        });
    }

    // 待决策点索引：id → candidate set。
    let mut expected: HashMap<&str, HashSet<String>> = HashMap::with_capacity(points.len());
    for p in points {
        expected.insert(p.id(), candidate_option_ids(p).into_iter().collect());
    }

    // 重复检测 + 未知检测。
    let mut seen: HashSet<&str> = HashSet::with_capacity(decisions.len());
    for d in decisions {
        if !seen.insert(d.point_id.as_str()) {
            return Err(ValidationError::DuplicatePoint {
                point_id: d.point_id.clone(),
            });
        }
        if !expected.contains_key(d.point_id.as_str()) {
            return Err(ValidationError::UnknownPoint {
                point_id: d.point_id.clone(),
            });
        }
    }

    // 逐点校验选项 + 主体；同时检测漏项。
    for p in points {
        let Some(d) = decisions.iter().find(|d| d.point_id == p.id()) else {
            return Err(ValidationError::MissingPoint {
                point_id: p.id().to_string(),
            });
        };
        if p.player_id() == csc_util::id::PlayerId::NONE {
            return Err(ValidationError::InvalidSubject {
                point_id: p.id().to_string(),
            });
        }
        let cands = expected.get(p.id()).expect("已插入");
        if !cands.contains(&d.option_id) {
            return Err(ValidationError::InvalidOption {
                point_id: p.id().to_string(),
                option_id: d.option_id.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offer::{TransferOffer, TransferTarget};
    use crate::point::{InjuryOption, LifeEventKind, LifeOption, TrainingOption};
    use csc_entities::injury::{Injury, InjuryKind, InjurySeverity};
    use csc_simulation::training::TrainingFocus;
    use csc_util::id::{PlayerId, TeamId};

    fn training_point() -> DecisionPoint {
        DecisionPoint::TrainingFocus {
            id: "2026-02-01|train|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: PlayerId(0),
            player_name: "MyPlayer".into(),
            options: vec![
                TrainingOption {
                    focus: TrainingFocus::Aim,
                    label: "瞄准特训".into(),
                    description: "x".into(),
                },
                TrainingOption {
                    focus: TrainingFocus::Utility,
                    label: "道具".into(),
                    description: "y".into(),
                },
            ],
        }
    }

    fn injury_point() -> DecisionPoint {
        DecisionPoint::InjuryDecision {
            id: "2026-02-01|injury|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: PlayerId(0),
            player_name: "MyPlayer".into(),
            injury: Injury {
                kind: InjuryKind::Wrist,
                severity: InjurySeverity::Minor,
                days_left: 3,
                sustained_date: "2026-02-01".into(),
                source: "s".into(),
                ticked: false,
            },
            options: vec![
                InjuryOption {
                    id: "PLAY_THROUGH".into(),
                    label: "带伤上阵".into(),
                    description: "x".into(),
                },
                InjuryOption {
                    id: "REST".into(),
                    label: "休养".into(),
                    description: "y".into(),
                },
            ],
        }
    }

    fn transfer_point() -> DecisionPoint {
        DecisionPoint::TransferWindow {
            id: "2026-02-01|transfer|MyPlayer".into(),
            date: "2026-02-01".into(),
            player_id: PlayerId(0),
            player_name: "MyPlayer".into(),
            offers: vec![TransferOffer {
                point_id: "2026-02-01|transfer|MyPlayer".into(),
                player_id: PlayerId(0),
                player_name: "MyPlayer".into(),
                from_team_id: Some(TeamId(1)),
                from_team_signature: Some("A|a,b,c,d,e".into()),
                candidates: vec![TransferTarget {
                    team_id: TeamId(2),
                    team_signature: "B|p,q,r,s,t".into(),
                    ranking: 1,
                    power_delta: 3.0,
                    salary: 100_000,
                }],
                discounted: false,
                current_ranking: Some(2),
            }],
        }
    }

    #[test]
    fn valid_full_submission_passes() {
        let points = vec![training_point(), injury_point()];
        let decisions = vec![
            PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM"),
            PlayerDecision::new("2026-02-01|injury|MyPlayer", "REST"),
        ];
        assert_eq!(validate_submission(&points, &decisions), Ok(()));
    }

    #[test]
    fn empty_submission_rejected() {
        let points = vec![training_point()];
        let err = validate_submission(&points, &[]).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::EmptyWhenPointsExpected { expected: 1 }
        ));
        assert_eq!(err.code(), "EMPTY_SUBMISSION");
    }

    #[test]
    fn missing_point_rejected() {
        let points = vec![training_point(), injury_point()];
        let decisions = vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM")];
        let err = validate_submission(&points, &decisions).unwrap_err();
        assert!(matches!(err, ValidationError::MissingPoint { .. }));
        assert_eq!(err.code(), "MISSING_POINT");
    }

    #[test]
    fn duplicate_point_rejected() {
        let points = vec![training_point()];
        let decisions = vec![
            PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM"),
            PlayerDecision::new("2026-02-01|train|MyPlayer", "UTILITY"),
        ];
        let err = validate_submission(&points, &decisions).unwrap_err();
        assert!(matches!(err, ValidationError::DuplicatePoint { .. }));
    }

    #[test]
    fn unknown_point_rejected() {
        let points = vec![training_point()];
        let decisions = vec![
            PlayerDecision::new("2026-02-01|train|MyPlayer", "AIM"),
            PlayerDecision::new("ghost|point", "X"),
        ];
        let err = validate_submission(&points, &decisions).unwrap_err();
        assert!(matches!(err, ValidationError::UnknownPoint { .. }));
    }

    #[test]
    fn invalid_option_rejected() {
        let points = vec![training_point()];
        let decisions = vec![PlayerDecision::new("2026-02-01|train|MyPlayer", "NUCLEAR")];
        let err = validate_submission(&points, &decisions).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidOption { .. }));
        assert_eq!(err.code(), "INVALID_OPTION");
    }

    #[test]
    fn transfer_accepts_stay_leave_and_candidates_only() {
        let points = vec![transfer_point()];
        // STAY / LEAVE / 候选签名均可。
        for opt in ["STAY", "LEAVE", "B|p,q,r,s,t"] {
            let decisions = vec![PlayerDecision::new("2026-02-01|transfer|MyPlayer", opt)];
            assert_eq!(
                validate_submission(&points, &decisions),
                Ok(()),
                "opt={opt}"
            );
        }
        // 候选外签名 → 拒绝。
        let bad = vec![PlayerDecision::new(
            "2026-02-01|transfer|MyPlayer",
            "C|z,z,z,z,z",
        )];
        assert!(matches!(
            validate_submission(&points, &bad).unwrap_err(),
            ValidationError::InvalidOption { .. }
        ));
    }

    #[test]
    fn teammate_blunder_fixed_reactions() {
        use csc_simulation::directives::BlunderKind;
        let p = DecisionPoint::TeammateBlunder {
            id: "2026-02-01|blunder|EV|1|9".into(),
            date: "2026-02-01".into(),
            player_id: PlayerId(0),
            player_name: "MyPlayer".into(),
            event_name: "EV".into(),
            map_number: 1,
            teammate_name: "tm".into(),
            teammate_id: PlayerId(9),
            teammate_role: csc_entities::role::Role::Rifler,
            blunder: BlunderKind::Tilt,
            detail: "d".into(),
        };
        for opt in ["SUPPORT", "CONFRONT", "IGNORE"] {
            let d = vec![PlayerDecision::new("2026-02-01|blunder|EV|1|9", opt)];
            assert_eq!(validate_submission(std::slice::from_ref(&p), &d), Ok(()));
        }
        let bad = vec![PlayerDecision::new(
            "2026-02-01|blunder|EV|1|9",
            "IGNORE_ALL",
        )];
        assert!(validate_submission(std::slice::from_ref(&p), &bad).is_err());
    }

    #[test]
    fn none_subject_rejected() {
        let p = DecisionPoint::LifeEvent {
            id: "2026-02-01|life|0|INTERVIEW|A:media".into(),
            date: "2026-02-01".into(),
            player_id: PlayerId::NONE,
            player_name: "?".into(),
            kind: LifeEventKind::Interview,
            actor: "media".into(),
            detail: "d".into(),
            options: vec![LifeOption {
                id: "SPOTLIGHT".into(),
                label: "l".into(),
                description: "d".into(),
            }],
            contact_team_id: None,
        };
        let d = vec![PlayerDecision::new(
            "2026-02-01|life|0|INTERVIEW|A:media",
            "SPOTLIGHT",
        )];
        let err = validate_submission(std::slice::from_ref(&p), &d).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidSubject { .. }));
    }
}
