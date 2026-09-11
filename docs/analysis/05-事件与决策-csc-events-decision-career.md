# 05 · 事件与决策：csc-events / csc-decision / csc-career

> 事件流（WorldJournal 事件溯源）+ 决策流（DecisionPoint/DecisionSource/DecisionLog）+ 生涯档案。
> 三者共同支撑「种子 + 决策日志 = 完全一致的世界」的可复现性契约。

---

## 一、csc-events（世界事件流）

### 1. Enum：`WorldEvent`（src/event.rs）

全部为纯值，**serde 序列化，`rename_all = "SCREAMING_SNAKE_CASE"`**；
每个事件除展示字段外一律携带稳定 ID（PlayerId/TeamId）。14 个变体：

| 变体 | 关键字段（除 date/seq 外） | 含义 |
|---|---|---|
| `MatchPlayed` | `event_name/tier/winner/winner_id/loser/loser_id/score/maps` | 一场系列赛 |
| `Championship` | `event_name/tier/champion/champion_id/mvp/mvp_id` | 赛事冠军（含 MVP） |
| `TournamentStage` | `event_name/stage/detail` | 阶段推进（Major 里程碑） |
| `TournamentCancelled` | `event_name/tier/reason` | 赛事取消（参赛不足） |
| `TransferDone` | `player_name/player_id/from_team/from_team_id/to_team/to_team_id/fee` | 转会完成 |
| `Retirement` | `player_name/player_id/age/from_team/from_team_id` | 退役 |
| `RookieIntake` | `player_name/player_id/age/into_team/into_team_id` | 新秀入世 |
| `InjuryOccurred` | `player_name/player_id/kind/severity` | 受伤 |
| `InjuryRecovered` | `player_name/player_id/kind` | 伤病痊愈 |
| `Conflict` | `player_name/player_id/teammate_name/teammate_id/severity` | 队内冲突 |
| `DecisionMade` | `player_name/player_id/point_id/option_id` | 玩家决策（决策日志的事件镜像） |
| `TrainingDone` | `player_name/player_id/focus` | 训练计划执行完成 |
| `HonourAwarded` | `player_name/player_id/honour` | 荣誉颁发 |
| `LiveUpdate` | `headline/detail` | 按天推进的直播消息 |

辅助：`WorldEventKind`（14 变体的轻量分类枚举）、`WorldEvent::kind()`。

### 2. Struct：`WorldJournal`（src/journal.rs）

| 方法 | 签名 | 说明 |
|---|---|---|
| `record` | `pub fn record(&mut self, event: WorldEvent)` | 记录事件（盖章 seq） |
| `all` | `pub fn all(&self) -> Vec<WorldEvent>` | 全量（存档） |
| `len` | `pub fn len(&self) -> usize` | 事件数 |
| `is_empty` | `pub fn is_empty(&self) -> bool` | 判空 |
| `cursor` | `pub fn cursor(&self) -> i32` | 当前游标（增量拉取） |
| `since` | `pub fn since(&self, seq: i32) -> Vec<WorldEvent>` | 增量拉取（REST since=） |
| `of` | `pub fn of(&self, player_name: &str) -> Vec<WorldEvent>` | 按选手名查 |
| `of_player` | `pub fn of_player(&self, player_id: PlayerId) -> Vec<WorldEvent>` | 按选手 ID 查 |
| `restore` | `pub fn restore(&mut self, events: Vec<WorldEvent>)` | 读档恢复 |

### 3. 投影：`ProjectionContext` / `JournalChannel` / `JournalProjection`（src/projection.rs）

双频道事件投影（R2/M11）：`career_feed`（主角相关）/ `world_wire`（世界新闻）。
- `JournalTier { S, A, B, C }`（`#[serde(rename_all="UPPERCASE")]`）；
- `JournalChannel { CareerFeed, WorldWire }`；
- `ProjectionContext { protagonist_id: Option<PlayerId>, protagonist_team_id: Option<TeamId> }`
  （`Copy`/`Default`；**不含** journal 引用，按值传入）；
- `JournalProjection { tier, channel, visible }`——`project(event, context) -> JournalProjection`
  （纯函数，返回投影元数据），`visible_in(channel)` 判定频道可见性，
  `projected_json(event, context) -> Option<serde_json::Value>` 输出序列化投影。

> 前端按频道独立游标循环拉取（store 里 `journalSeqCareer` / `journalSeqWorld` 双游标）。

### 4. 叙事：`narrator.rs`

叙事文案装配（事件 → 文案渲染），消费 `csc-text::TextBundle`。

---

## 二、csc-decision（决策流）

### 1. Enum：`DecisionPoint`（src/point.rs）

决策点（7 变体），全部携带 `id`（确定性生成：`日期|类型|PlayerId`）与稳定 ID：

| 变体 | 关键字段 | 含义 |
|---|---|---|
| `TransferWindow` | `offers: Vec<TransferOffer>` | 转会窗候选（选项 = 队签名 / "STAY"） |
| `TrainingFocus` | `options: Vec<TrainingOption>` | 训练重点（月度批次；2026 已改主动触发，保留兼容） |
| `MatchIntervention` | `event_name/map_number/series_score/options` | 场内干预（图间暂停；仅玩家队在场比赛） |
| `TeammateBlunder` | `event_name/map_number/teammate_id/teammate_role/blunder/detail` | 队友失误反应 |
| `InjuryDecision` | `injury/options` | 带伤上场 / 休养 |
| `SponsorshipOffer` | `brand/annual_value/years/options` | 代言邀约 |
| `LifeEvent` | `kind: LifeEventKind/options` | 场外人生事件（见 07 篇） |

> 赛季目标（SeasonGoal）**不是**决策点变体——经独立端点 `POST /season-goal` 主动设置。

### 2. Struct：`PlayerDecision`（src/point.rs）

`{ point_id: String, option_id: String }`；`PlayerDecision::new(point_id, option_id)`。

### 3. Trait：`DecisionSource`（src/source.rs）

```rust
pub trait DecisionSource {
    fn decide(&mut self, points: &[DecisionPoint]) -> Vec<PlayerDecision>;
}
```

**实现**：

| 实现 | 说明 |
|---|---|
| `AutoDecisionSource` | 权威默认决策（**唯一事实源**——CLI/集成测试/前端 TS 的默认决策都须与之对齐）；`default_decision_for(point) -> PlayerDecision` |
| `CareerDecisionSource` | 赛季内自动处理（FIFA 式推进用，稳妥选项） |
| `StdinDecisionSource`（csc-app） | CLI 交互（渲染 + stdin） |
| `ChannelDecisionSource`（csc-server） | 经 mpsc channel 等待前端提交（game/mod.rs:382；**不叫 HumanDecisionSource**） |

### 4. Struct：`DecisionLog`（src/log.rs）

`record(decision)` / `entries() -> Vec<PlayerDecision>` / `count()` / `restore(entries)`。
每条决策同时镜像为 `WorldEvent::DecisionMade` 事件。

### 5. Struct：`DecisionRecorder`（src/recorder.rs）

决策记录器：把 `DecisionSource` 的返回值写入 `DecisionLog` + journal 镜像。

### 6. Struct：`TransferOffer` / `TransferTarget`（src/offer.rs）

- `TransferTarget { team_id, team_signature, ranking, power_delta }`——目标队情报；
- `TransferOffer { point_id, player_id, player_name, from_team_id, offers: Vec<TransferTarget> }`——转会候选。

### 7. Enum：`LifeEventKind`（src/point.rs）

`SecretTeamContact / MatchFixerContact / TeammatePowerStruggle / TeamContact / Interview / TeammateInvite`。
`name() -> &'static str`。配套 `LifeOption { id, label, description }`。

---

## 三、csc-career（生涯档案）

### 1. Struct：`CareerArchive`（src/archive.rs）

按 PlayerId 键控的逐赛季记录：

| 方法 | 签名 | 说明 |
|---|---|---|
| `record_season` | `pub fn record_season(&mut self, player_id, record: SeasonRecord)` | 追加赛季记录 |
| `seasons_of` | `pub fn seasons_of(&self, player_id) -> Vec<SeasonRecord>` | 查选手赛季（**克隆返回**，archive.rs:82） |
| `totals_of` | `pub fn totals_of(&self, player_id) -> Option<SeasonTotals>` | 生涯合计 |
| `snapshot_map` | `pub fn snapshot_map(&self) -> HashMap<PlayerId, Vec<SeasonRecord>>` | 存档快照 |
| `restore` | `pub fn restore(&mut self, map)` | 读档 |

### 2. Struct：`SeasonRecord`

`{ player_id, player_name, year, team_name(Option), rating, maps_played, kills, deaths, assists, wins, earnings, honours, events_played, marks, injury_days, season_goal, goal_met, goal_outcome, ... }`
（archive.rs:16-55）——单赛季档案。**无 `tier` 字段；地图数为 `maps_played` 而非 `maps`**。

### 3. Struct：`SeasonTotals`

`{ seasons, total_kills, total_wins, total_earnings, peak_rating, honours }`——生涯合计。

### 4. Struct：`CareerEndingEvaluator`（src/career_ending.rs）

| 方法 | 签名 | 说明 |
|---|---|---|
| `evaluate` | `pub fn evaluate(player_id: PlayerId, name: &str, totals: &SeasonTotals, career: &CareerInfo, text: &TextBundle) -> CareerEnding` | 生涯结局评定（**PlayerId 按值**，career_ending.rs:89-95） |

### 5. Enum：`LegacyTier` + Struct：`CareerEnding`

- `LegacyTier`: **7 变体**（career_ending.rs:34-48）：`Rookie / Journeyman / Star / Legend / HallOfFame / EraIcon / Goat`；
- `CareerEnding { tier, goat_score: f64, verdict: String, … }`——结局（GOAT 评分跨选手比较）。

---

## 四、三者如何协同（可复现性闭环）

```
玩家意图 → DecisionSource::decide → DecisionRecorder
              ├→ DecisionLog（存档 decisions）
              └→ WorldJournal（WorldEvent::DecisionMade 镜像）
世界推进 → 引擎 RNG（Xoshiro256StarStar，存档 rng_state）
              ├→ 赛事/人口/伤病/... 全部随机
              └→ WorldJournal（MatchPlayed/TransferDone/...）
存档 GameState = { rng_state + decisions + journal + world + vrs + events + ... }
回放/校验：同 seed + 同 decisions → 同 RNG 消费序 → 同事件流（指纹门禁锁定）
```
