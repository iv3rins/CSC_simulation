# CSC 后端 API 文档（Archify 生成）

> 协议权威 = 代码：`backend/crates/csc-server/src/routes/mod.rs::router()`（47 条 REST + 1 条 WS）。
> 本文档由代码实读生成；与 `docs/FRONTEND-API.md` 同源互补。统一错误格式：非 2xx 响应体 = `{ "error": "<人话原因>" }`。

## 通用约定

- **游戏标识**：`game_id: u64`，每局独立模拟线程
- **请求体上限**：128MB（长生涯读档需要；axum 默认 2MB 会 413）
- **分层心智模型**：
  - `GET /games/{id}/view` → `ClientState`（轻量视图，**高频轮询主数据源**）
  - `GET /games/{id}/state` → `GameState`（完整快照 ≈68MB@20年，**仅存档/调试**）
  - `GET /games/{id}/journal?since=` → 事件流增量（`next_seq` 游标对账）
- **常见状态码**：200 OK / 201 Created / 202 Accepted（Human 异步推进）/ 400 参数互斥 / 404 游戏或资源不存在 / 409 冲突（批次不匹配/推进中/闸门激活）/ 413 请求体过大 / 422 业务校验失败 / 504 存档操作超时（>300s）

---

## 1. 生命周期与存档

### GET /health
健康检查（毫秒级；后台排发容量收缩，不等待）。
```json
{ "status": "ok", "games": 3, "persisted_games": 1, "max_active": 4 }
```

### POST /games
创建游戏。**201** → `{ "game_id": 7 }`；**422** 创建失败（如资产损坏）。

请求体：
```jsonc
{
  "seed": 42,               // u64，缺省 Engine::DEFAULT_SEED
  "policy": "auto",         // "auto" | "human"，缺省 auto
  "player_name": "Player",  // 缺省 "Player"
  "role": "RIFLER"          // IGL|AWP|RIFLER|ENTRY|SUPPORT|LURKER，缺省=随机
}
```
说明：主角从 VRS 垫底队伍起步（青训新秀叙事）；主角卡 RNG 独立于引擎主 RNG（`seed ^ 0x5EED_C0DE`），同一世界可换卡。开局即物化当月赛事排程（修"卡赛事"）。

### DELETE /games/{id}
销毁游戏：模拟线程退出 + 存档与 LIVE 会话回收。**200** `{ "shutdown": true }` / **404** 不存在。

### GET /games/{id}/state
完整 `GameState` 快照（存档/调试用）。⚠️ 20 年生涯 ≈68MB，**勿高频拉取**。

### GET /games/{id}/save
下载存档（canonical JSON）。响应头 `Content-Type: application/json`、`Cache-Control: no-store`。**409** 已有存档任务进行中；**504** 超时。

### GET /games/{id}/save/gzip
下载 gzip 存档（裸 gzip 字节，不带 Content-Encoding，可直接落盘；实测 ≈1/10）。

响应头：
| 头 | 含义 |
|---|---|
| `Content-Type` | `application/gzip` |
| `x-csc-save-version` | 存档格式版本（当前 9） |
| `x-csc-save-date` | 存档日期标签 `YYYY-MM-DD` |
| `x-csc-json-bytes` | 未压缩 JSON 字节数 |
| `x-csc-gzip-bytes` | gzip 字节数 |
| `x-csc-save-crc32` | 8 位 hex CRC32（gzip 包校验） |

### POST /games/{id}/load
读档。体 = 完整 `GameState` JSON（自动迁移低版本 → v9）。**200** `{ "loaded": true }` / **422** 非法存档 / **409** 正在保存或读档。

### POST /games/{id}/load/gzip
gzip 读档。体 = gzip 字节。可选请求头 `x-csc-expected-crc32`（hex）让服务端校验，不匹配 → **422**。解压后上限 768MB（防解压炸弹）。

---

## 2. 推进 / 决策 / 主动操作

### POST /games/{id}/advance
推进。**auto** → 200 `{ "summaries": [StepSummary] }`；**human** → 202（进度经 WS Step 事件 / 状态轮询观察）。

```jsonc
{ "months": 1,   // 1..=240，缺省 1
  "days": null } // 1..=360，日级推进（逐日文字直播）；与 months 互斥，同传 → 400
```
**409**：本局正在推进中 / 比赛日闸门激活（Human 策略下今日有 pending LIVE 对阵，须先 `POST /live` 进入或 `POST /live/today/skip` 跳过——日级推进同样被闸门拦截，防止"下一天"绕过）。

### POST /games/{id}/advance-season
推进到赛季边界（FIFA 式赛季跳转；体 = `{}`）。**200** `{ "mode": "season", "summaries": [...] }` / **409** 闸门激活或推进中。

### GET /games/{id}/decisions/pending
当前待决策批次（Human）。**200** `{ "pending": PendingBatch | null }`。

### POST /games/{id}/decisions
提交决策。**200** `{ "submitted": true }` / **409** batch_id 缺省或不匹配。
```jsonc
{ "decisions": [{ "point_id": "...", "option_id": "..." }],
  "batch_id": 3 }  // 对应 PendingBatch.batch_id；重连去重
```

### GET /games/{id}/training/options
训练计划选项（前端主动拉取展示；训练已从月度强制决策改为主动触发）。

### POST /games/{id}/training
排定下月训练计划（写入 `CareerInfo.pending_training`，下一月推进开头应用）。
```jsonc
{ "focus": "AIM" }  // AIM|UTILITY|CLUTCH|PHYSICAL|MENTAL|COMMUNICATION|REST
```
**200** `{ "planned": true, "focus": "AIM", "applies_next_month": true }` / **409** 推进中 / **422** 非法 focus。

### POST /games/{id}/season-goal
设置本赛季主目标（写入存档 v6+）。
```jsonc
{ "goal": "WIN_TITLE" }
// WIN_TITLE|TOP20|IMPROVE_RATING|SECURE_STARTING|SEEK_TRANSFER|RECOVER_FORM
```

### POST /games/{id}/finance
资金运用（买断自己换队 / 投资自己 / 购买饰品）。
```jsonc
{ "action": "BUYOUT" }                    // 买断合同进自由市场
{ "action": "INVEST", "param": "AIM" }    // AIM|ENDURANCE|MENTAL
{ "action": "SKIN", "param": "rare" }     // param 缺省 = 标准
```
**200** `{ "cash": 12345, "message": "...", "skin?": {...} }` / **409** 推进中 / **422** 参数非法或资金不足。

### POST /games/{id}/actions/statement
发表声明（媒体/舆论）。
```jsonc
{ "tone": "humble" }   // 语气词，见引擎 make_statement 支持集
```
**200** `{ "message": "..." }`。

### POST /games/{id}/actions/match-plan
设置赛前 BP/战术安排。
```jsonc
{ "style": "aggressive" }
```
**200** `{ "set": true }`。

---

## 3. 查询

### GET /games/{id}/view
`ClientState` 轻量视图（高频轮询主数据源；长生涯下保持 MB 级以下，完整快照的 1~3%）。
```jsonc
{ "next_milestone": {...}|null, "view_version": 3, "month": 5, "date": "2026-06-01",
  "sim_year": 2026, "sim_month": 6, "sim_day": 1, "locks": [...],
  "world": { /* ClientWorld */ }, "season": {...}|null,
  "archive": [ /* SeasonRecord 逐赛季 */ ],
  "events": [ /* ClientTournamentResult，series 已裁剪 */ ],
  "team_count": 128, "player_count": 640, "npc_count": 872,
  "event_count": 34, "journal_len": 1200, "decisions_logged": 56 }
```

### GET /games/{id}/calendar?year=2026
年度赛历（已物化月份赛事）。
```jsonc
{ "year": 2026, "sim_date": "2026-06-15",
  "plan": [ /* 全年完整排期，与引擎动态排程同源 */ ],
  "results": [ /* 该年已完赛全部赛事摘要（含 NPC；不含逐图 KDA） */ ],
  "player_events": [ /* 主角实际参与的赛程（P0-A 修复：不再用"全部同档赛事"误展示） */ ] }
```

### GET /games/{id}/events/{event_name}
单赛事聚合对象（R2 赛事对象化：参赛队伍+VRS+冠军+对阵布局+空状态原因）。
```jsonc
{ "name": "...", "tier": "T1", "date": "...", "end_date": "...",
  "status": "future|active|completed|cancelled",
  "teams": [...], "fixtures": [ClientFixture], "champion": {...}|null,
  "player_participating": true, "emptiness_reason": null }
```
**404** 赛事不存在。

### POST /games/{id}/fixtures/watched
标记某赛事对局已看完（持久语义，幂等，写入存档）。
```jsonc
{ "event_name": "...", "fixture_id": 12 }
```
**200** `{ "watched": true, "found": true }` / **409** 冲突。

### GET /games/{id}/news
世界新闻流（近期 T1+ 对阵，含 NPC；每场赛事最多取最后 3 场，共 ≤10 条）。
```jsonc
{ "items": [{ "date": "...", "event_name": "...", "tier": "T1",
             "winner": "...", "loser": "...", "score": "2:0" }] }
```

### GET /games/{id}/replay?event=...&date=...&index=0
单场直播回放懒加载（series 不在 ClientState 中下发）。回合级回放由 `ReplayBuilder` 纯函数确定性生成（种子派生自 series 本身，不消耗主 RNG）。
**200** `{ "series": {...}, "replay": {...} }` / **404** 不可回放或不存在。

### GET /games/{id}/summary
世界概览计数（数据源已内联进 `/view`，此端点为兼容保留）。

### GET /games/{id}/journal?since=N&channel=...&limit=...
事件流增量。
- `since`：已同步游标（缺省 -1 = 全量）
- `channel`：`career_feed`（默认，主角相关）| `world_wire`（完整世界线）
- `limit`：服务端分页上限，缺省 200，clamp(1, 1000)

**200** `{ "events": [...], "next_seq": 1234 }`
- `next_seq` = 截断窗口末端 + 1（**不是**全流末端——否则中间事件被游标跳过永久丢失）；空窗口兜底全流末端
- **channel 可见性过滤在 take 之前**（先 take 再过滤会导致 career_feed 0 条但游标推进，中间事件丢失）
- 事件附带 `narration`（title/body/aside，由 Narrator 纯函数渲染 + text 文案包）

### GET /games/{id}/archive
生涯档案。**200** `{ "archive": { "<player_id>": [SeasonRecord] } }`（整数键 JSON 字符串化）。

### GET /games/{id}/ending
生涯结局/复盘（退役页数据源）。
```jsonc
{ "player_id": 0, "player_name": "...", "tier": "\"LEGEND\"", "tier_label": "...",
  "goat_score": 87.5, "retired": false, "verdict": "...", "review": [...] }
```
生涯尚短 → `tier="ROOKIE"`；未开启 → `{ "pending": true, "note": "..." }`；退役后仍可查询。

### GET /games/{id}/top20
本年度 TOP20 实时榜。**跨年回退**：新赛季初期实时榜正常入围 <20 人时回退展示最近一届完整榜单。
```jsonc
{ "year": 2026, "source": "CURRENT" | "COMPLETED", "entries": [Top20Entry] }
```

### GET /games/{id}/top20/history
历届年度 TOP20 榜单快照（含 NPC 名次与入选依据）。**200** `{ "boards": [...] }`。

### GET /games/{id}/top20/reference
真实 HLTV TOP20 三年「传奇参照」榜（`assets/top20-data/*.json`；有年龄基线时补 `age_in_year`）。**200** `{ "years": [{ "year": 2024, "players": [...] }] }`。

### GET /games/{id}/world
世界故事流（队伍风云 + 宿敌：谁在重建/崛起/王朝/低谷；纯只读派生，不新增状态）。

### GET /games/{id}/insights
生涯洞察（结果解释层：为什么有/没有转会报价、比赛洞察、队伍状态）。
```jsonc
{ "transfer": {...}|null, "match": {...}|null, "team": {...}|null }
```

### GET /games/{id}/calibration
生态系统校准报告（真实三年 TOP20 轨迹拟合虚拟成长/衰减/轮换参数 + 新人走势推演）。

---

## 4. 转会

### GET /games/{id}/transfers/market
转会窗市场（去弹窗契约：从 pending 批次提取主角 `DecisionPoint::TransferWindow` 报价）。
**200** `{ "market": {...} | null }`（null = 无挂起报价，前端显示空态）。
market 含 `date`、`deadline`（报价月月末）、`offers[]`（含确定性薪资）。

### GET /games/{id}/transfers/offers
当前可签约候选（合同到期 / 买断后才有）。**200** `{ "offers": [TransferOffer] }` / **422** 无主角。
`TransferOffer`：`{ point_id, player_id, player_name, from_team_id?, from_team_signature?, candidates: [{ team_id, team_signature, ranking, power_delta, salary }], discounted, current_ranking? }`

### POST /games/{id}/transfers/sign
签约。
```jsonc
{ "team_signature": "TeamY|p,q,r,s,t" }
```
**200** `{ "message": "转会完成：...", "event": {...}, "cash": 123 }` / **409** 推进中 / **422** 非法签名或不可签。

---

## 5. LIVE 比赛（REST）

### POST /games/{id}/live
开 LIVE。**幂等可重进**：同 `(game_id, match_id)` 已存在 → 覆盖重建（丢弃旧进度与 watched）。**201** → `LiveSessionView` / **409** 冲突。

体 = `LiveMapConfig`：
```jsonc
{ "match_id": "m1", "series_id": "s1", "map_id": "map_1", "map_number": 1,
  "team_a_id": 0, "team_b_id": 1, "team_a_sig": "A|...", "team_b_sig": "B|...",
  "best_of": 1, "world_seed": 42, "base_win_prob_a": 0.55,
  "team_a_players": [{ "id": 0, "name": "a0" }], "team_b_players": [...] }
```

### GET /games/{id}/live/today
今日主角 pending 对阵（比赛日闸门查询）。**200** `{ "matches": [LiveGateFixture] }`（空数组 = 今天无玩家比赛可 LIVE）。
`LiveGateFixture`：`{ event_name, event_date, tier, fixture_id, round, team_a, team_b, stage, best_of, team_a_roster?, team_b_roster? }`

### POST /games/{id}/live/today/skip
跳过**今日** LIVE 提示（持久语义）：把主角今日 pending 对阵全部标记 `skipped`。**不改变比赛结算**——月末仍走确定性后台模拟回填比分。
**200** `{ "skipped": 2 }`。

### GET /games/{id}/live/{match_id}
查询 LIVE 会话。**200** `LiveSessionView` / **404** 不存在。
```jsonc
{ "state": LiveRoundState, "finished": false, "decision_required": true,
  "decision_request": { "round_number": 13, "reason": "match_point",
                        "candidates": [...], "economy_a": "...", "economy_b": "..." },
  "watched": false, "played_rounds": [1,2,...], "current_round": {...}|null }
```
关键回合判定（reason）：`match_point`（任一方回合前已达 12 分）/ `late_close_score`（回合前合计 ≥20 差 ≤1）/ `economy_clash`（ForceBuy vs Eco/HalfBuy）——**统一以回合开始前比分为唯一口径**（P2-8 修复）。

### POST /games/{id}/live/{match_id}/advance
推进一回合。**200** `LiveAdvanceView` `{ output: { event, map_finished, series_finished }, state, decision_required, decision_request }` / **404** 不存在 / **409** 需先提交决策。

### POST /games/{id}/live/{match_id}/decide
提交回合决策。
```jsonc
{ "decision": { "type": "change_pace", ... } }
// LiveRoundDecision: None|CallTimeout|ChangePace(Pace)|AggressiveOpening|
//   PlayForTrade|Save|ForceBuy|ChangeUtilityPlan(UtilityPlan)|Encourage(PlayerId)|Criticize(PlayerId)
```

### GET /games/{id}/live/{match_id}/review
复盘回放（关键回合，与实时暂停同口径）。
**200** `LiveReview` `{ match_id, finished, final_score_a, final_score_b, key_rounds: [{ round_number, reason, ... }] }` / **404**。

### POST /games/{id}/live/{match_id}/skip
跳至赛果（有界回合：`MAX_ROUNDS=48` 硬上界，永不悬挂）。**200** `LiveSessionView`。

### POST /games/{id}/live/{match_id}/watch
标记已看完（幂等；skip/review 路径也置 watched）。**200** `{ "watched": true }`。前端据此不再重复弹"观看回放"入口。

---

## 6. WebSocket

### /games/{id}/ws

**服务端 → 客户端**（`WsEvent`，`tag="type"` + `snake_case`）：

| type | 载荷 | 说明 |
|---|---|---|
| `decisions` | `PendingBatch { batch_id, date?, points: [DecisionPoint] }` | 新决策批次（决策面板渲染） |
| `step` | `StepSummary { game_id, month, date, day?, journal_added, decisions_made, season_end, season_index }` | 月/日步完成；`season_end=true` 时建议暂停自动推进弹赛季总结 |
| `journal` | `{ events: [...], next_seq }` | 事件流增量（与 REST `journal?since=` 的 `next_seq` 同口径） |
| `notice` | `{ message }` | 系统即时提示（如决策超时自动继续）；**可忽略不破坏协议**（确定性事件走 journal） |
| `ping` | `{ ts }` | 服务端主动心跳（30s 周期）；客户端回 `{"type":"pong"}`，10s 无 pong 判死 |
| `lagged` | `{ skipped }` | 慢消费者丢事件提示（REST journal 兜底对账） |
| `error` | `{ message }` | 客户端消息错误回执 |

**客户端 → 服务端**（`WsIn`）：

| type | 载荷 | 说明 |
|---|---|---|
| `decide` | `{ decisions: [PlayerDecision], batch_id? }` | 提交决策（batch_id 缺省/不匹配 → error） |
| `advance` | `{ months?: 1..=240 }` | 请求推进（Human 策略下进度经 step 事件回报） |
| `ping` | — | 应用层心跳 → 服务端回 `{"type":"pong"}` |
| `pong` | — | 回应服务端 ping（任何 pong 都证明链路存活） |

---

## 7. 核心 DTO 速查

### StepSummary
```jsonc
{ "game_id": 7, "month": 6, "date": "2026-07-01", "day": null,
  "journal_added": 42, "decisions_made": 2, "season_end": false, "season_index": 1 }
```

### PendingBatch
```jsonc
{ "batch_id": 3, "date": "2026-06-30", "points": [DecisionPoint] }
```

### DecisionPoint（7 种，`SCREAMING_SNAKE_CASE` tag）
`TRANSFER_WINDOW`（offers）/ `TRAINING_FOCUS`（options）/ `MATCH_INTERVENTION`（图间暂停）/ `TEAMMATE_BLUNDER`（队友失误反应）/ `INJURY_DECISION`（带伤上阵/休养）/ `SPONSORSHIP_OFFER`（代言）/ `LIFE_EVENT`（场外人生：私下接触/假赛/宫斗/采访/队友邀请，含 `contact_team_id`）。
每个决策点携带 `id`（确定性生成）、`date`、`player_id`（稳定 ID）、`player_name`（展示）。

### PlayerDecision
```jsonc
{ "point_id": "2026-06-08|transfer|0", "option_id": "..." }
```

### WorldEvent（journal 元素，14 种）
`MATCH_PLAYED` / `CHAMPIONSHIP` / `TOURNAMENT_STAGE` / `TOURNAMENT_CANCELLED` / `TRANSFER_DONE` / `RETIREMENT` / `ROOKIE_INTAKE` / `INJURY_OCCURRED` / `INJURY_RECOVERED` / `CONFLICT` / `DECISION_MADE` / `TRAINING_DONE` / `HONOUR_AWARDED` / `LIVE_UPDATE`。
全部携带 `date` + `seq`（全局递增游标）+ 稳定 ID（player_id/team_id）。

### GameState 字段（v9）
`version / month / last_contract_year / locks / sim_year|month|day / rng_state[4] / decisions[] / journal[] / archive{} / world / vrs / events[] / yearly_rating / top20_history[] / scheduled_records[] / sim_version`

---

## 8. 前端对接注意事项（契约侧）

1. **别高频拉 `/state`**：轮询用 `/view` + `/journal?since=` + WS
2. **读档即迁移**：POST 旧版 GameState 到 `/load` 自动升到 v9；响应不含新档，需重新 GET
3. **Human 推进是异步的**：`/advance` 可能 202 + 后续 WS `step` 事件；等待决策期间 `advancing` 保持 true
4. **batch_id 幂等**：决策提交带批次号，重复提交 409；前端应忽略已处理过的 batch_id
5. **错误永远是人话 JSON**：`{ "error": "..." }`，无需二次解析即可展示
6. **LIVE 覆盖语义**：重复开同一场 = 重来（旧进度与 watched 丢弃）；历史回放走 `/replay`
7. **WS `notice` 可忽略**：系统提示不破坏协议；确定性复现走 journal（同种子同决策日志一致）
8. **比赛日闸门**：Human 策略下任何推进（含日级）在今日有 pending LIVE 对阵时 409，须先进入 LIVE 或 `live/today/skip`
