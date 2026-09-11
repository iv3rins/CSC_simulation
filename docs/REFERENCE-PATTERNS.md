# 工业级参考源码转写蓝本（REFERENCE-PATTERNS）

> 用途：CSC 修复工程（P0/P1 缺陷）的**代码转写蓝本**。每个方向给出 2-3 个具体开源项目（GitHub URL + 关键文件路径 + 核心代码模式摘录），提炼可直接转写的签名/结构，并标注与 CSC 现有代码的映射关系。
> 产出：pro-review-2（t2，工业级参考调研）
> 依据：`docs/REVIEW-COMPARATIVE-FINAL.md` §3.2 最优先 5 件事（P0-① 指纹门禁、G1 决策通道三件套、G2 溢出、S-H2+M2 决策超时与 WS 心跳、M4/M5/M1 确定性小洞）
> 取证方式：raw.githubusercontent.com / GitHub API 抓取 master/main 分支实际源码（非 README 转述），关键行号已注明。本文所有"摘录"均为真实源码节选。
> 版本基线：HEAD `fe9dc77` 工作树。

---

## 0. 总览：四方向 → CSC 映射速查

| 方向 | 工业级参照 | 对应 CSC 修复项（IMPLEMENTATION-ORDERS） |
|---|---|---|
| 1. 服务端安全/并发（限流 + 防重放 + 超时） | OpenTTD（命令队列与防滥用）、open-football（worker 协议 + 批超时）、tower-governor/axum（限流中间件） | A-1（G1 扩修三件套）、A-4（决策超时）、A-3（WS Advance clamp） |
| 2. 确定性模拟指纹（状态哈希 + 同 seed 双跑 + 版本化） | bevy_ggrs（ChecksumPart→XOR 聚合）、ggrs（SyncTestSession 双跑比对）、ruff（CacheKey 确定性哈希 trait）、open-football（HydrationRng / PROTOCOL_VERSION） | B-1（G3+S3 指纹门禁） |
| 3. 前端 WS 心跳（ping/pong + 重连 + seq 游标） | engine.io-protocol（服务端主动 ping 权威协议）、reconnecting-websocket（重连库）、axum websockets 示例（服务端半） | C-3（WS 心跳）、A-4（超时兜底联动） |
| 4. 存档版本化（JSON schema 校验 + 迁移） | schemars（schema 生成 + serde 属性对齐）、serde 生态（deny_unknown_fields/default 迁移）、firecracker（SNAPSHOT_VERSION semver 常量） | D-1（SAVE-FORMAT.md）、B-1 的 sim_version 字段、A-2 行为变更记录 |

---

## 1. 服务端安全/并发：请求限流 + 命令/决策提交防重放 + 超时

### 1.1 OpenTTD — 服务端命令队列的防滥用闸门（20+ 年确定性工程的输入面防线）

- **仓库**：https://github.com/OpenTTD/OpenTTD（8.2k★，GPLv2，C++20）
- **关键文件**：
  - `src/network/network_server.cpp`（命令入口校验 + 队列上限）
  - `src/command.cpp`（test-run/execute 两段式命令执行）
  - `docs/desync.md`（确定性同步理论 + desync 检测/回放定位）
- **核心模式摘录**（`network_server.cpp:1108-1192`，`ReceiveClientCommand`）：

```cpp
NetworkRecvStatus ServerNetworkGameSocketHandler::ReceiveClientCommand(Packet &p)
{
    // 状态机前置校验：未完成入局/已退出 → 直接断错误响应
    if (this->status < ClientStatus::DoneMap || this->HasClientQuit())
        return this->SendError(NetworkErrorCode::NotExpected);

    // ① 每客户端命令队列上限（防单客户端灌爆队列）
    if (this->incoming_queue.size() >= _settings_client.network.max_commands_in_queue)
        return this->SendError(NetworkErrorCode::TooManyCommands);

    CommandPacket cp;
    auto err = this->ReceiveCommand(p, cp);
    ...
    // ② 逐项身份/权限校验（每条命令都要验），失败即踢
    if (GetCommandFlags(cp.cmd).Test(CommandFlag::Server) && ci->client_id != ClientID::Server)
        return this->SendError(NetworkErrorCode::Kicked);       // 越权：服务器专属命令
    if (!GetCommandFlags(cp.cmd).Test(CommandFlag::Spectator) && !Company::IsValidID(cp.company) ...)
        return this->SendError(NetworkErrorCode::Kicked);       // 无效主体
    if (... ci->client_playas != cp.company)
        return this->SendError(NetworkErrorCode::CompanyMismatch); // 冒用他人公司

    // ③ 通过全部校验才入队（服务端单点权威，客户端只发意图）
    this->incoming_queue.push_back(std::move(cp));
    return NetworkRecvStatus::Okay;
}
```

- **可转写模式（对应 A-1）**：
  1. **队列上限前置校验**：在「入 channel 缓冲」之前先查 `sync_channel(16)` 容量语义 → CSC 用 `try_send` 的 `TrySendError::Full` 直接等价实现（比 OpenTTD 的队列计数更原生）。OpenTTD 的教训是**拒绝必须在消费侧阻塞之前发生**，CSC 当前 `tx.send()`（game.rs:759）阻塞在 tokio worker 上就是反例。
  2. **状态机前置校验**：OpenTTD 的 `status < DoneMap → NotExpected` 对应 CSC 的「advancing 前置校验」——未在推进中（无 pending 批次）时提交决策应立即拒绝（409），而不是静默吞进缓冲。
  3. **两段式执行（test-run → execute）**（`command.cpp:162-189` `InternalDoBefore/After`）：test-run 不可改状态、execute 前用 `Backup<CompanyID>` 保存现场、执行后校验 test/exec 结果一致（`command.cpp:363` assert）。CSC 的 `decide()` 单段消费模型不需要完整两段式，但**校验与执行分离**的思想直接对应 A-1 的「batch_id 校验（只读锁 pending）→ try_send（提交）」两段结构。
  4. **防重放/陈旧命令**：OpenTTD 用「网络帧号（network frame）+ 队列时间戳」给每条命令标定执行点——与 CSC 的 `batch_id`（PendingBatch.batch_id，game.rs:152-160）完全同构。陈旧批次（客户端断线重连重发）被「帧号已过」语义拒绝，正是 A-1 要补的 batch_id 校验。

### 1.2 open-football — worker 协议：版本握手 + liveness 探活 + 批级超时围栏

- **仓库**：https://github.com/ZOXEXIVO/open-football（208★，Rust；"Football world simulator"——模拟俱乐部/联赛/比赛/转会/财政的生态模拟引擎）
- **关键文件**：
  - `src/web/src/worker/protocol.rs`（协议信封 + `PROTOCOL_VERSION` + Ping/Pong）
  - `src/web/src/worker/dispatcher.rs`（分派器：超时围栏 + 失败隔离 + 本地兜底）
  - `src/database/src/generators/rng.rs`（HydrationRng：按 ID 派生确定性 RNG，见 §2.3）
- **核心模式摘录 A**（`protocol.rs:11-36`）：

```rust
/// Wire protocol version. Bumped when the on-wire shape changes...
/// v2: added `Request::Ping` / `Response::Pong` liveness probe.
pub const PROTOCOL_VERSION: u32 = 2;

pub enum Request {
    Handshake { coordinator_version: String, protocol_version: u32 },
    PlayBatch { items: Vec<MatchEnvelope> },
    /// Liveness probe sent by the coordinator's health monitor over an
    /// otherwise-idle worker connection... Cheap enough to send every few seconds.
    Ping,
}
pub enum Response {
    Handshake { version: String, protocol_version: u32, threads: usize, ... },
    HandshakeRejected { reason: String },
    PlayBatch { items: Vec<MatchOutcome> },
    Error { reason: String },
    /// Reply to `Request::Ping`. Its mere arrival (in time) is the proof of life.
    Pong,
}
```

- **核心模式摘录 B**（`dispatcher.rs:58-75`）：

```rust
/// Upper bound on a single remote batch round-trip (request write + response read).
/// A stopped or wedged worker that never delivers a FIN/RST would otherwise leave
/// `Frame::read` blocked forever. ... The timeout converts that unbounded hang into
/// an ordinary batch failure the fence/requeue/safety-net path already handles.
const REMOTE_BATCH_TIMEOUT: Duration = Duration::from_secs(60);
```

- **可转写模式（对应 A-4）**：
  1. **超时常量语义**：`REMOTE_BATCH_TIMEOUT` 的注释写得非常清楚——超时不是性能调参，是把「无限阻塞」变成「可处理的普通失败」。CSC 的 `ChannelDecisionSource::decide` 的 `recv()`（game.rs:397）无限阻塞就是同类问题，A-4 的 `recv_timeout(Duration::from_secs(300))` 直接复用此语义（超时 → 默认决策继续推进）。
  2. **协议版本显式化**：`PROTOCOL_VERSION: u32 = 2` 与握手时双方互报版本、不匹配即拒绝——对应 B-1/S3 的 `WORLD_SIM_VERSION`（指纹链版本标签）。教训：**版本号必须是协议/指纹的显式组成项**，CSC 的 `SIMULATION_VERSION` 只在 live_match.rs:31（回放链），世界链没有，正好照此补齐。
  3. **Ping/Pong 作为一等协议消息**：协议枚举里 Ping/Pong 与业务消息同级（不是传输层 hack），"Pong 的准时到达本身就是活性证明"——与 C-3 的应用层心跳设计一致（见 §3）。

### 1.3 tower-governor + axum — 限流中间件的成熟落法

- **仓库**：https://github.com/benwis/tower-governor（governor GCRA 算法的 tower 中间件，支持 axum）；https://github.com/tokio-rs/axum（26.9k★，examples/chat、examples/websockets）
- **关键文件**：tower-governor `README.md`（完整示例）；axum `examples/websockets/src/main.rs`
- **核心模式摘录**（tower-governor README）：

```rust
let governor_conf = GovernorConfigBuilder::default()
    .per_second(2)          // 令牌桶：每 2s 补充 1 个
    .burst_size(5)          // 突发上限 5 个
    .finish().unwrap();
// 后台任务定期清理限流状态存储（防无界增长）
std::thread::spawn(move || loop {
    std::thread::sleep(interval);
    governor_limiter.retain_recent();
});
let app = Router::new()
    .route("/", get(hello))
    .layer(GovernorLayer::new(governor_conf));   // layer 挂载，按 peer IP 键控
```

- **axum websockets 服务端关键点**（`examples/websockets/src/main.rs:101-149`）：① 升级后先发 `Message::Ping` 探活（注释明言"unsupported by some browsers"，可失败）；② `socket.split()` 分离发送/接收，`tokio::select!` 双任务竞速——**CSC 的 ws_loop（routes.rs:1584-1630）已经是 `tokio::select!` + broadcast 结构，只需在分支上加 ping/pong 处理**；③ 任一任务退出即 abort 另一任务（资源释放纪律）。
- **可转写模式（对应 A-3/M7 治理 + C-3 服务端半）**：
  1. 若 CSC 未来加全局限流：`GovernorLayer` 挂 `Router::layer()`，键控用 `PeerIpKeyExtractor`（默认）或自定义 KeyExtractor（按 game_id 限流）。**注意**：CSC 是单用户工具，当前 G1 修复不需要引入 governor；本项留作「若公开部署」时的选项（M7 已指出无鉴权，限流只能缓解不能替代鉴权）。
  2. axum 的 `Message::Ping` 是协议级 ping（浏览器 WS API 会自动回 pong 且 JS 无法监听/发送协议级 ping）——**JS 客户端只能用应用层 ping/pong（文本消息）**，这正是 C-3 采用 `{type:"ping"}`/`{type:"pong"}` JSON 消息的原因。服务端 ws_loop 收到 `Message::Text` 解析出 ping 再回 pong 即可。

---

## 2. 确定性模拟指纹：状态哈希/校验和 + 同 seed 双跑断言 + 版本化

### 2.1 bevy_ggrs — ChecksumPart → XOR 聚合管线（月级增量指纹的直接模板）

- **仓库**：https://github.com/gschup/bevy_ggrs（361★，MIT）
- **关键文件**：`docs/architecture.md`（Checksum Pipeline 一节）；`src/`（ChecksumPlugin、ComponentChecksumPlugin）
- **核心模式摘录**（architecture.md:90-98）：

```
Every registered type contributes a ChecksumPart (a u128 stored as a component
flagged with ChecksumFlag<T>) to the running frame checksum:

1. During SaveWorldSystems::Checksum, each type's plugin computes its hash
   and upserts a ChecksumPart entity.
2. After that set, ChecksumPlugin::update XORs all parts together into the
   Checksum resource.
3. run_ggrs_schedules reads Checksum after SaveWorld and forwards it to GGRS
   via cell.save(frame, None, checksum).

GGRS compares checksums from all peers and fires GgrsEvent::DesyncDetected (P2P)
or SyncTestMismatch (SyncTest) if they diverge.
```

- **快照存储关键结构**（architecture.md:50-54）：`GgrsSnapshots<For, As>` 是 `(frame, snapshot)` 双端队列（newest-first），深度同步 `MaxPredictionWindow`，确认后修剪旧帧——CSC 指纹门禁不需要回滚，但「**每帧/每月一个哈希、按序存储**」就是 B-1 的「月级增量指纹序列」。
- **可转写模式（对应 B-1）**：
  1. **「每类型各自哈希 → 聚合器 XOR/叠加」**：CSC 不按类型分片（GameState 是单一聚合根），但可转写为「**分层指纹**」：`月指纹 = fnv1a64(canonical(state))`，`序列指纹 = 逐月指纹累积哈希`（如 XOR 或链式 hash）。bevy_ggrs 用 XOR 聚合的目的是各类型独立并行计算；CSC 单聚合根用链式更佳（可检测顺序交换）。
  2. **`cell.save(frame, data, checksum)` 签名**（ggrs `src/sync_layer.rs:18`）：`save(&self, frame: Frame, data: Option<T>, checksum: Option<u128>)`——「帧号 + 状态 + 校验和」三元组，正是 B-1 `t1` 要采集的 `(month, fnv1a64(canonical(state)))` 序列的结构。u128 位宽（非 u64）值得借鉴：碰撞风险更低；CSC 用 `fnv1a64` 即可（csc-util 已有测试向量，且门禁是 CI 断言非安全边界），文档注明此取舍。
  3. **Checksum 只在 SaveWorld 阶段算**（不在每 tick）：CSC 对应「只在月步完成时（finish_advance / snapshot 更新点）计算指纹」，不逐日算。

### 2.2 ggrs — SyncTestSession：同 seed 双跑 + 回滚重放比对的标准范式

- **仓库**：https://github.com/gschup/ggrs（678★，MIT）
- **关键文件**：`src/sessions/sync_test_session.rs`（双跑比对核心）；`src/sync_layer.rs`（GameStateCell/保存窗口）
- **核心模式摘录**（`sync_test_session.rs:9-24` 文档注释 + `116-138` 主体）：

```
Every call to advance_frame() performs the following sequence:
1. Save — the current game state is saved.
2. Advance — your game logic runs for one frame with the inputs you provided.
3. Roll back — GGRS rewinds check_distance frames and re-simulates forward,
   requesting a Load followed by check_distance × Advance requests.
4. Compare — the checksums produced during re-simulation are compared against
   those saved during the original run. A mismatch means your game is not
   deterministic.

If a checksum mismatch is detected, advance_frame() panics — this is intentional.
```

```rust
// checksums_consistent: 首见记录，再见比对（checksum_history 窗口内）
let Some(latest_cell) = self.sync_layer.saved_state_by_frame(frame_to_check) else {
    return true;
};
if let Some(&cs) = self.checksum_history.get(&latest_cell.frame()) {
    cs == latest_cell.checksum()          // 同帧同输入 → 校验和必须全等
} else {
    self.checksum_history.insert(latest_cell.frame(), latest_cell.checksum());
    true
}
```

- **可转写模式（对应 B-1 t1/t2）**：
  1. **SyncTest 的哲学可直接照抄**：mismatch 即 panic/assert，非警告——"a mismatch is a bug that must be fixed before shipping"。B-1 门禁测试必须 `assert_eq!` 且不 `#[ignore]`。
  2. **t2（中途存档/恢复续跑全等）就是 SyncTest 的「Load → 重跑 → 比对」去掉网络版**：CSC 已有 `Engine::snapshot/restore`（engine.rs:413-464），B-1 的 `t2_mid_save_restore_continue_equals_uninterrupted` 直接映射。
  3. **`GameStateCell::save(frame, data, checksum)` 的三元组存储**（sync_layer.rs:14-24）是 B-1 指纹序列采集的数据结构模板（`Vec<(month, u64)>`）。
  4. **SyncTest 的一个工程细节**：`check_distance` 窗口内保留 checksum_history（`retain(|&k,_| k >= oldest_allowed_frame)`，sync_test_session.rs:206-208），防止无限增长——B-1 若未来扩展到 12+ 月序列同样需要窗口裁剪。

### 2.3 ruff — CacheKey trait：确定性哈希的正确姿势（HashMap 键序陷阱的答案）

- **仓库**：https://github.com/astral-sh/ruff（Rust linter，工业级 Rust 代码库）
- **关键文件**：`crates/ruff_cache/src/cache_key.rs`（全文件 478 行，值得通读）
- **核心模式摘录**（cache_key.rs:16-85、312-336）：

```rust
/// Why a new trait rather than reusing Hash?
/// * Cache keys must be deterministic where hash keys do not have this constraint.
///   That's why pointers don't implement CacheKey but they implement Hash.
/// * Ideally, cache keys are portable
pub trait CacheKey {
    fn cache_key(&self, state: &mut CacheKeyHasher);
    fn cache_key_slice(data: &[Self], state: &mut CacheKeyHasher) where Self: Sized {
        for piece in data { piece.cache_key(state); }
    }
}

// HashMap：长度前缀 + 按键排序后逐项写入 —— 消除迭代序噪声
impl<K, V, S> CacheKey for HashMap<K, V, S> where K: CacheKey + Ord, V: CacheKey {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_usize(self.len());
        for (key, value) in self.iter().sorted_by(|(l, _), (r, _)| l.cmp(r)) {
            key.cache_key(state);
            value.cache_key(state);
        }
    }
}

// Vec/slice：长度前缀防歧义（["ab","c"] != ["a","bc"]）
impl<T: CacheKey> CacheKey for [T] {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_usize(self.len());
        CacheKey::cache_key_slice(self, state);
    }
}
```

- **可转写模式（对应 B-1 canonical_state_bytes）**：
  1. **长度前缀**：CSC 的 `seed_chain.rs:61-65` `push_str` 已是「len(u64 LE) + bytes」同款（域标签 + 定宽 + 长度前缀），指纹链直接复用此纪律。
  2. **HashMap 键排序**：CSC 的 `canonical_state_bytes`（game.rs:115-136）已用 serde_json 对象键排序解决顶层；ruff 的方案提示更底层的手写 hasher 也可以做（写长度 + 排序）。B-1 提取到 csc-util 时**保留 JSON 键排序方案**即可，但文档需标注：`Vec` 顺序仍是语义（archive HashMap 由 JSON 键排序覆盖，journal/decisions 有序）。
  3. **域标签**：CacheKey 的注释强调指针不实现（地址不确定）；CSC 对应铁律是「指纹只 hash 值语义，绝不 hash 指针/Display 输出」。

### 2.4 open-football HydrationRng + 版本标签（附带项，B-1 的 RNG 参考）

- **关键文件**：`src/database/src/generators/rng.rs`（106 行全文，SplitMix64 + warm-up + Box-Muller）
- **核心模式摘录**（rng.rs:17-23、65-69）：

```rust
/// Deterministic stream for a known identity (e.g. an ODB player id).
pub fn from_seed(seed: u64) -> Self {
    let mut rng = HydrationRng { state: seed };
    // One warm-up draw so trivially small seeds (id 1, 2, …) don't hand
    // their raw value to the first consumer.
    rng.next_u64();
    rng
}
// Box-Muller（注意：u1 只 max 下界，与 CSC 的 M5 议题同款）
pub fn normal(&mut self) -> f32 {
    let u1 = self.f32().max(1e-10);
    let u2 = self.f32();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
}
```

- **映射**：CSC 的 B-3（M5 Box-Muller 收敛）目标口径与 open-football 一致（`max(1e-10)` 只拦下界，因为 `f32()` ∈ [0,1) 永不为 1，`ln(1)=0` 分支不可达——这为「generator 的 clamp 上界 1-1e-12 是多余防御」提供了第二个独立工业佐证）。warm-up draw 是 CSC seed_chain 没有的模式（CSC 用域标签解决），记作备选。

---

## 3. 前端 WS 心跳：应用层 ping/pong + 重连 + seq 游标

### 3.1 engine.io-protocol — 服务端主动 ping 的权威协议（浏览器定时器节流问题的答案）

- **仓库**：https://github.com/socketio/engine.io-protocol（Socket.IO 传输层协议 v4，生产级百万并发验证）
- **关键文件**：`README.md`（Heartbeat 一节，:222-241）
- **核心模式摘录**：

```
### Heartbeat
At a given interval (the pingInterval value sent in the handshake) the server
sends a ping packet and the client has a few seconds (the pingTimeout value)
to send a pong packet back.

If the server does not receive a pong packet back, then it SHOULD consider
that the connection is closed.

Conversely, if the client does not receive a ping packet within
pingInterval + pingTimeout, then it SHOULD consider that the connection is closed.
```

默认参数（README:409-410）：`pingInterval: 300`（ms）、`pingTimeout: 200`。历史节（:371-374）明确记录 v3→v4 的动机："The ping packets are now sent by the server, because the timers set in the browsers are not reliable enough. We suspect that a lot of timeout problems came from timers being delayed on the client-side."——**这正是 C-3 设计「前端 30s 主动 ping」的已知弱点（后台标签页定时器被节流）的工业级答案：服务端主动 ping 更可靠**。CSC 是单用户工具，前端主动 ping 可接受（C-3 指令的最小实现），但本文档记录升级路径：服务端 `tokio::time::interval` 主动 ping + 客户端只应答。
- **可转写模式（对应 C-3）**：
  1. `pingInterval`/`pingTimeout` 两个参数在手握手中协商（服务端下发）——CSC 可硬编码常量（30s/10s），不必协商，但**两个参数缺一不可**（间隔 + 容忍）。
  2. **双向判死**：服务端收不到 pong 判死；客户端 `pingInterval + pingTimeout` 内收不到 ping 判死。CSC 最小实现只需客户端单向判死（10s 内无 pong → close → 触发既有 onclose 指数退避重连），但服务端判死是 A-4 决策超时的前哨（可选增强）。

### 3.2 reconnecting-websocket — 指数退避重连 + 消息缓冲的标准实现

- **仓库**：https://github.com/pladaria/reconnecting-websocket（1.3k★，MIT，TypeScript）
- **关键文件**：`reconnecting-websocket.ts`（513 行单文件）
- **核心模式摘录**（:39-49 默认参数、:309-325 退避计算、:252-263 send 缓冲）：

```ts
const DEFAULT = {
    maxReconnectionDelay: 10000,
    minReconnectionDelay: 1000 + Math.random() * 4000,  // 随机化防惊群
    minUptime: 5000,          // 连接需存活 5s 才算"成功"（防闪断风暴）
    reconnectionDelayGrowFactor: 1.3,
    connectionTimeout: 4000,  // 连接建立超时
    maxRetries: Infinity,
    maxEnqueuedMessages: Infinity,
};

private _getNextDelay() {
    let delay = 0;
    if (this._retryCount > 0) {
        delay = minReconnectionDelay * Math.pow(reconnectionDelayGrowFactor, this._retryCount - 1);
        if (delay > maxReconnectionDelay) delay = maxReconnectionDelay;
    }
    return delay;
}

public send(data: Message) {
    if (this._ws && this._ws.readyState === this.OPEN) this._ws.send(data);
    else {
        if (this._messageQueue.length < maxEnqueuedMessages) this._messageQueue.push(data);
    }  // 断线时入队，open 后 flush（_handleOpen 中 forEach send）
}
```

- **可转写模式（对应 C-3 / ws.ts 现状）**：
  1. CSC 的 ws.ts 已有指数退避（`retryDelay = min(retryDelay*2, 15000)`，ws.ts:17/131），对照差异：**缺随机化**（防多客户端同时重连的惊群，单用户可忽略）、**缺 connectionTimeout**（连接挂起探测）、**缺 minUptime**（闪断风暴防护）。三者都是低成本的既有模式补强，flash 可实现时按需取用。
  2. **消息队列**：reconnecting-websocket 断线时缓冲 send——CSC 有意不缓冲决策（`sendDecide` 断线时返回 false，调用方回退 REST，ws.ts:78-81 注释），这是**正确的取舍**（决策提交必须走权威路径不能重放旧消息，与 A-1 的 batch_id 防重放形成闭环）。文档明确：**CSC 不采用消息缓冲**，理由即 S1 漏洞。
  3. `_handleError` 中 TIMEOUT → `_disconnect` → `_connect()` 的链式恢复（:458-469）是 C-3「PONG_TIMEOUT 内未收到 pong → ws.close() → 既有 onclose 触发重连」的现成模板。

### 3.3 axum examples/websockets + CSC 现有 journal seq 游标

- **仓库**：https://github.com/tokio-rs/axum `examples/websockets/src/main.rs`（见 §1.3）
- **seq 游标映射**：CSC 已有 `GET /games/{id}/journal?since=N → {events, next_seq}`（routes.rs:474-533，`next_seq = last.seq()+1`，:487）+ WS `lagged` 提示（routes.rs:1597-1599）。缺的是 **WS journal 事件带 next_seq 游标**（终审 §3.2-4）。engine.io 的 sid + 断线重连用 `since` 续传模式与此同构。补充建议：`WsEvent::Journal` 加 `next_seq: i64` 字段（game.rs:192 枚举变体）后，前端 appendJournal 的去重/排序可用该游标替代纯本地推断。

---

## 4. 存档版本化：JSON schema 校验 + 迁移模式

### 4.1 schemars — 从 Rust 类型生成 JSON Schema（资产/存档 schema 校验的现成工具）

- **仓库**：https://github.com/GREsau/schemars（Rust，MIT，MSRV 1.74+）
- **关键文件**：`README.md`（Basic Usage / Serde Compatibility 两节）
- **核心模式摘录**（README）:

```rust
use schemars::{schema_for, JsonSchema};
#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MyStruct { ... }

let schema = schema_for!(MyStruct);
println!("{}", serde_json::to_string_pretty(&schema).unwrap());
```

关键特性：**schemars 会读取 `#[serde(...)]` 属性并调整 schema**（README Serde Compatibility 一节："Any generated schema should match how serde_json would serialize/deserialize"）——`deny_unknown_fields` → `additionalProperties: false`、`#[serde(default)]` → `"default": null`、`rename_all` → camelCase 键名。
- **可转写模式（对应 D-1 / 资产 schema 的 P1）**：
  1. CSC 的 `GameState` 已 `deny_unknown_fields` + version 字段 + migrate_state（state.rs:47-139），版本化骨架完整。**缺的是文档化 schema 与资产（assets/*.json）的 schema 校验**。schemars 可对 GameState 生成权威 schema 进 SAVE-FORMAT.md；对资产文件（standings/ratings/profile）可仿 CDDA chkjson 思路用 schemars+jsonschema 校验。
  2. 若引入 schemars，建议仅作为**开发期生成文档/校验**（build 脚本或 CI 步骤），不运行时依赖（CSC 的 deny_unknown_fields 已是运行时防线，schemars 是文档与离线校验补充）。

### 4.2 serde 生态成熟实践 — `deny_unknown_fields` + `#[serde(default)]` 迁移 + 显式版本门禁

- **仓库**：https://github.com/serde-rs/serde（serde 生态标准）
- **模式**（CSC 已部分采用，此节为「对齐工业惯例的确认 + 补强点」）：
  1. **未知字段拒绝**（`deny_unknown_fields`）：防格式漂移静默默认化。CSC 已在 PersistedGame/GameState/LockRecord 三处采用。
  2. **`#[serde(default)]` 做前向兼容**：新字段补默认值。CSC 的 top20_history/scheduled_records（state.rs:80-85）采用。
  3. **显式版本号 + 未知版本拒绝加载**：CSC `from_json_str`（state.rs:107-122）已实现 `version > CURRENT_VERSION → Err`。工业惯例中更严的做法是**版本号用语义化版本（semver）而非单调整数**——见 §4.3。
  4. **迁移函数集中化**：`migrate_state`（state.rs:130-139）在「读入后、使用前」单点迁移。对照 OpenTTD 的做法（savegame 版本逐级升级，`saveload/` 目录整套 SLF 框架），CSC 的规模用单函数足够，但 D-1 要求迁移记录集中到 SAVE-FORMAT.md（v1–v8 字段级 diff 表）。

### 4.3 firecracker — `SNAPSHOT_VERSION: semver::Version` 常量（版本化快照的工业样板）

- **仓库**：https://github.com/firecracker-microvm/firecracker（AWS 开源 microVM，快照/恢复是核心功能）
- **关键文件**：`src/vmm/src/persist.rs`
- **核心模式摘录**（persist.rs:15、165-166）：

```rust
use semver::Version;
/// Snapshot version
pub const SNAPSHOT_VERSION: Version = Version::new(11, 0, 0);
```

快照加载路径对 `SNAPSHOT_VERSION` 做 semver 兼容性判断（`semver::VersionReq` 匹配），比单调整数能表达「major 不兼容 / minor 可迁移」语义。
- **可转写模式（对应 B-1 sim_version / D-1 bump 规则）**：
  1. CSC 的 `version: u32`（存档格式）保持不动（现有迁移逻辑依赖单调整数）；新增的 `sim_version`（指纹链）也可先用 u32（B-1 指令已定 `WORLD_SIM_VERSION: u32 = 1`）。**若未来模拟语义频繁变更**，semver 化的收益才显现——文档记录此权衡即可，本轮不引入 semver 依赖。
  2. firecracker 的「版本常量 + 加载时校验」双件套与 open-football 的 `PROTOCOL_VERSION`（§1.2）形成两个独立佐证：**版本标签必须是加载/握手的显式检查项**，B-1 的 `Engine::restore` 忽略校验但指纹比对用（指令已定）符合「恢复宽容、门禁严格」的分工。

---

## 5. 给 flash 实现的转写要点汇总（按 CSC 修复项）

| CSC 修复项 | 转写蓝本 | 关键签名/结构 | 直接映射点 |
|---|---|---|---|
| **A-1 G1 三件套** | OpenTTD `ReceiveClientCommand` | `try_send` + 状态机前置校验 + 队列上限语义 | `GameEntry::submit_decisions(&self, decisions, batch_id: Option<u64>) -> Result<(), String>`；`TrySendError::Full → "决策通道已满"` |
| **A-4 决策超时** | open-football `REMOTE_BATCH_TIMEOUT` + 注释语义 | `recv_timeout(Duration)` → 超时分支用 `AutoDecisionSource::default_decision_for` | `ChannelDecisionSource` 加 `decision_timeout: Duration` 字段；`std::sync::mpsc::Receiver::recv_timeout`（std 自带，不换 tokio channel） |
| **A-3 WS Advance clamp** | axum 示例的输入校验纪律 | `months.unwrap_or(1).clamp(1, 240)` | 与 REST routes.rs:1113-1114 对齐 |
| **B-1 指纹门禁** | bevy_ggrs ChecksumPart→XOR + ggrs SyncTest + ruff CacheKey | `fnv1a64(canonical_json_bytes(state))`；`Vec<(month, u64)>` 序列；mismatch → `assert_eq!` | `canonical_json_bytes<T: Serialize>` 提取到 csc-util；`GameState.sim_version` + `WORLD_SIM_VERSION`；`tests/fingerprint.rs`（不 ignore） |
| **B-3 M5 Box-Muller** | open-football `HydrationRng::normal` | `u1.max(1e-10)` 只拦下界 | 佐证 csc-util::gaussian 口径正确，generator 上界 clamp 可删 |
| **C-3 WS 心跳** | engine.io Heartbeat + reconnecting-websocket | `pingInterval=30s`/`pingTimeout=10s`；`{type:"ping"}/{type:"pong"}` JSON 消息 | ws.ts `GameSocket` 加 pingTimer/pongDeadline；routes.rs ws_loop 应答 ping；**不采用消息缓冲**（决策必须走 REST 回退，防重放） |
| **D-1 SAVE-FORMAT.md** | firecracker SNAPSHOT_VERSION + serde 迁移惯例 | 版本表 + bump 规则节 | state.rs:88-114 注释誊录 + 迁移记录强制同步条款 |
| **资产 schema（P1）** | schemars | `#[derive(JsonSchema)]` + `schema_for!` | 可选引入，仅开发期文档/校验用 |

## 6. 风险与未决项

1. **tower-governor 引入价值有限**：CSC 无鉴权（M7），限流只挡 DoS 不挡越权；且 G1 的根因是「阻塞 send」而非请求频率。结论：**本轮不引入**，仅在文档记录「公开部署时」的落法。
2. **服务端主动 ping vs 前端主动 ping**：engine.io v4 的历史教训（浏览器定时器不可靠）指向服务端主动 ping；C-3 指令取最小实现（前端主动），单用户工具可接受，但后台标签页场景心跳可能失效——升级路径已记录。
3. **u128 vs u64 指纹位宽**：ggrs/bevy_ggrs 用 u128；CSC 用 fnv1a64（已有测试向量 + 非安全边界），碰撞风险可忽略（64 位 × CI 断言场景）。若未来指纹用于多世界对账（非本机 CI），升级 u128。
4. **schemars 的 schema 结构跨版本可能漂移**（README 明示"exact structure of generated schemas may change between versions"）——若把 schema 生成结果提交进仓库，需锁 schemars 版本（--locked）。

---

## 附录：取证清单（全部 raw.githubusercontent.com / GitHub API 抓取，master/main 分支）

| 文件 | 仓库 | 分支 | 关键行 |
|---|---|---|---|
| src/network/network_server.cpp | OpenTTD/OpenTTD | master | 1108-1192（ReceiveClientCommand 校验链） |
| src/command.cpp | OpenTTD/OpenTTD | master | 162-189（test-run/execute）、363（test/exec 一致性 assert） |
| docs/desync.md | OpenTTD/OpenTTD | master | 全文（确定性理论 + 校验和 + 分段回放定位） |
| src/web/src/worker/protocol.rs | ZOXEXIVO/open-football | master | 1-84（PROTOCOL_VERSION、Ping/Pong、Handshake） |
| src/web/src/worker/dispatcher.rs | ZOXEXIVO/open-football | master | 58-75（REMOTE_BATCH_TIMEOUT 注释语义） |
| src/database/src/generators/rng.rs | ZOXEXIVO/open-football | master | 11-106（HydrationRng + Box-Muller） |
| src/core/src/match/engine/flow/context/rng.rs | ZOXEXIVO/open-football | master | 42-63（splitmix64 种子展开 + RefCell<StdRng>） |
| docs/architecture.md | gschup/bevy_ggrs | main | 48-98（快照存储 + Checksum 管线） |
| src/sessions/sync_test_session.rs | gschup/ggrs | main | 9-24、116-138、204-220（SyncTest 双跑比对） |
| src/sync_layer.rs | gschup/ggrs | main | 14-24（GameStateCell::save 三元组）、140-162（SavedStates 环形窗口） |
| crates/ruff_cache/src/cache_key.rs | astral-sh/ruff | main | 16-85（CacheKey trait）、312-336（HashMap 键排序 + 长度前缀） |
| README.md（Heartbeat） | socketio/engine.io-protocol | main | 222-241（心跳协议）、371-374（v3→v4 服务端主动 ping 动机）、409-410（默认参数） |
| reconnecting-websocket.ts | pladaria/reconnecting-websocket | master | 39-49（默认参数）、252-263（send 缓冲）、309-325（指数退避）、458-469（TIMEOUT→重连） |
| examples/websockets/src/main.rs | tokio-rs/axum | main | 101-149（Ping 探活 + split + select! 双任务） |
| README.md | benwis/tower-governor | main | 34-90（GovernorConfigBuilder + GovernorLayer 示例） |
| README.md | GREsau/schemars | master | Basic Usage / Serde Compatibility 两节 |
| src/vmm/src/persist.rs | firecracker-microvm/firecracker | main | 165-166（SNAPSHOT_VERSION semver 常量） |
| src/persistence.rs | veloren/veloren | — | （404 未取得，已从清单移除，不影响结论） |
