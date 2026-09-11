//! 模拟错误类型（D6 错误处理分层）——**外部输入**的统一错误契约。
//!
//! 分层约定：
//! - 内部不变量（编程错误）：`assert!`/`panic!`（等价 Kotlin `require`），不可恢复；
//! - 外部输入（资产文件 / 存档 / 协议决策）：`Result<T, SimError>`，可恢复、
//!   可展示、可跨服务端协议传播——**严禁对外部输入 panic**（坏资产不应打崩服务）。
//!
//! 依赖：零第三方（serde 之外），被 csc-vrs/csc-entities/csc-core 消费。

use std::fmt;

/// 模拟错误——外部输入的失败（资产 / 存档 / 协议）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimError {
    /// 外部资产（standings / roles_baseline 等）解析或语义校验失败。
    Asset {
        /// 资产文件名（未知时为 "<memory>"）
        file: String,
        /// 失败原因（人话，可直接展示给调用方）
        reason: String,
    },
    /// 存档解析 / 版本校验 / 未知字段拒绝。
    Save {
        /// 失败原因（人话）
        reason: String,
    },
    /// 协议层非法输入（候选外转会目标、未知决策点等）。
    Protocol {
        /// 失败原因（人话）
        reason: String,
    },
}

impl SimError {
    /// 资产错误便捷构造。
    pub fn asset(file: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Asset {
            file: file.into(),
            reason: reason.into(),
        }
    }

    /// 存档错误便捷构造。
    pub fn save(reason: impl Into<String>) -> Self {
        Self::Save {
            reason: reason.into(),
        }
    }

    /// 协议错误便捷构造。
    pub fn protocol(reason: impl Into<String>) -> Self {
        Self::Protocol {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset { file, reason } => write!(f, "资产「{file}」校验失败：{reason}"),
            Self::Save { reason } => write!(f, "存档失败：{reason}"),
            Self::Protocol { reason } => write!(f, "协议错误：{reason}"),
        }
    }
}

impl std::error::Error for SimError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_equality() {
        let e = SimError::asset("standings_2026.json", "负积分");
        assert_eq!(e, SimError::asset("standings_2026.json", "负积分"));
        assert_eq!(e.to_string(), "资产「standings_2026.json」校验失败：负积分");
        let s = SimError::save("版本不兼容");
        assert_eq!(s.to_string(), "存档失败：版本不兼容");
        let p = SimError::protocol("候选外目标");
        assert_eq!(p.to_string(), "协议错误：候选外目标");
    }

    #[test]
    fn implements_std_error() {
        fn needs_error<E: std::error::Error>(_: &E) {}
        needs_error(&SimError::save("x"));
    }
}
