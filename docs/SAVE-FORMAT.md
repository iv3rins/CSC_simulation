# SAVE-FORMAT —— 存档格式契约（版本演进集中记录）

> 本文档是 `GameState` 存档格式的唯一权威演进记录（bump 义务见 §3 强制条款）。
> 代码权威：`backend/crates/csc-core/src/state.rs`（结构定义 / `CURRENT_VERSION` / 迁移）；
> 持久化信封：`backend/crates/csc-server/src/game/（types.rs 定义 PersistedGame、persist.rs 写盘、manager.rs 恢复）`。
> 与 `docs/ORDERS-BD.md`（域 B/D 派发指令）联动；指纹门禁见 §4。

---

## 1. 存档包格式（外层信封）

- 磁盘包 = `gzip(JSON(PersistedGame envelope))`（`game/persist.rs::write_persisted_package`：先写 `*.tmp` 再 `rename` 原子替换）。
- envelope v3 = `{ format: "csc-persisted-game", format_version: 3, policy, story, career_session, state_crc32, state }`（`game/types.rs::PersistedGame`，`deny_unknown_fields`）。`career_session` 保存传输 generation 与完整场景回执（request_id/scene_instance_id/choice_id/outcome），不参与模拟 RNG/指纹。
- `state_crc32 = crc32fast::hash(canonical_json_bytes(state))`——CRC 作用于**键排序规范化**后的 state JSON（`game/types.rs::canonical_state_bytes`，递归键字典序消除 HashMap 迭代序噪声；Vec 顺序是语义，不重排）。
- v1 → v2 升级：v1 包无 CRC 字段（`state_crc32` 带 `serde(default)`），恢复端接受并升级——内存恢复后立即把迁移后的最新状态重写为 v2 包（`game/manager.rs::restore_from_disk`）。
- 恢复支持 envelope 1/2/3；旧包 `story=false`、缺失 career_session 创建传输身份。未知版本拒绝。
- CRC 校验失败 → 包改名 `*.corrupt` 隔离，返回错误（不静默吞坏档）。
- GameState 内部：未知版本（> `CURRENT_VERSION`）拒绝加载；未知字段拒绝解析（`deny_unknown_fields`——格式漂移静默默认化是存档损坏的温床）。

## 2. GameState 版本表（v1–v11）

> 每行：版本号 / 日期 / 字段级变更 / 迁移策略 / 是否 bump 指纹版本标签（`WORLD_SIM_VERSION`，B-1 引入，v8 及以前不存在该标签，标「—」）。
> v1 为「初版全量字段」定义（对照 `state.rs` 当前结构减去后续版本新增项；v1 定义以代码注释为准）。

| 版本 | 日期（2026） | 字段级变更 | 迁移策略 | bump 指纹版本标签 |
|---|---|---|---|---|
| v1 | 初版 | 初版全量字段（version/month/last_contract_year/locks/sim_year/sim_month/sim_day/rng_state/decisions/journal/archive/world/vrs/events/yearly_rating 等；v1 定义以代码注释为准） | — | — |
| v2 | 2026 | `top20_history` 新增（历届年度 TOP20 榜单快照） | `serde(default)`：v1 旧档读入补空列表 | — |
| v3 | 2026 | `pending_training`（主动训练）引入（历史格式） | `serde(default)` | — |
| v4 | 2026 | `CareerInfo.transfer_contacts`（转会市场主动接触/队伍招募情报） | `serde(default)`：旧档读入自动补空 | — |
| v5 | 2026 | `YearlyRatingTracker.honor_points` 量纲校准：MVP=10/EVP=5 绝对分 → Rating 等值单位（MVP=1.0/EVP≤0.5） | 显式迁移：读入 v1–v4 旧档时 `rescale_honour_points(0.1)`（×0.1 归一） | — |
| v6 | 2026 | `CareerInfo.season_goal / season_goal_year`（赛季目标） | `serde(default)`：旧档读入补 None/0 | — |
| v7 | 2026 | `PlayerFinance.skins / last_invest_year`（资金运用） | `serde(default)`：旧档读入补空/0 | — |
| v8 | 2026 | `CareerInfo.match_style / public_stance`（主动操作） | `serde(default)`：旧档读入补 None | — |
| v9 | 2026 | `scheduled_records`（future 赛程权威记录）+ `sim_version`（世界级模拟器指纹版本标签，B-1） | `serde(default)`：旧档读入补空/1；迁移 = 无操作（两个字段此前已随代码落地、只是版本号未 bump） | 否 |
| v10 | 09-10/11 工作树草稿 | `narrative` 进度和 `execution` 草稿 | 旧档补空进度/Idle；该版 Awaiting 缺少真实 continuation，不能据此恢复世界步骤 | 否 |
| v11 | 09-11 | `NarrativeProgress.next_scenes / closed_arcs / choice_history` | v10 Idle 补空字段；`GameState::validate_execution` 和 Engine 读档拒绝非 Idle 执行草稿，保留当前局不变 | 否 |

### v11 的明确兼容边界

- v1–v9 无剧情进度可加载；既有量纲迁移继续执行。不凭旧数据补造剧情或比赛参与史。
- v10 Idle、空剧情可升级；已开始剧情必须装配相同内容版本。当前资产内容版本 2，v1 已活动/完成剧情没有语义迁移，因此拒绝用 v2 正文覆盖。
- `ExecutionState::AwaitingDecision` 仅保留旧 serde 形状用于给出清晰错误；缺少月/日/地图 continuation，一律拒绝恢复。不会月初重放、重复抽候选或默默清成 Idle。
- 当前真实可保存的故事停点是 `narrative.active_scene`，`execution=Idle`；没有开放故事世界/比赛推进。`PendingWorldBatch` 可序列化但尚未接入 GameState 的完整执行链，v11 **不代表 T02–T05 全部完成**。
- envelope v3 场景回执与状态在同一 gzip 文件替换边界提交。CRC 仍只校验 canonical GameState；外层传输字段由 gzip 完整性/JSON shape 检查保护，CRC 不覆盖这些字段。

迁移实现（`state.rs::migrate_state`）：`version < CURRENT_VERSION` 时执行逐级迁移（当前仅 v5 有显式动作：`rescale_honour_points(0.1)`），其余新字段由 `serde(default)` 补默认值，最后 `version = CURRENT_VERSION`。v8→v9 迁移 = 无操作（`scheduled_records` / `sim_version` 均为 `serde(default)`，旧档读入自动补空/1）。

## 3. bump 规则（强制条款）

1. 每次 `GameState::CURRENT_VERSION` 变更，**必须**在本文件追加一行（版本号 / 日期 / 字段级 diff / 迁移策略 / 是否 bump `WORLD_SIM_VERSION`）。
2. PR 评审检查本文件与 `state.rs` 同步；**未同步即打回**。
3. 字段语义变更（不只增删字段）同样必须记录（如 v5 量纲校准）。
4. `WORLD_SIM_VERSION` bump 与格式版本 bump 是**两回事**：前者表示世界级模拟语义/RNG 消费序变化（见 §4），存档格式可能不变；后者表示存档可解析性变化。两者独立记录。

## 4. 与指纹门禁的联动（B-1 / G3 / S3）

- `GameState.sim_version`（= `WORLD_SIM_VERSION`，B-1 引入；`serde(default)`，旧档读入补 1，不破坏向后兼容；`Engine::snapshot` 写入、`Engine::restore` 忽略）。
- 世界指纹 = `FNV-1a64(sim_version.to_le_bytes() ‖ canonical_json_bytes(state))`（门禁：`backend/crates/csc-core/tests/fingerprint.rs`，随 `cargo test --workspace` 默认执行，禁止 `#[ignore]`）。
- `WORLD_SIM_VERSION` bump → 指纹漂移 = **预期行为**，必须在 bump 提交说明 + 本文件记录。
- `Engine::restore` 忽略 `sim_version`（恢复宽容、门禁严格——指纹比对才是消费方）。
- 语义：任何改变世界级模拟语义/RNG 消费序的改动（确定性修复、文案池增删、阶段管线重排）都必须 bump `WORLD_SIM_VERSION`。

## 5. 已知残留 / 格式变更记录

> 承接域 B（ORDERS-BD §2）的格式变更与已知残留。改动提交时必须回填本节。

### 5.1 M1（ORDERS-BD §2.4）：决策点 ID 由「名字键控」改「PlayerId 键控」

- 5 类决策点 ID 格式：`日期|transfer|{player_id}`、`日期|injury|{player_id}`、`日期|sponsor|{player_id}`、`日期|life|{player_id}|{kind}|{actor_key}`、`日期|blunder|{event_name}|{map_number}|{teammate_id}`（旧格式用玩家名/队名/固定文案串）。
- 影响：旧存档 `decisions: Vec<PlayerDecision>` 里的旧 point_id 与重放时的新格式 `find(|d| d.point_id == point.id())` 失配——当前无跨版本决策重放协议，影响面 = 同一存档内历史决策日志展示（batch.rs 匹配逻辑不改）。
- actor_key 约定：`TeammateInvite | TeammatePowerStruggle` → 队友 `PlayerId` 字符串；`SecretTeamContact | TeamContact` → `T:{team_id}` 稳定 ID 字符串（TeamId 化已于 P2-4 落地，见 5.2）；`Interview | MatchFixerContact` → `A:` 前缀固定文案串。**actor_key 不得含 `|`（队名资产契约）**。

### 5.2 已落地：life_events 的 TeamId 化（P2-4，2026-08 收尾）

- **改动**：`SecretTeamContact / TeamContact` 的 actor 键从队名串升级为稳定 `TeamId`（`T:{team_id}`，删除队名串键）。`DecisionPoint::LifeEvent` 新增 `contact_team_id: Option<TeamId>` 字段（`#[serde(default)]`——旧档读入自动补 `None`，**存档格式版本不 bump**，向后兼容）。
- **事件生成语义（修订 · 2026-09-10，依据 22 §4 裁决）**：接触方**优先**从 `CareerInfo.transfer_contacts`（已含 `team_id`）反查；`transfer_contacts` 为空时**恢复 VRS 排名兜底**（经 `world.team_by_name` 由队名反查稳定 `TeamId`）——R1 返工要求：兜底分支必须保留，否则无 contacts 的主角 `kinds` 长度不同 → `rng.next_i32_bound(kinds.len())` 消费分叉 → 世界轨迹漂移（RNG 消费序稳定性契约）。只有 VRS 也查不到接触方时才 `kinds.retain` 剔除 TeamContact/SecretTeamContact。**旧版本文档「为空时不再生成接触事件」的描述已作废**（life_events.rs:88-124 现行实现与 07 号文档一致）。
- **决策日志影响**：point_id 格式变更（`T:队名` → `T:TeamId`）属决策日志 ID 格式变更（§3.3 强制记录）。旧档决策日志中旧格式 point_id 无法与新格式重放匹配——与 M1 同款影响面（历史决策日志展示），无迁移。
- **确定性**：TeamId 化只改 key 构造，不改变 RNG 消费序（取值逻辑同序）→ **`WORLD_SIM_VERSION` 不 bump**，指纹门禁 3/3 验证通过（`cargo test -p csc-core fingerprint` 全绿）。
- 5.1 的 actor_key 约定同步更新：`SecretTeamContact | TeamContact` → `T:{team_id}`（TeamId Display，csc-util/id.rs）。

### 5.3 M4/M5 预期漂移（G3 基线建立前的顺序内漂移）

- **M4（growth.rs `roll_bp` 整数化）**：裸浮点分支 `next_double() < p` → 整数化 BP 掷点（`roll_bp(p)`）。RNG 消费序/比较语义改变，同种子重放结果与旧版本不同——属 G3 指纹基线建立前的顺序内漂移（ORDERS-BD §0.5 顺序保证），指纹基线以 M4/M5 之后的世界为锚。
- **M5（generator.rs 收敛 `csc_util::gaussian`）**：删除局部 Box-Muller 实现，统一走 `csc_util::gaussian`（64 次逐位对拍锁定）。新秀生成数值与旧版不同——同上，属基线建立前的预期漂移，不 bump `WORLD_SIM_VERSION`（基线建立后才开始版本敏感）。

---

*维护：本文档由域 D 实现（flash-dev-4）依 ORDERS-BD §3.1 建立；后续版本演进由 §3 强制条款约束。*
