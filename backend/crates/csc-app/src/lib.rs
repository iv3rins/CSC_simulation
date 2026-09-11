//! # csc-app —— 交互式 CLI（M10）
//!
//! 玩家层闭环的最小实现（UX 设计稿到位前的本地游玩入口）：
//! 决策点渲染 → stdin 决策源 → 推进 → 事件流复盘。
//!
//! ```text
//! csc-app <assets_dir> [--seed 42] [--name Player] [--auto]
//! ```
//!
//! 交互命令：
//! ```text
//! <回车>      推进 1 个月（决策点逐个渲染，输入选项序号，空格分隔；空 = 全默认）
//! auto N      连续自动推进 N 个月（AutoDecisionSource，旁观模式）
//! save <file> 存档（GameState JSON）
//! load <file> 读档
//! summary     世界概览
//! journal N   最近 N 条事件
//! quit        退出
//! ```

pub mod render;

use std::io::Write;

use csc_decision::point::{DecisionPoint, PlayerDecision};
use csc_decision::source::DecisionSource;

/// stdin 决策源：决策点渲染到 stdout，从 stdin 读取选项序号。
///
/// 输入格式：每个决策点一个选项序号（0 起），空格分隔；缺省/空行 = 该点全默认
/// （选项 0 / 类型默认，与 `AutoDecisionSource` 对齐）。
#[derive(Default)]
pub struct StdinDecisionSource {
    /// 渲染文案包（默认 = 内嵌 zh-CN；可由调用方注入覆盖）
    pub text: csc_text::TextBundle,
}

impl DecisionSource for StdinDecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision> {
        let bundle = &self.text;
        let mut stdout = std::io::stdout();
        let stdin = std::io::stdin();
        let mut out = Vec::with_capacity(points.len());

        for (i, point) in points.iter().enumerate() {
            let options = render::option_labels(point, bundle);
            writeln!(
                stdout,
                "\n=== 决策点 {}/{}：{}（{}）===",
                i + 1,
                points.len(),
                render::kind_label(point, bundle),
                point.id()
            )
            .expect("stdout");
            for (j, label) in options.iter().enumerate() {
                writeln!(stdout, "  [{j}] {label}").expect("stdout");
            }
            loop {
                write!(
                    stdout,
                    "  选择（0-{}，回车 = {}）> ",
                    options.len() - 1,
                    render::default_label(point, bundle)
                )
                .expect("stdout");
                stdout.flush().expect("flush");
                let mut line = String::new();
                stdin.read_line(&mut line).expect("stdin");
                let line = line.trim();
                if line.is_empty() {
                    out.push(render::default_decision(point));
                    break;
                }
                match line.parse::<usize>() {
                    Ok(idx) if idx < options.len() => {
                        out.push(render::decision_at(point, idx));
                        break;
                    }
                    _ => writeln!(stdout, "  ❌ 非法序号，重试").expect("stdout"),
                }
            }
        }
        out
    }
}

/// 解析整批选项序号（"1 2 0" → 各点对应选项；测试用）。
pub fn parse_batch(points: &[DecisionPoint], input: &str) -> Result<Vec<PlayerDecision>, String> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if !tokens.is_empty() && tokens.len() != points.len() {
        return Err(format!(
            "选项数 {} ≠ 决策点数 {}",
            tokens.len(),
            points.len()
        ));
    }
    if tokens.is_empty() {
        return Ok(points.iter().map(render::default_decision).collect());
    }
    tokens
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let idx: usize = t
                .parse()
                .map_err(|_| format!("第 {} 项「{t}」不是数字", i + 1))?;
            let n = render::option_labels(&points[i], &csc_text::TextBundle::default()).len();
            if idx >= n {
                return Err(format!("第 {} 项序号 {idx} 越界（0-{}）", i + 1, n - 1));
            }
            Ok(render::decision_at(&points[i], idx))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_decision::point::TrainingOption;
    use csc_simulation::training::TrainingFocus as Focus;

    fn training_points(n: usize) -> Vec<DecisionPoint> {
        (0..n)
            .map(|i| DecisionPoint::TrainingFocus {
                id: format!("p{i}"),
                date: "2026-01-01".into(),
                player_id: csc_util::id::PlayerId(1),
                player_name: "P".into(),
                options: vec![
                    TrainingOption {
                        focus: Focus::Aim,
                        label: "瞄准".into(),
                        description: "x".into(),
                    },
                    TrainingOption {
                        focus: Focus::Clutch,
                        label: "残局".into(),
                        description: "x".into(),
                    },
                ],
            })
            .collect()
    }

    #[test]
    fn empty_input_defaults_all() {
        let points = training_points(2);
        let ds = parse_batch(&points, "").unwrap();
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].option_id, "AIM", "空输入 = 选项 0");
    }

    #[test]
    fn explicit_indexes() {
        let points = training_points(2);
        let ds = parse_batch(&points, "1 0").unwrap();
        assert_eq!(ds[0].option_id, "CLUTCH");
        assert_eq!(ds[1].option_id, "AIM");
    }

    #[test]
    fn count_mismatch_and_oob_rejected() {
        let points = training_points(2);
        assert!(parse_batch(&points, "1").is_err(), "数量不符");
        assert!(parse_batch(&points, "0 9").is_err(), "越界");
        assert!(parse_batch(&points, "0 x").is_err(), "非数字");
    }

    #[test]
    fn option_labels_present() {
        let points = training_points(1);
        assert_eq!(
            render::option_labels(&points[0], &csc_text::TextBundle::default()).len(),
            2
        );
    }
}
