//! # csc-systems —— M6：世界子系统引擎（五合一 crate）
//!
//! Kotlin 五个同包 sub-engine 的转写（Kotlin 中它们在 `chemistry/`、`economy/`、
//! `injury/`、`population/`、`transfer/` 五个包；按用户决策**系统包合一**，
//! 合并为单个 crate，模块边界保持）：
//!
//! | 模块 | Kotlin 源 | 职责 |
//! |---|---|---|
//! | `chemistry` | ChemistryEngine | 队友失误反应 → 关系/凝聚力/印记；月度关系恢复 |
//! | `economy` | EconomyEngine | 玩家侧财务：奖金分成 + 代言合同（评估/签约/月付） |
//! | `injury` | InjuryEngine | 伤病/体能：月度 tick（恢复/倒计时/掷伤）+ 伤病决策应用 |
//! | `population` | PopulationEngine | 人口再生：退役（≥35 岁）+ 青训新秀补位 |
//! | `transfer` | TransferEngine | 转会窗：候选生成 → 决策执行 → 培养补偿费 |
//!
//! ## 转写核心设计
//!
//! - **World 为唯一状态容器**：Kotlin 各引擎持 `EntityEngine`（名字 map）→ Rust
//!   引擎方法接收 `&mut World`（ID arena），查询按 ID/名字线性扫描；
//! - **无状态引擎 + 参数化依赖**：Rust 引擎结构只持有**跨调用状态**
//!   （如转会事件列表），其余依赖（VRS/时钟/日志/随机源）全部作为方法参数
//!   传入——消除 `&mut` 借用冲突，让 M8 `csc-core` 可任意组合编排；
//! - **公式零持有**：与 Kotlin 一致，全部规则在 `csc-simulation` 纯函数层，
//!   本 crate 只做状态变更与决策管线对接。

pub mod chemistry;
pub mod economy;
pub mod injury;
pub mod life_events;
pub mod npc_market;
pub mod population;
pub mod transfer;

/// 测试共享 fixture（`boost_power` / `vrs_with_teams` 的唯一事实源）。
///
/// 历史上 `npc_market.rs` / `transfer/mod.rs` / `transfer/tests.rs` 各自复制过一份
/// 逐字符相同的 `boost_power` / `vrs_with_teams`（Jaccard=1.00）——一旦一边修改
/// 另一边未同步，不同测试会跳变到不同强度的「拉满选手」，断言不可比。
#[cfg(test)]
pub(crate) mod test_fixtures;
