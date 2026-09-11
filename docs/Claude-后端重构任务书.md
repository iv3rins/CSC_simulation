# Claude 后端重构任务书（可直接复制投喂）

> 用法：以下每个任务都是**独立批次**，一次只投喂一个。完成一个、跑绿门禁、再投下一个。
> 不要一次投喂多个任务——Claude 一次只该改一处，否则上下文爆炸、必返工。
> 每个任务末尾的「验收命令」是硬门槛：全绿才算完成，不绿就让 Claude 继续修到绿。
> 文档定位：docs/analysis/19-后端代码审查-2026-09-06.md（含全部问题位置与修复方向）。

---

## 任务 0 · P0-1 修复 CI 格式门禁（最简单，5 分钟）

任务：修复 csc-core/src/client/mod.rs 的 rustfmt 违规，让 CI 的 cargo fmt 门禁转绿。

文档定位：docs/analysis/19-后端代码审查-2026-09-06.md §P0-1
现状：csc-core/src/client/mod.rs 测试模块的 use 顺序未排序（约 L38 附近），
      `cargo fmt --all -- --check` 报 diff；该违规已在 HEAD（cce548b），CI 必红。
契约：执行 `cargo fmt --all` 自动修复，不要手改。
边界：只允许 rustfmt 自动格式化，禁止任何其他代码改动。
验收：`cd backend && cargo fmt --all -- --check` 退出码 0。
文档同步：无需（纯格式）。

---

## 任务 1 · P0-2 LiveSessionStore 加会话回收路径（修内存泄漏 + 无法重进同场 LIVE）

任务：给 LiveSessionStore 增加完整的会话生命周期回收，修复两个缺陷：
      (a) 每场进入过 LIVE 的比赛永久驻留内存（泄漏，只随进程重启释放）；
      (b) 同一场比赛一旦进入过 LIVE 就再也无法重新进入（start 对同 key 直接 Err）。

文档定位：docs/analysis/19-后端代码审查-2026-09-06.md §P0-2
现状（已核实到行）：
- backend/crates/csc-server/src/live.rs L117 `LiveSessionStore { sessions: Arc<Mutex<HashMap<LiveSessionKey, LiveSession>>> }`
- L122 `start()`：key 已存在时 `Err("LIVE 比赛 {match_id} 已存在")` —— 需改为幂等重建
- L147 `get` / L161 `advance` / L191 `decide` / L230 `skip` / L257 `mark_watched` / L270 `review`
- 全 store 无任何 end/remove/clear/retain 方法
- LiveSessionKey 定义在 live.rs L24（{ game_id: u64, match_id: String }）
- 会话"打完"判定：`LiveMatchEngine::is_finished(&session.state)`（csc-simulation/src/live_match.rs L264，pub）
- skip() L230 已演示"推进到打完"的循环写法（含 MAX_ROUNDS 硬上界）
- 消费方：backend/crates/csc-server/src/routes/live.rs（start_live/get_live/advance_live/decide_live/get_live_review/skip_live/mark_live_watched）
- 游戏销毁：backend/crates/csc-server/src/routes/lifecycle.rs L97 `shutdown_game` 只调 `state.games.shutdown(id)`，不碰 `state.live_sessions`
- LRU 淘汰：backend/crates/csc-server/src/game/manager.rs `persist_and_remove`（约 L636）只摘 games map，不碰 live_sessions

契约（按优先级实现）：
1. 新增 `pub fn end(&self, game_id: u64, match_id: &str)`：从 sessions 移除该 key（存在才移除，幂等）。
2. `start()` 改为幂等：key 已存在时**覆盖重建**（丢弃旧会话、插入新会话并返回新视图），
   使"比赛打完/页面误关/想重看一次"都能重新进入。注意：覆盖前应确保没有正被并发 advance 的会话——
   锁内操作天然安全（sessions 是 Mutex），直接在同一把锁内替换即可。
3. 比赛自然打完的路径自动回收：在 skip() 循环结束（is_finished 为真）且返回前调用 end 语义
   （可直接 remove；或抽内部 `fn remove_locked`）。注意 review() 在会话移除后仍要能工作——
   review 需要的是已结束会话的数据，若打完即删会导致 review 404。**先确认 review 的数据源**：
   若 review 依赖 sessions 里的 LiveSession，则打完不能立刻删，改为「打完置 finished 标记 +
   GameManager 淘汰/DELETE 时清理」；若 review 的数据已持久化在 session 外，则可打完即删。
   以源码实际为准，选择不破坏 review 的方案，并在交付说明里写清你的选择与理由。
4. `shutdown_game`（lifecycle.rs L97）在 games.shutdown(id) 成功后，调用 live_sessions 清理该
   game_id 全部会话（需新增 `pub fn end_all_for_game(&self, game_id: u64)`，retain 掉 game_id 匹配项）。
5. GameManager 的 LRU 淘汰路径同样在移除 game 时清理该 game_id 的 live 会话（同 end_all_for_game）。

边界：只允许改 backend/crates/csc-server/src/live.rs、routes/live.rs、routes/lifecycle.rs、
      game/manager.rs 四个文件；禁止改 csc-simulation 与其他 crate。若发现必须改 csc-simulation
      才能完成，停下来报告，不要擅自跨 crate。
验收：
  - `cd backend && cargo test -p csc-server --locked` 全绿（现有 + 新增）
  - `cargo fmt --all -- --check` 退出码 0；`cargo clippy -p csc-server --all-targets --locked -- -D warnings` 0 警告
  - 新增测试覆盖至少两条：① 同场 LIVE start 两次（第二次在第一次结束后）返回新会话而非 Err；
    ② shutdown_game / 淘汰后，该 game_id 的 live 会话全部清空（可查 store 内部计数或行为断言）。
文档同步：更新 docs/analysis/19-后端代码审查-2026-09-06.md 的 P0-2 条目，行首加「✅ 已闭环（2026-09-06）」。
交付：diff 摘要 + 你选择的「打完即删 vs 打完保留待清理」决策及理由 + 测试清单 + 验收命令输出。

---

## 任务 2 · P1-3 csc-wasm 读档补版本迁移（一行 + 测试）

任务：修复 csc-wasm 读档跳过 GameState 版本迁移的缺陷。

文档定位：docs/analysis/19-后端代码审查-2026-09-06.md §P1-3
现状（已核实）：
- backend/crates/csc-wasm/src/lib.rs L134 `csc_restore`：
  `match GameState::from_json_str(&json) { Ok(state) => { session.engine.restore(state); 0 } ... }`
  —— **没有调 migrate_state()**。
- 对照：服务端全部迁移——csc-server/src/routes/lifecycle.rs L294/L344 `.map(|snap| snap.migrate_state())`、
  csc-server/src/game/manager.rs L715 `persisted.state.migrate_state()`；csc-core/src/state.rs L144 也有
  `Ok(Self::from_json_str(json)?.migrate_state())` 的现成组合入口。
- 后果：v1–v4 旧档在 wasm 单机模式读入后 honor_points 仍是旧量纲（迁移应 ×0.1），TOP20 荣誉分
  整体错一个数量级；version 字段停留旧值，再存盘写出"版本号旧、内容已新引擎改过"的混合档。
契约：
  1. 首选：把 `from_json_str` 与 `migrate_state()` 的组合收敛为**唯一公开入口**（参考 state.rs L144
     的现成写法，若该入口已是 pub 则直接复用），让"parse 即迁移"从类型上杜绝漏调；
  2. 若收敛入口涉及改 csc-core 的 pub API，确保 csc-server / csc-app / 其他调用方仍编译通过
     （它们现在各自调 migrate，收敛后应统一走新入口，消除重复）；
  3. csc_restore 改为调该入口。
边界：允许改 csc-wasm/src/lib.rs 与 csc-core/src/state.rs（仅迁移入口收敛）；
      若需连带改 csc-server/csc-app 的调用点，一并小改（同一迁移语义收敛），不许改其他逻辑。
验收：
  - `cd backend && cargo test -p csc-wasm -p csc-core --locked` 全绿（wasm 现有测试 + core 存档往返测试）
  - `cargo build -p csc-wasm --target wasm32-unknown-unknown --locked` 通过
  - 新增或调整测试：构造一个低版本（如 v1–v4 形态）存档 JSON，经新入口读入后断言
    version 已升到 CURRENT_VERSION 且 honor_points 已按迁移规则归一（×0.1）。
文档同步：若收敛了入口，更新 docs/analysis/09-核心引擎.md / 10-服务端与运行时.md 的对应 API 描述；更新 19 文档 P1-3 为 ✅ 已闭环。
交付：diff 摘要 + 你收敛入口的方式说明 + 测试清单 + 验收输出。

---

## 任务 3 · P1-5 存档版本号 bump 到 9（契约动作）

任务：GameState::CURRENT_VERSION 从 8 bump 到 9，并登记 v8→v9 迁移。

文档定位：docs/analysis/19-后端代码审查-2026-09-06.md §P1-5 + AGENTS.md §0.3（存档格式变化 → bump + 同步 SAVE-FORMAT）
现状（已核实）：
- backend/crates/csc-core/src/state.rs L121 `pub const CURRENT_VERSION: u32 = 8`
- L93 `pub scheduled_records: Vec<...ScheduledTournamentRecord>`（serde(default)，注释自称 v8 旧档补空）
- L96-102 `pub sim_version: u32`（serde(default = "default_sim_version")，doc 注释写 v9 待定）
- 即：两个字段已落地但版本号停在 8，无法靠 version 区分"含/不含"这些字段的存档
契约：
  1. `CURRENT_VERSION` bump 到 9；
  2. `migrate_state()`（L148 附近）对 version 8 → 9 的迁移为**无操作**（仅补默认值，serde(default)
     已处理），但要在迁移函数里显式登记该跳变（match 分支或注释），确保未来有锚点；
  3. 同步 docs/SAVE-FORMAT.md 版本表：新增 v9 行，注明 v8→v9 = 无操作（scheduled_records/sim_version 补默认）。
边界：只允许改 state.rs 的版本常量/迁移登记 + docs/SAVE-FORMAT.md；禁止改任何字段/序列化逻辑。
验收：`cd backend && cargo test -p csc-core --locked` 全绿（含存档往返测试——旧 v8 档仍能读入并升到 v9）。
文档同步：docs/SAVE-FORMAT.md + docs/analysis/19 P1-5 标 ✅ 已闭环。
交付：diff + 迁移登记说明 + 验收输出。

---

## 任务 4 · P1-4 Engine::restore 读档原子性（框架级，单独慎重做）

任务：修复读档失败留下"半恢复引擎"继续对外服务的缺陷。

文档定位：docs/analysis/19-后端代码审查-2026-09-06.md §P1-4
现状（已核实）：
- backend/crates/csc-core/src/engine.rs L461 `pub fn restore(&mut self, state: GameState)`：按字段顺序
  逐个就地覆盖 director/clock/decision_log/journal/archive/world/vrs/tournaments，
  最后才调 `self.loop_.restore_locks(&state.locks, &mut self.world)`；
- backend/crates/csc-core/src/loop_.rs L163 `restore_locks`：队伍 ID 在 world 找不到时 L168 `panic!`
  （"快照与实体状态不一致，拒绝静默丢锁"）；
- 服务端 GameManager 用 catch_unwind 捕获该 panic 回 422，但 panic 时前面字段已被新档覆盖、
  锁没装上 → 该局带着"新档世界 + 旧锁状态"继续服务。
契约（二选一，推荐方案 1）：
  1. **构造-交换**：先在临时 Engine（或先 restore 到一个全新 Engine）上完整恢复，全部成功后再
     整体替换 self（天然全有或全无）；失败则 self 保持原样；
  2. 或让 restore / restore_locks 返回 Result 而非 panic，失败回滚。
  同时：restore_locks 的 panic 改为返回 Result（或由调用方在构造-交换路径外捕获），
  消除 catch_unwind 承接可预期错误的坏味道。
边界：允许改 csc-core/src/engine.rs、loop_.rs 及 csc-server 中调用 restore 的路径（manager.rs/lifecycle.rs）；
      必须保证存档往返测试、并发恢复测试（concurrent_restore_of_same_game_never_404s）全绿。
验收：
  - `cd backend && cargo test --workspace --locked` 全绿（596 + 新增）
  - 新增测试：构造一个 lock 引用不存在队伍 ID 的坏档，restore 后引擎状态与 restore 前完全一致
    （不残留任何新档字段），且返回错误而非 panic。
文档同步：docs/analysis/09-核心引擎.md（restore 语义）+ docs/analysis/19 P1-4 标 ✅ 已闭环。
交付：你选的方案与理由 + diff + 测试清单 + 验收输出。

---

## 任务 5+ · 文件拆分（18-文件作战清单，每批一个文件）

任务：按 docs/analysis/18-文件拆分作战清单.md 逐个拆分超 500 行文件。一次只拆一个。

文档定位：docs/analysis/18-文件拆分作战清单.md（优先级表 + 拆分纪律）
契约（该文档四、拆分纪律铁律）：
  1. 零行为变化：只挪代码 + 修 import/pub 可见性，不改签名/逻辑；
  2. 每拆一个文件跑 cargo check + 相关测试；挂就回滚；
  3. 测试随模块走或移 tests/；行数口径 = 生产代码行数（排除 #[cfg(test)]）。
建议顺序：csc-core/engine.rs → csc-tournaments/engine.rs → csc-tournaments/conductor.rs →
          csc-simulation/live_match.rs → ...（按清单优先级，每批一个）
边界：一次只拆清单里的一个文件；拆完该文件生产代码压到 ≤500 行（或按清单给出的拆法目标）。
验收：`cd backend && cargo test --workspace --locked` 全绿 + `cargo fmt --all -- --check` 0 + 该文件生产代码 ≤500 行。
文档同步：更新 18-清单该文件为 ✅ 已拆；docs/analysis/17-map-* 对应文档若存在则同步模块结构。
交付：拆分前后行数对比 + 模块结构 + diff + 验收输出。

---

## 给 Claude 的总前置指令（每个任务前都附上）

你是 CSC 仓库（CS2 职业选手生涯模拟器，Rust 后端）的重构工程师。
铁律：
1. **先读"文档定位"指定的文档章节，再读对应源码**；文档与源码冲突时停下来说明，不擅自决定。
2. **只改任务"边界"内允许的文件**；发现必须越界时停下报告，不要自作主张。
3. **禁止另起炉灶**：仓库已有实现先复用（见 AGENTS.md §10.2 防止重复造轮子三规则）。
4. 完成后跑全部"验收命令"，以真实输出为准，不编造通过；不绿就继续修到绿。
5. 交付物：diff 摘要 + 关键决策及理由 + 测试清单 + 验收命令输出（PASS/FAIL）。
