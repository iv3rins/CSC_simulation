//! 值类型：策略 / 持久化包 / 决策批次 / 步进摘要 / WS 协议消息。

use std::time::Duration;

use csc_core::state::GameState;
use csc_decision::point::{DecisionPoint, PlayerDecision};
use serde::{Deserialize, Serialize};

// —— 存档辅助（persist.rs 使用） ——
pub(crate) fn canonical_state_bytes(state: &GameState) -> Result<Vec<u8>, String> {
    csc_util::canonical_json_bytes(state)
}
/// WS/广播事件通道容量（慢消费者丢事件——增量查询 `journal?since=` 兜底）。
pub const WS_CHANNEL_CAP: usize = 256;

/// Human 决策等待超时（断连兜底；5 分钟 = 正常思考宽容，勿与 WS 心跳 30s 挂钩）。
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(300);

/// 决策策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// 全自动（服务端权威推演 / 回放校验 / 压测）
    Auto,
    /// 真人决策（决策点经 WS 推送 / REST 拉取，决策回传后继续）
    Human,
}

impl Policy {
    /// 是否人类交互（需要决策通道）。
    pub fn is_human(self) -> bool {
        matches!(self, Policy::Human)
    }
}

/// 磁盘持久化包：policy + 完整 GameState（gzip）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedGame {
    pub format: String,
    pub format_version: u32,
    pub policy: Policy,
    /// 是否故事模式（关键选择持续等待）。`serde(default)`：旧包读入补 false
    /// （向后兼容；v2 包此前无此字段）。
    #[serde(default)]
    pub story: bool,
    /// v3: durable scene receipts and transport identity, outside simulation state.
    #[serde(default)]
    pub career_session: super::career::CareerSession,
    /// GameState JSON 的 CRC32，防止 gzip 包内容静默损坏。
    /// v1 包无此字段：`default` 使其可反序列化，恢复端按 `format_version` 分支处理。
    #[serde(default)]
    pub state_crc32: u32,
    pub state: GameState,
}

/// 待决策批次（推送给前端决策面板的载荷）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingBatch {
    /// 批次序号（自增，重连时可识别重复推送）
    pub batch_id: u64,
    /// 批次日期（dateLabel；比赛级干预与月度批次共用格式）
    pub date: Option<String>,
    /// 全部决策点（两级决策流共享同一类型体系）
    pub points: Vec<DecisionPoint>,
}

/// 决策提交信封（22 号审查 R1 修复）——**携带批次身份**跨通道传递。
///
/// 旧实现只把 `Vec<PlayerDecision>` 送进 mpsc 通道：批次号在通道里丢失，
/// 接收端不重新校验，双击/并发重试可把旧选择送进新批次。本信封把
/// `batch_id` 与 `request_id` 一并携带到**状态拥有者**处，由其原子校验 + 消费，
/// 并通过 `reply` 把权威结果回传提交方（不再 fire-and-forget）。
#[derive(Debug)]
pub struct DecisionSubmission {
    /// 请求幂等身份（前端重试携带同一 ID；缺省 = 无幂等语义）。
    pub request_id: Option<String>,
    /// 提交针对的批次号（必须等于当前 pending 批次）。
    pub batch_id: u64,
    /// 提交的决策（逐点对应 pending.points，服务端在状态拥有者处再校验）。
    pub decisions: Vec<PlayerDecision>,
    /// 权威结果回传（单次）。`Ok(())` = 已被该批次消费；`Err` = 批次过期/校验失败。
    pub reply: std::sync::mpsc::Sender<Result<(), String>>,
}

/// 提交载荷指纹（幂等比对用；对 `(point_id, option_id)` 序列做稳定哈希）。
///
/// 同 request_id 同指纹 → 视为重试（返回原确认）；异指纹 → 拒绝（防止
/// 用同一 ID 重放到不同内容）。顺序敏感：决策顺序是提交语义的一部分。
pub fn submission_fingerprint(decisions: &[PlayerDecision]) -> u64 {
    let mut buf = String::new();
    for d in decisions {
        buf.push_str(&d.point_id);
        buf.push('\u{1f}');
        buf.push_str(&d.option_id);
        buf.push('\u{1e}');
    }
    csc_util::fnv1a64(buf.as_bytes())
}

/// request_id 幂等记录（有界缓存条目）。
#[derive(Debug, Clone)]
pub struct ReceiptEntry {
    /// 提交载荷指纹（同 ID 同指纹 = 重试；异指纹 = 拒绝）。
    pub fingerprint: u64,
    /// 被接受的批次号（诊断/对账用）。
    pub batch_id: u64,
}

/// request_id 幂等缓存上限（有界；超限淘汰最旧）。
pub const RECEIPT_CACHE_CAP: usize = 64;

/// request_id 幂等缓存（有界 FIFO）。
///
/// 语义（22 R1）：同 request_id + 同载荷指纹 = 重试 → 返回原确认（不重复应用）；
/// 同 request_id + 异载荷 = 拒绝。缓存只在「提交被接受」时写入；过期淘汰后
/// 同一 ID 视为全新请求（此时它多半已因 batch_id 不匹配被 409 拦截）。
#[derive(Debug, Default)]
pub struct ReceiptCache {
    entries: std::collections::HashMap<String, ReceiptEntry>,
    order: std::collections::VecDeque<String>,
}

impl ReceiptCache {
    /// 查询一个 request_id 的历史指纹（None = 未见过的 ID）。
    pub fn get(&self, request_id: &str) -> Option<&ReceiptEntry> {
        self.entries.get(request_id)
    }

    /// 记录一次被接受的提交（同 ID 覆盖旧值并刷新顺序）。
    pub fn insert(&mut self, request_id: String, fingerprint: u64, batch_id: u64) {
        if self
            .entries
            .insert(
                request_id.clone(),
                ReceiptEntry {
                    fingerprint,
                    batch_id,
                },
            )
            .is_none()
        {
            self.order.push_back(request_id);
        }
        while self.order.len() > RECEIPT_CACHE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            }
        }
    }
}

/// 单个月步摘要（推进进度事件）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSummary {
    pub game_id: u64,
    /// 推进后月计数
    pub month: i32,
    /// 推进后模拟日期
    pub date: String,
    /// 日级推进时为当月日号；月步为 None
    #[serde(default)]
    pub day: Option<u32>,
    /// 本步新增事件数
    pub journal_added: usize,
    /// 本步新增决策数
    pub decisions_made: usize,
    /// 本步是否为赛季末（month % 12 == 0）：前端据此暂停自动推进并弹赛季总结
    pub season_end: bool,
    /// 第几个赛季（1 起；month/12 向上取整）
    pub season_index: i32,
}

/// 服务端 → 客户端 WS 事件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    /// 新决策批次到达（决策面板渲染）
    Decisions(PendingBatch),
    /// 一个月步完成
    Step(StepSummary),
    /// 事件流增量（复盘/通知）；`next_seq` = 下一条事件序号（对账游标，
    /// 与 REST `/journal?since=` 的 `next_seq` 同口径，M2 WS 侧半）。
    Journal {
        events: Vec<serde_json::Value>,
        next_seq: i32,
    },
    /// 系统即时提示（P2-2：决策等待超时自动继续等）。前端可选 toast 展示；
    /// 忽略不破坏协议（switch default 静默）。journal 侧由
    /// `WorldEvent::LiveUpdate` 承载（确定性可复现，同种子同决策日志下超时行为一致）。
    Notice { message: String },
}

/// 客户端 → 服务端 WS 消息。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsIn {
    /// 提交决策（对应某批 Decisions 事件；batch_id 缺省/不匹配 → 409）
    Decide {
        decisions: Vec<PlayerDecision>,
        #[serde(default)]
        batch_id: Option<u64>,
        /// 幂等身份（22 R1：与 REST 同一语义；同 ID 同载荷返回原确认）。
        #[serde(default)]
        request_id: Option<String>,
    },
    /// 请求推进（Human 策略下挂起等待决策时由 Step 事件回报进度）
    Advance { months: Option<u32> },
}
