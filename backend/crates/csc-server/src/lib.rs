//! # csc-server —— 服务端（M9）
//!
//! axum REST/WS + **channel 决策源桥接**：每局游戏一个模拟线程（`GameSim` actor），
//! 多局天然隔离（D1）；同步 `DecisionSource` trait 经 `mpsc` channel 桥接到
//! HTTP/WS 层——世界级批次与比赛级图间干预走**同一通道**（D3）。
//!
//! ```text
//! 模拟线程（同步推进）                     HTTP / WS 层（tokio）
//! ┌──────────────────────┐  decide(points)  ┌─────────────────────────┐
//! │ Engine.advance_month │ ──阻塞等待──────► │ HumanDecisionSource      │
//! │   └─ ChannelSource ──┤◄──decisions──────┤  │ WS 推送 points（决策面板）│
//! │ 更新快照 → 广播 Step │                  │ 玩家 submit → 回传决策   │
//! └──────────────────────┘                  └─────────────────────────┘
//! ```
//!
//! REST 供查询/存档/全自动推进；WS 供真人交互（决策面板 + 增量事件流）。

pub mod game;
pub mod live;
pub mod routes;
pub mod state;

pub use game::{GameManager, PendingBatch, Policy, StepSummary, WsEvent};
pub use routes::router;
pub use state::{AppState, AssetsBundle};
