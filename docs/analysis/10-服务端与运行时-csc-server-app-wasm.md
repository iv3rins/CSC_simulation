# 10 · 服务端与运行时：csc-server / csc-app / csc-wasm

> 2026-09-11新增：`game/career.rs` 单局序章命令与持久回执，`routes/career.rs` 真实HTTP适配。下表为已实现的有限能力；**正式世界/赛事continuation未完成，story禁止旧advance/advance-season，包括WS调用request_advance**。

| 方法与路径 | 已实现契约 |
|---|---|
| POST /games | 增 `story`，缺省false；true要求human，装配内容并开始序章 |
| GET /games/{id}/career | generation、scene、progress、pending、view、capabilities.advance_world=false |
| POST /games/{id}/career/choices | generation/request_id/scene_instance_id/choice_id；成功后返回完整committed receipt与真实outcome |
| POST /games/{id}/career/continue | 只选满足条件后继；无可选则409，不推进世界、不代答 |
| GET /games/{id}/career/history | items=已提交receipt，next_cursor=null；首版最多64条 |
| GET /games/{id}/career/operations/{request_id} | 已知receipt或404；不是后台模拟任务伪进度 |

`CareerSession`是envelope v3传输状态，不入GameState模拟指纹。actor内部候选Engine→全量效果校验→写盘→交换→回执。重复请求先按完整身份查询旧receipt，不要求原scene仍pending；容量64满明确拒绝新选择，不淘汰复活旧请求。显式load在候选引擎恢复后生成并持久化新generation再交换；正常磁盘恢复保持原generation/receipt。新路由通过spawn_blocking等待命令，保持租约，不阻塞Tokio worker。旧Human `submit_decisions`可靠性遗留仍见25号报告，不因新场景链通过而视为修复。

> csc-server：axum REST + WS 服务端（每局一线程 + 租约 + 存档 gzip + WS 心跳）。
> csc-app：交互式 CLI。csc-wasm：浏览器单机模式。

---

## 一、csc-server（axum REST/WS）

### 1. 全局结构（src/）

| 文件 | 职责 |
|---|---|
| `main.rs` | 入口（读 assets 目录 + 端口） |
| `lib.rs` | 库根 |
| `state.rs` | `AppState`（游戏注册表 + 容量模型 + 持久化） |
| `game/mod.rs` | 单局运行时（模拟线程 + 决策通道 + 存档 + WS 会话） |
| `live.rs` | LIVE 会话管理（逐回合推进 + 决策） |
| `routes/` | REST/WS 路由（按领域拆分 7 文件：mod.rs=router+err/game_of/health 146 行、lifecycle.rs=生命周期+存档 384、query.rs=GET 查询 594、advance.rs=推进 151、decision.rs=决策/训练/财务/主动操作 210、transfer.rs=转会 72、live.rs=LIVE+WS 302） |
| `tests/api.rs` | 集成测试（1570 行） |
| `examples/advance_profile.rs` | 推进性能画像 |

### 2. Struct：`AppState`（src/state.rs）

```
games: GameRegistry（get_or_restore / lease_entry / touch / count / persisted_count / max_active）
capacity: 容量模型（max_active 等）
config: 配置（assets 目录等）
```

**容量模型**（docs/CAPACITY-MODEL.md）：
- `get_or_restore` 只淘汰**空闲局**（推进/保存/落盘/租约中都不动）；
- 请求路径登记租约（`GameHandle`，Drop 自动释放）——路由永远拿不到「已退出的模拟线程」；
- `/health` 后台排发收缩（长档 gzip 落盘是秒级重操作，不阻塞请求路径）；
- 多局 Evict/Resume：`restore_with_profile` 保留新秀实力分布配置。

### 3. Enum：`Policy`（src/game/mod.rs）

`Auto`（全自动推进）/ `Human`（决策点弹窗等待玩家）。

### 4. REST 端点全集（routes/ 目录）

| 方法 + 路径 | 说明 |
|---|---|
| `GET /health` | 健康检查（状态 + 局数 + 容量） |
| `POST /games` | 创建游戏 `{ seed?, policy?, player_name?, role? } → { game_id }` |
| `DELETE /games/{id}` | 关闭游戏 |
| `GET /games/{id}/state` | 完整 GameState（存档/调试） |
| `GET /games/{id}/view` | 轻量 ClientState（高频轮询） |
| `GET /games/{id}/calendar?year=N` | 年度赛历 + 赛果摘要 |
| `GET /games/{id}/events/{event_name}` | 赛事对象化聚合（R2） |
| `POST /games/{id}/fixtures/watched` | 标记对阵已看完 |
| `GET /games/{id}/news` | 世界新闻 |
| `GET /games/{id}/replay?event=&date=&index=` | 单场回放懒加载 |
| `GET /games/{id}/summary` | 世界概览 |
| `GET /games/{id}/journal?since=&channel=&limit=` | 增量事件（分页 + 双频道） |
| `GET /games/{id}/archive` | 生涯档案 |
| `GET /games/{id}/ending` | 生涯结局 |
| `GET /games/{id}/save` / `POST /load` | 存档往返（JSON） |
| `GET /games/{id}/save/gzip` / `POST /load/gzip` | gzip 存档（流式压缩，约 1/10） |
| `POST /games/{id}/advance` | 推进 `{ months? | days? }` |
| `POST /games/{id}/advance-season` | 推进到赛季边界（FIFA 主入口） |
| `GET /games/{id}/top20` / `top20/history` / `top20/reference` | TOP20 榜单/历史/参考 |
| `GET /games/{id}/world` | 世界故事 |
| `GET /games/{id}/insights` | 生涯洞察 |
| `GET /games/{id}/calibration` | 校准资产 |
| `GET /games/{id}/decisions/pending` | 待决策批次 |
| `POST /games/{id}/decisions` | 提交决策 `{ decisions, batch_id }`（batch_id 陈旧校验） |
| `GET /games/{id}/training/options` | 训练选项 |
| `POST /games/{id}/training` | 排定训练 `{ focus }` |
| `POST /games/{id}/season-goal` | 设置赛季目标 |
| `POST /games/{id}/finance` | 资金运用 `{ action: BUYOUT\|INVEST\|SKIN, param? }` |
| `GET /games/{id}/transfers/market` | 转会窗市场 |
| `GET /games/{id}/transfers/offers` | 转会候选 |
| `POST /games/{id}/transfers/sign` | 立即签约 `{ team_signature }` |
| `POST /games/{id}/actions/statement` | 主动发言 `{ tone }` |
| `POST /games/{id}/actions/match-plan` | 主动 BP `{ style }` |
| `POST /games/{id}/live` | 开始 LIVE `{ config }` |
| `GET /games/{id}/live/today` | 今日 LIVE 闸门 |
| `POST /games/{id}/live/today/skip` | 跳过今日 LIVE |
| `GET /games/{id}/live/{match_id}` | LIVE 会话视图 |
| `POST /games/{id}/live/{match_id}/advance` | 推进下一回合 |
| `POST /games/{id}/live/{match_id}/decide` | 提交回合决策 |
| `GET /games/{id}/live/{match_id}/review` | 回放审查 |
| `POST /games/{id}/live/{match_id}/skip` | 跳过 LIVE |
| `POST /games/{id}/live/{match_id}/watch` | 标记已看完 |
| `WS /games/{id}/ws` | WebSocket（decisions/step/journal 推送 + decide/advance 指令） |

**请求体上限 128MB**（20 年存档 ≈68MB；axum 默认 2MB 会把 /load 打成 413）。

**比赛日闸门（`POST /games/{id}/advance` 与 `/advance-season`）**：
- Human 策略下，只要**今天**有主角可 LIVE 的 pending 对阵（`live_gate_active()`，
  `GET /live/today` 非空），**任何粒度**的推进（`days` 日级 / `months` 月级 / 赛季）一律
  `409 CONFLICT`——日级推进同样拦截（P2 修复：手动「下一天」不再绕过比赛日闸门）。
- 玩家必须先处理比赛日：进入 LIVE（`POST /games/{id}/live`）或跳过
  （`POST /games/{id}/live/today/skip`，把今日 pending 对阵标记 `skipped`）后闸门关闭，
  推进放行；跳过的比赛仍由月末 `settle_month` 确定性结算比分。
- 非比赛日的日推进（逼近比赛日）正常放行（202 受理）。

### 5. WS 协议（src/game/mod.rs + routes/live.rs）

```
服务端 → 客户端: { type:"decisions", batch_id, points } / { type:"step", ... } / { type:"journal", ... } / { type:"notice", ... }
客户端 → 服务端: { type:"decide", batch_id, decisions } / { type:"advance", months? }
```

> `WsEvent` 实际 **4 变体**：`Decisions / Step / Journal / Notice`（game/mod.rs:168-183）。

**可靠性协议**（G1/S1/S2 闭环）：
- `submit_decisions(decisions, batch_id: Option<u64>)`：`advancing` 前置检查 → batch_id 比对
  （不持锁 send）→ `try_send`（Full → 429）；陈旧/错误 batch_id → 409；
- 决策超时 `DECISION_TIMEOUT=300s`：recv_timeout 三分支（Timeout → AutoDecisionSource 单一事实源）；
- WS 心跳：客户端 30s ping/pong 判死 close 重连（R2 起服务端主动 ping + 判死）。

### 6. 单局运行时（src/game/mod.rs）

```
GameHandle（租约）→ 模拟线程（每局一个）：
  - 决策通道：mpsc channel 桥接 ChannelDecisionSource（game/mod.rs:382）
  - 存档：save/load + gzip 流式压缩（crc32 完整性校验）
  - WS 会话：每连接独立订阅（journal 增量推送）
  - LIVE：LiveSession 状态机（advance/decide/skip/watch）
```

---

## 二、csc-app（CLI）

### Struct：`StdinDecisionSource`（src/lib.rs）

`{ text: TextBundle }`，实现 `DecisionSource`：渲染决策点 + stdin 读取选项。
`render.rs`：决策点/世界状态文本渲染。`main.rs`：交互循环入口
（`cargo run -p csc-app -- ../assets`）。

---

## 三、csc-wasm（WASM 绑定）

`src/lib.rs`（285 行）：**纯 C ABI**（非 wasm-bindgen、无 Engine 方法直接导出）——
**7 个导出**（lib.rs:58-178）：

| 导出 | 说明 |
|---|---|
| `csc_create` | 创建引擎（含种子/校准） |
| `csc_advance` | 推进（内部走 `run_season`，lib.rs:110） |
| `csc_snapshot_json` | 存档 JSON |
| `csc_restore` | 读档 |
| `csc_journal_json` | 事件流 JSON |
| `csc_free` / `csc_string_free` | 内存释放 |

当前完成度：auto 推演 ✅ + 存档/事件流 ✅；
**交互模式（EngineStep 步骤化状态机）⏳ 未建**（路线图 WASM v2）。
CI 含 wasm 编译门禁：`cargo build -p csc-wasm --target wasm32`（ci.yml:35-37）。

---

## 四、部署与运行

```bash
# 后端全量测试（585 passed / 0 failed / 3 ignored）
cd backend && cargo test --workspace

# 静态检查（0 警告门禁）
cargo clippy --workspace --all-targets -- -D warnings

# 长程门禁（20 年浸泡）
cargo test -p csc-core --release --locked -- --ignored twenty_year

# 服务端
cargo run -p csc-server -- ../assets 8080

# CLI
cargo run -p csc-app -- ../assets

```
