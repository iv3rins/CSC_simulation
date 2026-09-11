//! 游戏模拟 actor（每局一个线程）+ 游戏管理器（多局隔离）。
//!
//! - **模拟线程**：独占 `Engine`，同步推进；命令经 `mpsc` 进入
//!   （Advance/State/Journal/Load/Shutdown），月步完成后广播
//!   `WsEvent::Step`/`WsEvent::Journal`；**完整快照只在批次结束构建**，
//!   Human 月步只更新轻量 ClientState（性能契约见 `csc-core/client`）；
//! - **Human 策略**：`ChannelDecisionSource` 实现 `DecisionSource`——世界级批次
//!   与比赛级图间干预都在 `decide()` 处阻塞等待决策（D3 单一通道），待决策批次
//!   同时写入共享 `pending`（REST 拉取）并广播 `WsEvent::Decisions`（WS 推送）；
//! - **Auto 策略**：`AutoDecisionSource` 直连，无通道开销（全自动世界推演/服务端
//!   权威回放用）。
//!
//! 并发契约：
//! - Engine 只在模拟线程内被触碰（无锁设计）；跨线程共享的只有**纯值快照**
//!   （`Arc<Mutex<GameState>>`）与决策通道；
//! - `advancing` 原子位防并发推进（Human 等待决策期间亦视为推进中）；
//! - `advance_month` 返回 `Result`：决策源返回非法决策（协议错误）→ `Err(SimError)`
pub mod career;
pub mod decision;
pub mod entry;
pub mod manager;
pub mod persist;
pub mod types;

pub use entry::{GameEntry, GameHandle, SaveGuard};
pub use manager::GameManager;
pub use persist::GameLoader;
pub use types::*;
