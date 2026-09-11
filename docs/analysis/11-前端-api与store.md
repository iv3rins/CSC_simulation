# 11 · 前端：API 客户端 / 类型契约 / WebSocket / 状态管理

> 前端 = TypeScript + Solid（纯表现层）。唯一服务端协议面在 `src/api/`；
> 全局状态机在 `src/store/game.tsx`；类型契约全量在 `src/api/types.ts`（1332 行）。

---

## 一、REST 客户端：`api`（src/api/client.ts）

### Struct：`ApiError`

`{ status: number, message: string }`——服务端 `{ error }` 文本透传；status 0 = 网络不通。

### 请求辅助

| 函数 | 说明 |
|---|---|
| `request<T>(path, init?)` | JSON 请求（错误统一抛 ApiError） |
| `requestRaw(path, init?)` | 原始响应（gzip 二进制存档） |
| `requestOnce<T>(path)` | 会话级一次性缓存（静态资产：reference/calibration） |

### 方法清单（`api` 对象）

| 方法 | 请求 | 说明 |
|---|---|---|
| `health()` | GET /health | 健康检查 |
| `createGame(body)` | POST /games | 创建游戏 |
| `shutdownGame(id)` | DELETE /games/{id} | 关闭 |
| `state(id)` | GET /games/{id}/state | 完整快照 |
| `view(id)` | GET /games/{id}/view | **轻量视图（高频刷新入口）** |
| `calendar(id, year)` | GET /games/{id}/calendar?year= | 年度赛历 |
| `news(id)` | GET /games/{id}/news | 世界新闻 |
| `replay(id, event, date, index)` | GET /replay | 回放懒加载 |
| `summary(id)` | GET /summary | 世界概览 |
| `journal(id, since, channel?, limit?)` | GET /journal | 增量事件（分页 + 频道） |
| `startLive / live / advanceLive / decideLive / skipLive / liveReview / liveWatch(id, matchId, ...)` | /live 系列 | LIVE 会话 |
| `eventAggregate(id, eventName)` | GET /events/{name} | 赛事聚合 |
| `fixtureWatched(id, body)` | POST /fixtures/watched | 标记已看完 |
| `liveToday(id)` | GET /live/today | 今日 LIVE 闸门 |
| `archive(id)` / `ending(id)` | GET | 档案/结局 |
| `top20(id)` / `top20Reference(id)` | GET | TOP20（**无 `top20History` 方法**——历史榜单在 top20 响应内） |
| `world(id)` / `insights(id)` / `calibration(id)` | GET | 世界故事/洞察/校准 |
| `save(id)` / `saveGzip(id)` | GET /save(/gzip) | 存档（gzip 返回 Blob + crc32 + 字节数） |
| `load(id, state)` / `loadGzip(id, bytes, crc32?)` | POST /load(/gzip) | 读档 |
| `advance(id, months)` / `advanceDays(id, days)` | POST /advance | 推进 |
| `advanceSeason(id)` | POST /advance-season | 赛季推进（FIFA 主入口） |
| `transferOffers(id)` / `transferMarket(id)` / `signTransfer(id, sig)` | /transfers/* | 转会 |
| `statement(id, tone)` / `matchPlan(id, style)` | /actions/* | 发言/BP |
| `pending(id)` / `submitDecisions(id, decisions, batchId)` | /decisions | 决策 |
| `trainingOptions(id)` / `planTraining(id, focus)` | /training | 训练 |
| `setSeasonGoal(id, goal)` | POST /season-goal | 赛季目标 |
| `finance(id, action, param?)` | POST /finance | 资金运用 |

---

## 二、类型契约（src/api/types.ts，1332 行）

### 基础类型（type alias）

| 类型 | 值 |
|---|---|
| `PlayerId` / `TeamId` | `number`（u32 newtype 的裸数序列化） |
| `Policy` | `"auto" \| "human"` |
| `Role` | `"IGL" \| "AWP" \| "RIFLER" \| "ENTRY" \| "SUPPORT" \| "LURKER"` |
| `TourneyTier` | `"MAJOR" \| "SUPERELITE" \| "ELITE" \| "T1" \| "T2" \| "QUALIFY"` |
| `JournalTier` | `"S" \| "A" \| "B" \| "C"` |
| `JournalChannel` | `"career_feed" \| "world_wire"` |
| `InjuryKind` | `"WRIST" \| "FINGER" \| "SHOULDER" \| "BACK" \| "LEG" \| "EYE_STRAIN" \| "ILLNESS"` |
| `InjurySeverity` | `"MINOR" \| "MODERATE" \| "SEVERE"` |
| `Playstyle` | `"AGGRESSIVE" \| "BALANCED" \| "CONSERVATIVE"` |
| `BlunderKind` | `"CHOKE" \| "MISCARRY" \| "TILT" \| "LACK_OF_FOCUS"` |
| `TrainingFocus` | `"AIM" \| "UTILITY" \| "CLUTCH" \| "PHYSICAL" \| "MENTAL" \| "COMMUNICATION" \| "REST"` |
| `SeriesStage` | `"GROUP" \| "PLAYOFF" \| "QUARTERFINAL" \| "SEMIFINAL" \| "FINAL" \| "UNKNOWN"` |
| `EventImportance` | `"BACKGROUND" \| "IMPORTANT" \| "MAJOR" \| "CHAMPIONSHIP"` |
| `LiveCriticalReason` | `"match_point" \| "late_close_score" \| "economy_clash"` |

### 核心接口（interface）

| 接口 | 对应后端 | 要点 |
|---|---|---|
| `BaseAttributes / SkillAttributes / ProAttributes / WeaponAttributes` | csc-entities | 属性四组 |
| `PlayerCharacter` | World 选手 | 全字段 |
| `Team` | World 队伍 | 含 chemistry/budget/roster_ids |
| `World` | World arena | 全量 |
| `GameState` | csc-core GameState | 存档聚合根（15 字段） |
| `ClientState` | ClientState | 轻量视图 |
| `ClientWorld / ClientSeason / ClientTournamentResult / ClientBracketSlot / MajorPresentation / CareerMilestone` | client.rs 投影 | 视图值对象 |
| `CalendarResponse / PlayerCalendarEntry / ClientFixture / ClientFixtureScore` | calendar | 赛历 |
| `EventAggregate / EventTeam / EventChampion` | client.rs R2 | 赛事聚合 |
| `WorldEvent`（union）+ 各 `*Data` 接口（14 个） | csc-events | 事件 |
| `DecisionPoint`（union）+ `DecisionKind` + `DecisionData` + 各 `*Data`（8 个） | csc-decision | 决策 |
| `PlayerDecision / PendingBatch / StepSummary / SeasonAdvanceResponse` | 协议 | 决策/推进 |
| `SeriesResult / MapScore / PlayerLine / MatchOutcomeAnalysis / LiveMatchState` | csc-simulation | 比赛 |
| `LiveMapConfig / LiveRoundState / LiveRoundEvent / LiveKillEvent / LiveSessionView / LiveAdvanceView / LiveReview / LiveDecisionRequest / LiveTodayMatch / LiveTodayResponse` | live | LIVE |
| `TransferEvent / TransferOffer / TransferTarget / TransferMarket` | 转会 | — |
| `TrainingOption / InterventionOption / InjuryOption / SponsorOption / LifeOption` | 决策选项 | — |
| `SeasonRecord / CareerMemory / CareerInfo / PlayerFinance / Sponsorship / Honours / TeamContact` | 生涯 | — |
| `GameSummary / InsightsResponse / NewsResponse / Top20Response / Top20ReferenceResponse / WorldStoriesResponse / CalibrationResponse / ArchiveResponse / CareerEndingResponse / CareerEndingPending / FinanceResponse` | 查询端点 | 各响应信封 |
| `JournalProjection / JournalResponse` | journal | 双频道 |
| `WsEvent / WsIn` | WS 协议 | 推送/指令 |

---

## 三、WebSocket：`GameSocket`（src/api/ws.ts）

### Enum：`WsStatus`

`"disconnected" \| "connecting" \| "connected"`。

### Interface：`WsHandlers`（ws.ts:15-21）

`{ onDecisions?(batch), onStep?(summary), onJournal?(events), onError?(err), onStatus?(status) }`
——**无 onOpen/onClose/onPing**（连接状态经 onStatus 回调；ping/pong 在内部处理；onError 处理 notice/lagged/error，ws.ts:176-185）。

### Class：`GameSocket`

| 方法 | 签名 | 说明 |
|---|---|---|
| `connect` | `(gameId, handlers) => void` | 连接 WS |
| `sendDecide` | `(batchId, decisions) => void` | 提交决策 |
| `sendAdvance` | `(months?) => void` | 请求推进 |
| `close` | `() => void` | 断开 |
| `status` | `WsStatus` | 状态 |

心跳：30s ping/pong 判死 → close → store 自动重连（REST 兜底拉 journal）。

---

## 四、全局状态：`GameContextValue`（src/store/game.tsx）

Solid context + store（createStore），**前端唯一的服务端协议消费入口**。

### Store 字段（关键）

| 字段 | 类型 | 说明 |
|---|---|---|
| `gameId / policy / wsStatus / serverUp` | — | 会话基础 |
| `state` | `ClientState \| null` | 轻量视图（step 后刷新） |
| `journal` / `journalSeq` | `WorldEvent[]` / `number` | 事件流（增量维护） |
| `journalSeqCareer` / `journalSeqWorld` | `number` | **R2 双频道独立游标** |
| `advancing / syncStuck / seasonAdvancing / advanceAnchor` | — | 推进状态机 |
| `queue / current` | `PendingBatch[]` / `PendingBatch \| null` | 决策批次队列（FIFO 弹窗） |
| `transferMarket / transferBatch` | — | 转会窗（t6 非弹窗） |
| `autoRun / autoRunDelay / autoDecisionPolicy / autoDecisionStyle / autoHandledKinds` | — | 自动推进偏好 |
| `liveMatch / liveSession / liveToday / livePrompt` | — | LIVE 状态 |
| `seasonRecap / celebration / toast` | — | 弹层 |
| `calendarCache` | `Map<string, CalendarResponse>` | **t1 新增**：年度赛历缓存（key=`gameId:year`，见 `getCalendar`） |

### 方法（GameContextValue）

| 方法 | 说明 |
|---|---|
| `boot()` | 会话启动（读 localStorage game_id → attach） |
| `createGame(opts)` | 创建 + 连接 |
| `attach(gameId, policy)` | 挂载既有局 |
| `refresh()` | 拉 view + journal 兜底（**t1 节流**：同 (gameId, 日期) 500ms 窗口合并，leading edge 执行、in-flight 共享，完成后 500ms 新鲜短路；boot 全量跳过） |
| `getCalendar(year)` | **t1 新增**：年度赛历（会话级 Map 缓存 + in-flight 去重，同 (gameId, year) 只请求一次） |
| `advance(months)` / `advanceDays(days)` / `advanceSeason()` | 推进 |
| `submitDecisions(decisions, source?)` | 提交决策（player/auto） |
| `refreshTransferMarket()` / `submitTransfer(optionId)` / `dismissTransfer()` | 转会窗（**deadline 由后端 `month_end_label` 计算**，前端不再本地 monthEndOf——2026-09 重构） |
| `setSeasonGoal(goal)` | 赛季目标 |
| `autoDecideCurrent()` / `startAutoRun(delayMs)` / `stopAutoRun()` | 自动决策 |
| `openBroadcast(req)` / `closeBroadcast()` | 直播回放弹层 |
| `openLiveSession(req)` / `closeLiveSession()` | LIVE 会话 |
| `handleLiveToday(match)` / `dismissLivePrompt()` | LIVE 闸门 |
| `submitLiveDecision(decision)` / `skipLiveToday()` | LIVE 决策 |

### 关键业务规则（与后端 Human 协议对齐）

1. 点击「推进」≠ 立刻跳月：advance 只是请求时间前进，节奏由后端驱动；
2. 一个月有多个决策批次（每图干预 + 月度批次），每个 decisions 事件都必须回一次 decide；
3. 批次以队列承载，模态框逐批弹出；
4. WS 断线重连后：增量 journal 由完整快照刷新兜底，遗漏批次由 GET /decisions/pending 拉回；
5. 自动决策策略：`off`（全部暂停）/ `minor-only`（仅自动 P2）/ `all`（全部自动）；
   `autoHandledKinds` 允许玩家明确「本类后续自动」。

---

## 五、lib 工具层（src/lib/）

| 文件 | 导出 | 说明 |
|---|---|---|
| `decide.ts` | `defaultDecision / styleDecision / batchSeverity` | 默认决策（与后端 AutoDecisionSource 对齐；M10 收敛后唯一 TS 副本） |
| `journal.ts` | `mergeJournalEvents` | 事件合并（seq 去重） |
| `selectors.ts` | `protagonist` | 主角选择器 |
| `event.ts` | `eventKind / eventData / decisionKind / decisionData` | 事件/决策判别 |
| `format.ts` | 格式化 + 标签（`PLAYSTYLE_LABELS` 不存在——C5 已核销） | 数值/日期/标签 |
| `crc32.ts` | `crc32Hex` | 存档完整性校验 |
| `date.ts` | 日期工具 | — |
| `storage.ts` | `readAutoDecisionPolicy / persistLiveSession / journal 游标` | localStorage |
| `telemetry.ts` | `setTelemetryAdapter / track / telemetryDump` | ⚠️ 预留未接线（C4） |
| `teamLogo.ts` | `preloadTeamLogos` | 队标预加载 |
| `tournament.ts` / `matchReplay.ts` / `motion.ts` / `hltv.ts` / `describe.ts` | 各领域工具 | — |
| 测试 | `decide.test.ts / journal.test.ts / matchReplay.test.ts / crc32.test.ts` | vitest 32 测试 |

---

## 六、i18n

`src/i18n/index.ts`（`t()/tf()`）+ `zh-CN.json`——前端标签/事件文案；
与后端 `assets/text/zh-CN.json`（csc-text）分工：前端管 UI 标签，后端管叙事文案。
