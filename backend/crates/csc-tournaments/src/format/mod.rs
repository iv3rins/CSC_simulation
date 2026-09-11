//! 赛制引擎（Kotlin `tournaments/format/` 转写）——纯对阵推进，经 MatchRunner 解耦。

pub mod bracket;
pub mod double_elim;
pub mod single_elim;
pub mod swiss;
pub mod swiss_playoff;

/// 测试共享 runner（固定 A 胜的伪造 MatchRunner）——**唯一事实源**。
///
/// 历史上 `double_elim.rs` / `swiss.rs` / `swiss_playoff.rs` / `single_elim.rs`
/// 各自内嵌一份几乎逐字符相同的 `runner()` 辅助函数（risk_diagnose Jaccard 高达 1.00）。
/// 提取后，四个赛制模块的测试共享同一伪造 runner，未来新增/调整赛制只改一处。
#[cfg(test)]
pub(crate) mod tests_common;

pub use bracket::{BracketResult, BracketSeeding, MatchRunner, TeamEntry};
pub use double_elim::{DoubleElimGroup, DoubleElimGroupsBracket};
pub use single_elim::SingleElimPlayoff;
pub use swiss::SwissStage;
pub use swiss_playoff::SwissPlayoffBracket;
