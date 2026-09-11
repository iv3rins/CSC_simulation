# FRONTEND-API —— 前端对接文档（后端协议唯一入口）

> 读者：前端开发者。目标：不读 Rust 源码即可对接 CSC 服务端全部能力。
> 协议权威 = 代码：路由注册 `backend/crates/csc-server/src/routes/mod.rs::router()`（47 条 REST + 1 WS）；
> 本文档与代码同步维护，若发现不一致以代码为准并回报（后端文档同步铁律）。
> 最后校准：2026-09-06（P1-3/P1-4/P1-5/P2-6~9/11 修复后；存档 v9）。

---

## 0. 总览

- 服务端：axum REST + WebSocket。游戏以 `game_id: u64` 索引，一局 = 一个模拟线程。
- **分层心智模型**：
  - `/games/{id}/view`（ClientState）= 高频轮询的轻量视图（网页端主数据源）；
  - `/games/{id}/state`（GameState）= 完整存档（约 68MB @20 年）——**仅存档/调试用，勿高频拉**；
  - `/games/{id}/journal?since=` + WS `journal` 事件 = 事件流增量（seq 游标对账）；
  - WS `decisions`/`step`/`notice` = 主动推送（Human 策略必需）。
- **统一错误格式**：所有非 2xx 响应体为 `{ "error": "<人话原因>" }`。常用状态码：404 游戏不存在 / 409 批次不匹配 / 422 业务拒绝 / 413 请求体超限。
- **请求体上限 128MB**（长生涯读档用；axum 默认 2MB 会把 `/load` 打成 413）。

---

## 1. REST 端点清单（47 条）

### 1.1 生命周期与存档

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/health` | 健康检查；返回 `{status,games,persisted_games,max_active}` |
| POST | `/games` | 建局。体：`{seed?,policy?,player_name?,role?}` → `{game_id}`。`policy`=`auto`/`human`；`role`=IGL/AWP/RIFLER/ENTRY/SUPPORT/LURKER |
| DELETE | `/games/{id}` | 关局并删除磁盘存档与 LIVE 会话 |
| GET | `/games/{id}/state` | 完整 GameState JSON（存档/调试；勿高频） |
| GET | `/games/{id}/save` | 下载存档：`application/json` + `Cache-Control: no-store` |
| GET | `/games/{id}/save/gzip` | 下载 gzip 存档（裸 gzip 字节，无 Content-Encoding）。响应头见 §3.2 |
| POST | `/games/{id}/load` | 读档。体 = GameState JSON（低版本自动迁移到 v9） |
| POST | `/games/{id}/load/gzip` | gzip 读档。体 = gzip 字节；可选请求头 `x-csc-expected-crc32` 让服务端校验 |

### 1.2 推进与决策

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/games/{id}/advance` | 推进。体 `{months?}` 或 `{days?}`（**互斥**，同传 → 400）；auto → `{summaries:[...]}`，human → 202 |
| POST | `/games/{id}/advance-season` | 推进到赛季边界 |
| GET | `/games/{id}/decisions/pending` | 当前待决策批次（Human；`PendingBatch` 或 null） |
| POST | `/games/{id}/decisions` | 提交决策。体：`{decisions:[...],batch_id?}`；batch_id 不匹配 → 409 |
| GET | `/games/{id}/training/options` | 训练计划选项 |
| POST | `/games/{id}/training` | 排定训练。体 `{focus}` |
| POST | `/games/{id}/season-goal` | 设赛季目标。体 `{goal}` |
| POST | `/games/{id}/finance` | 资金运用 |
| POST | `/games/{id}/actions/statement` | 公开表态。体 `{tone}` |
| POST | `/games/{id}/actions/match-plan` | 赛前战术安排 |

### 1.3 查询

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/games/{id}/view` | ClientState 轻量视图（主数据源，高频轮询用） |
| GET | `/games/{id}/calendar` | 赛季赛程（含已物化月份赛事） |
| GET | `/games/{id}/events/{event_name}` | 单赛事事件聚合 |
| POST | `/games/{id}/fixtures/watched` | 标记对局已看完（幂等） |
| GET | `/games/{id}/news` | 新闻流 |
| GET | `/games/{id}/replay` | `{series}` 单场回放懒加载 |
| GET | `/games/{id}/summary` | 世界概览 |
| GET | `/games/{id}/journal?since=N` | `{events,next_seq}` 事件流增量 |
| GET | `/games/{id}/archive` | 生涯档案 |
| GET | `/games/{id}/ending` | 生涯结局/终结视图 |
| GET | `/games/{id}/top20` | 本年度 TOP20 |
| GET | `/games/{id}/top20/history` | 历届 TOP20 |
| GET | `/games/{id}/top20/reference` | TOP20 参考依据 |
| GET | `/games/{id}/world` | 世界故事流 |
| GET | `/games/{id}/insights` | 洞察 |
| GET | `/games/{id}/calibration` | 校准视图 |

### 1.4 转会

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/games/{id}/transfers/market` | 转会窗市场（去弹窗契约） |
| GET | `/games/{id}/transfers/offers` | 主动转会候选（合同到期/买断后） |
| POST | `/games/{id}/transfers/sign` | 签约。体含目标队 |

### 1.5 LIVE 比赛（REST）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/games/{id}/live` | 开 LIVE。体 = `LiveMapConfig`。**幂等可重进**：同 `(game_id,match_id)` 已存在 → 覆盖重建而非 422 |
| GET | `/games/{id}/live/today` | 今日主角 pending 对阵 |
| POST | `/games/{id}/live/today/skip` | 跳过今日 LIVE（`skip` 持久语义，比赛照常确定性结算） |
| GET | `/games/{id}/live/{match_id}` | 会话视图 `LiveSessionView` |
| POST | `/games/{id}/live/{match_id}/advance` | 推进一回合 → `LiveAdvanceView` |
| POST | `/games/{id}/live/{match_id}/decide` | 提交回合决策 |
| GET | `/games/{id}/live/{match_id}/review` | 复盘 `LiveReview`（关键回合回放） |
| POST | `/games/{id}/live/{match_id}/skip` | 跳至赛果（有界回合，不悬挂） |
| POST | `/games/{id}/live/{match_id}/watch` | 标记已看完（幂等；skip/review 也置 watched） |

### 1.6 WebSocket

| 路径 | 说明 |
|---|---|
| `/games/{id}/ws` | 双向：服务端推 `WsEvent`，客户端发 `WsIn`。见 §5 |

---

## 2. 核心 DTO

### 2.1 ClientState（GET /view —— 高频轮询主数据源）

```jsonc
{
  "next_milestone": { /* CareerMilestone 或 null：首页 NEXT / LIVE 倒计时 */ },
  "view_version": 1,
  "month": 5,            // 模拟月份计数（0 起）
  "date": "2026-06-01",  // 显示日期
  "sim_year": 2026, "sim_month": 6, "sim_day": 1,
  "locks": [ /* LockRecord：本月进行中赛事队伍锁 */ ],
  "world": { /* ClientWorld：主角视角世界投影 */ },
  "season": { /* ClientSeason 或 null：主角年度 Rating 标量 */ },
  "archive": [ /* SeasonRecord：主角生涯逐赛季 */ ],
  "events": [ /* ClientTournamentResult：主角参赛过的赛事（series 已裁剪） */ ],
  "team_count": 128, "player_count": 1000, "npc_count": 872,
  "event_count": 12, "journal_len": 340, "decisions_logged": 8
}
```

### 2.2 GameState（GET /state —— 完整存档，勿高频拉）

唯一聚合根，`deny_unknown_fields`。顶层：`version`(当前 9) / `month` / `last_contract_year` / `locks` / `sim_year|month|day` / `rng_state[4]` / `decisions[]` / `journal[]` / `archive{}` / `world` / `vrs` / `events[]` / `yearly_rating` / `top20_history[]` / `scheduled_records[]` / `sim_version`。
前端通常**不需要**解析它——读档把它原样 POST 回 `/load` 即可（含迁移）。

### 2.3 StepSummary（advance 返回 / WS step）

```jsonc
{ "game_id": 7, "month": 6, "date": "2026-07-01", "day": null,
  "journal_added": 42, "decisions_made": 2,
  "season_end": false, "season_index": 1 }
```
`season_end == true` 时建议暂停自动推进并弹赛季总结。

### 2.4 PendingBatch（decisions/pending + WS decisions）

`GET /decisions/pending` 实际响应为 **包装对象** `{ "pending": PendingBatch | null }`
（不是裸 `PendingBatch`）。`PendingBatch` 本体：

```jsonc
{ "batch_id": 3, "date": "2026-06-30",
  "points": [ /* DecisionPoint[]：见 §4 决策流 */ ] }
```

### 2.5 WsEvent / WsIn（WS 载荷，`type` 判别）

服务端→客户端（`tag="type"`，snake_case）：`decisions`(PendingBatch) / `step`(StepSummary) / `journal`(`{events[],next_seq}`) / `notice`(`{message}` 可选 toast，忽略不破坏协议)。
客户端→服务端：`decide`(`{decisions[],batch_id?}`) / `advance`(`{months?}`)。

---

## 3. 存档协议（v9）

### 3.1 格式契约

- `GameState.version = 9`（2026-09-06 bump；v8→v9 = 无操作迁移，`scheduled_records`/`sim_version` 由 serde 补默认）。
- 未知版本（>9）拒绝加载；未知字段拒绝解析（`deny_unknown_fields`）；**旧档（v1–v8）仍可正确读入并自动迁移到 v9**。
- 迁移历史：v5 荣誉积分 ×0.1 归一；其余新字段 `serde(default)` 补默认。权威演进记录：`docs/SAVE-FORMAT.md`。
- 读档统一走迁移入口（`GameState::migrate`）：服务端 `/load`、`/load/gzip`、磁盘恢复、WASM `csc_restore` 全部一致——**前端不需要自己迁移**。

### 3.2 gzip 存档响应头（GET /save/gzip）

| 头 | 含义 |
|---|---|
| `Content-Type` | `application/gzip`（裸 gzip，不带 Content-Encoding，可直接落盘） |
| `x-csc-save-version` | 存档格式版本（当前 9） |
| `x-csc-save-date` | 存档日期标签 |
| `x-csc-json-bytes` | 未压缩 JSON 字节数 |
| `x-csc-gzip-bytes` | gzip 字节数（≈1/10） |
| `x-csc-save-crc32` | 8 位 hex CRC32（校验用） |

读回：POST `/load/gzip` 时可带 `x-csc-expected-crc32` 让服务端校验（不匹配 → 422）。

---

## 4. 决策流（两级）

### 4.1 月度/事件批次（Human 策略）

```
服务端推进挂起 → WS 推送 { type:"decisions", ...PendingBatch }
  前端渲染决策面板
  用户提交 → POST /decisions 或 WS decide（带 batch_id 去重）
  服务端继续推进 → WS 推送 step / journal
```
- `batch_id` 用于重连去重：缺省/不匹配 → 409；前端应忽略已处理过的 batch_id。
- 决策点类型见 `DecisionPoint` 枚举（transfer / injury / sponsor / life / blunder / training 等，两级决策流共享同一类型体系）。

### 4.2 LIVE 回合决策

```
POST /live/{match_id}/advance → 遇关键回合返回 decision_request（含 reason）
  前端弹回合决策面板 → POST /live/{match_id}/decide → 继续 advance
```
- **关键回合判定唯一口径（P2-8 修复）**：以「回合开始前比分」为准——MatchPoint（任一方前已达 12 分）/ LateCloseScore（前合计 ≥20 差 ≤1）/ EconomyClash（ForceBuy vs Eco/HalfBuy）。复盘 `review` 与实时暂停用同一函数，标出来的关键回合与当时实际暂停过的回合**必然一致**。

---

## 5. LIVE 会话协议

- `LiveSessionView`：`{ state, finished, decision_required, decision_request, watched, played_rounds[], current_round }`。
- `LiveAdvanceView`：`{ output:{event,map_finished,series_finished}, state, decision_required, decision_request }`。
- `LiveReview`：`{ match_id, finished, final_score_a, final_score_b, key_rounds[] }`；`key_rounds[].reason` 见 §4.2。
- 语义：
  - **start 幂等可重进**：同一 `(game_id, match_id)` 重复 start = 覆盖重建（丢弃旧进度与 watched）。
  - **watched 语义**：看完 = `mark_watched` / `skip` / `review` 任一路径置位；幂等，前端据此不再重复弹「观看」入口。
  - **skip 有界不悬挂**：服务端推进有 MAX_ROUNDS 硬上界，永远返回。
  - `played_rounds`（已播回合编号升序）+ `current_round`（最近回合含击杀流）由服务端下发，前端不必自行遍历 `state.round_history`。

---

## 6. 前端注意事项（后端契约侧）

1. **别高频拉 `/state`**：完整快照 20 年约 68MB；轮询用 `/view`（轻量）+ `/journal?since=`（增量）+ WS。
2. **读档即迁移**：POST 旧版 GameState 到 `/load` 会被自动升到 v9；响应不含新档，需重新 GET。
3. **Human 策略推进是异步的**：POST `/advance` 可能 202 + 后续 WS `step` 事件回报进度；等待决策期间 `advancing` 保持 true。
4. **batch_id 幂等**：决策提交带批次号，重复提交被服务端去重（409）。
5. **错误永远是人话 JSON**：`{ "error": "..." }`；显示给玩家前无需二次解析。
6. **LIVE 覆盖语义**：重复开同一场 = 重来，旧会话回合进度与 watched 被丢弃——需要历史回放走 `/replay` 懒加载端点。
7. **WS `notice` 可忽略**：系统提示（如决策超时自动继续）不破坏协议；确定性事件走 `journal`（同种子同决策可复现）。
# 2026-09-11 文字生涯增量契约

新展示工程在 `web/`（Solid/TypeScript/Vite）；运行与限制见 `web/README.md`。现有查询接口继续复用。

- 新建 `{story:true,policy:"human",seed,player_name,role}`；响应 `{game_id}`，0是合法ID。
- GET `/games/{id}/career` 返回 `{game_id,story,generation,status,capabilities:{advance_world:false},limitation,scene,progress,pending,view}`。scene包含instance_id/scene_id/date/actor_role/actor_name/title/chapter_title/location/paragraphs/choices；choices的impact_certain/impact_possible均为**字符串**。
- POST `/games/{id}/career/choices` 请求 `{generation,request_id,scene_instance_id,choice_id}`，成功返回同名身份字段、`status:"committed"`和`outcome:{immediate:string[],tracking:string[]}`。tracking是内部稳定键，界面不要直接显示。成功意味着本次场景提交已写入配置的存档目录；没有目录或写入失败拒绝提交。
- 相同完整请求在场景已消费/正常重启后返回同receipt；异内容和旧generation拒绝。先选择radio再确认；不确定的网络结果重试沿用request_id。
- POST `/games/{id}/career/continue`目前只寻找已满足条件的后继。无场景409；不得用旧advance替代，当前**不能完整推进职业生涯**。
- GET `/games/{id}/career/history`为`{items:receipt[],next_cursor:null}`；GET `/games/{id}/career/operations/{request_id}`为已提交receipt或404。首版最多64条receipt，满后明确停止新增选择。
- JSON save/load继续使用既有端点，GameState v11；显式load更换transport generation，前端清除旧草稿/回执。正常磁盘恢复使用envelope v3保持身份。

下文为原接口文档；遇版本/新功能冲突以本增量及analysis/10为准。
