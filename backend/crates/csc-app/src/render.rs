//! 决策点渲染（CLI 文本视图）——选项序号与 option_id 的映射单点。
//!
//! 约定：选项序号 0 起；`default_decision` = 空输入时的默认（**直接委托
//! `AutoDecisionSource::default_decision_for`**——权威唯一实现，CLI 不再
//! 自维护副本，避免与核心漂移破坏可复现性）；`decision_at` = 显式序号选择。
//! 服务端/前端渲染应复用同一映射逻辑（当前 CLI 专用，未来可抽出共享 crate）。

use csc_decision::point::{DecisionPoint, PlayerDecision};
use csc_decision::source::AutoDecisionSource;
use csc_text::TextBundle;

/// 决策点类型标签（中文展示；文案来自 `cli.kind.*` 键）。
pub fn kind_label<'a>(p: &DecisionPoint, text: &'a TextBundle) -> &'a str {
    match p {
        DecisionPoint::TransferWindow { .. } => text.get("cli.kind.transfer"),
        DecisionPoint::TrainingFocus { .. } => text.get("cli.kind.training"),
        DecisionPoint::MatchIntervention { .. } => text.get("cli.kind.intervention"),
        DecisionPoint::TeammateBlunder { .. } => text.get("cli.kind.blunder"),
        DecisionPoint::InjuryDecision { .. } => text.get("cli.kind.injury"),
        DecisionPoint::SponsorshipOffer { .. } => text.get("cli.kind.sponsor"),
        DecisionPoint::LifeEvent { .. } => text.get("cli.kind.life"),
    }
}

/// 队友失误反应选项（**id** = ChemistryEngine 常量；展示标签来自 `cli.blunder.*` 键）。
pub const BLUNDER_OPTIONS: [&str; 3] = ["SUPPORT", "CONFRONT", "IGNORE"];

/// 队友失误反应选项标签（文案外置）。
pub fn blunder_label<'a>(id: &str, text: &'a TextBundle) -> &'a str {
    match id {
        "SUPPORT" => text.get("cli.blunder.support"),
        "CONFRONT" => text.get("cli.blunder.confront"),
        _ => text.get("cli.blunder.ignore"),
    }
}

/// 选项标签列表（与 `decision_at` 的序号一一对应；文案来自 `cli.opt.*` 键）。
pub fn option_labels(p: &DecisionPoint, text: &TextBundle) -> Vec<String> {
    match p {
        DecisionPoint::TransferWindow { offers, .. } => {
            let mut v: Vec<String> = offers
                .iter()
                .flat_map(|o| {
                    o.candidates.iter().map(|c| {
                        let team = c
                            .team_signature
                            .split('|')
                            .next()
                            .unwrap_or("?")
                            .to_string();
                        format!(
                            "转会 {team}（VRS #{}，实力 +{:.1}）",
                            c.ranking, c.power_delta
                        )
                    })
                })
                .collect();
            v.push("留队（STAY）".into());
            v
        }
        DecisionPoint::TeammateBlunder {
            teammate_name,
            detail,
            ..
        } => BLUNDER_OPTIONS
            .iter()
            .enumerate()
            .map(|(i, id)| {
                text.format(
                    "cli.opt.blunder",
                    &[
                        blunder_label(id, text),
                        teammate_name,
                        detail,
                        if i == 0 {
                            text.get("cli.opt.recommended")
                        } else {
                            ""
                        },
                    ],
                )
            })
            .collect(),
        DecisionPoint::InjuryDecision {
            options, injury, ..
        } => options
            .iter()
            .map(|o| {
                text.format(
                    "cli.opt.injury",
                    &[
                        &o.label,
                        &o.description,
                        injury.kind.label(),
                        &injury.days_left.to_string(),
                    ],
                )
            })
            .collect(),
        DecisionPoint::SponsorshipOffer {
            options,
            brand,
            annual_value,
            years,
            ..
        } => options
            .iter()
            .map(|o| {
                text.format(
                    "cli.opt.sponsor",
                    &[
                        &o.label,
                        &o.description,
                        brand,
                        &annual_value.to_string(),
                        &years.to_string(),
                    ],
                )
            })
            .collect(),
        DecisionPoint::LifeEvent {
            options,
            actor,
            detail,
            ..
        } => options
            .iter()
            .map(|o| text.format("cli.opt.life", &[&o.label, &o.description, actor, detail]))
            .collect(),
        DecisionPoint::TrainingFocus { options, .. } => options
            .iter()
            .map(|o| text.format("cli.opt.join", &[&o.label, &o.description]))
            .collect(),
        DecisionPoint::MatchIntervention { options, .. } => options
            .iter()
            .map(|o| text.format("cli.opt.join", &[&o.label, &o.description]))
            .collect(),
    }
}

/// 默认决策（空输入）——委托权威实现（`AutoDecisionSource::default_decision_for`）。
pub fn default_decision(p: &DecisionPoint) -> PlayerDecision {
    AutoDecisionSource::default_decision_for(p)
}

/// 按序号决策（0 起；越界由调用方保证）。
pub fn decision_at(p: &DecisionPoint, idx: usize) -> PlayerDecision {
    match p {
        DecisionPoint::TrainingFocus { id, options, .. } => {
            PlayerDecision::new(id, options[idx].focus.name())
        }
        DecisionPoint::TransferWindow { id, offers, .. } => {
            let candidates: Vec<&csc_decision::offer::TransferTarget> =
                offers.iter().flat_map(|o| o.candidates.iter()).collect();
            let opt = candidates
                .get(idx)
                .map(|c| c.team_signature.clone())
                .unwrap_or_else(|| "STAY".into());
            PlayerDecision::new(id, opt)
        }
        DecisionPoint::MatchIntervention { id, options, .. } => {
            PlayerDecision::new(id, options[idx].id.clone())
        }
        DecisionPoint::TeammateBlunder { id, .. } => PlayerDecision::new(id, BLUNDER_OPTIONS[idx]),
        DecisionPoint::InjuryDecision { id, options, .. } => {
            PlayerDecision::new(id, options[idx].id.clone())
        }
        DecisionPoint::SponsorshipOffer { id, options, .. } => {
            PlayerDecision::new(id, options[idx].id.clone())
        }
        DecisionPoint::LifeEvent { id, options, .. } => {
            PlayerDecision::new(id, options[idx].id.clone())
        }
    }
}

/// 默认选项的展示名（空输入提示；文案来自 `cli.default.*` 键）。
pub fn default_label<'a>(p: &DecisionPoint, text: &'a TextBundle) -> &'a str {
    match p {
        DecisionPoint::TransferWindow { .. } => text.get("cli.default.transfer"),
        DecisionPoint::TrainingFocus { .. } => text.get("cli.default.training"),
        DecisionPoint::MatchIntervention { .. } => text.get("cli.default.intervention"),
        DecisionPoint::TeammateBlunder { .. } => text.get("cli.default.blunder"),
        DecisionPoint::InjuryDecision { .. } => text.get("cli.default.injury"),
        DecisionPoint::SponsorshipOffer { .. } => text.get("cli.default.sponsor"),
        DecisionPoint::LifeEvent { .. } => text.get("cli.default.life"),
    }
}
