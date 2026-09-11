# AgentTeam 组队配置手册（cs-career-simulation）

> 本文件记录本项目使用 AgentTeams 多智能体协作时的**模型可用性、并发规则、角色分工、
> 任务组织方式与 Docs-First 工作流**。
> 用途：主线程/队长（Captain）在每次组队开工前查阅，确保按当前资源正确建队、正确派活、避免踩坑。
> 最后更新：2026-08-24（**Docs-First 工作流上线** + 并发升级：pro 5 / flash 4；分工调整：pro=审查、flash=写码）

---

## 0. Docs-First 工作流（写代码前必读 · 最高优先级铁律）

> ⭐ **任何新增/修改代码前，必须先查文档，按文档定位内容，再动手。**
> 本工作流适用于队长派活与每名队员执行任务——两个角色都必须遵守。

### 0.1 流程（每个代码任务必须完整走完 6 步）

```
① 查文档定位
   - 后端改动 → docs/analysis/02-14 对应子系统文档（API 风格：Struct/Enum/Trait/Function/Interface）
   - 存档/协议改动 → docs/analysis/09（GameState）+ docs/SAVE-FORMAT.md
   - 设计背景 → docs/ 根目录（design.md / README.md / 既有评审文档）
② 读文档定位目标 API/模块
   - 找到要改的 Struct/Trait/Function/Interface 及其签名、所属文件
   - 确认改动点（精确到文件:行号级别）
③ 对照源码确认
   - 文档优先，但以源码为最终事实——先信文档、再核源码
   - 若文档与源码不符 → **停下来报告队长**（可能文档滞后，需先修文档再改码）
④ 新增/修改代码
   - 严格按文档定位的模块/签名实现，不另起炉灶、不重复造轮子
   - 先 grep 确认没有既有实现再新建
⑤ 同步更新文档
   - 任何 API 新增/改名/签名变化 → 同步修订 docs/analysis/ 对应文档
   - 新功能 → 对应文档补录；遗留清单变化 → 更新 docs/analysis/14
⑥ 验证
  - 跑门禁：cargo test / cargo clippy / cargo fmt --check（见 §4.4）
```

### 0.2 文档快速定位表（要改什么 → 先查哪份）

| 改动目标 | 先查文档 |
|---|---|
| RNG / ID / 错误 / 时钟 / 值对象 | docs/analysis/02-基础层 |
| 实体 / 属性 / 队伍 / 生成器 | docs/analysis/03-实体层 |
| 规则公式（胜率/Rating/成长/训练/比赛/经济） | docs/analysis/04-规则层 |
| 事件流 / 决策点 / 生涯档案 | docs/analysis/05-事件与决策 |
| VRS 排名 | docs/analysis/06-排名系统 |
| 系统引擎（转会/人口/伤病/化学/经济/人生） | docs/analysis/07-系统引擎 |
| 赛事系统（排程/赛制/结算/TOP20） | docs/analysis/08-赛事系统 |
| 总引擎 / GameState / 查询层 | docs/analysis/09-核心引擎 |
| 服务端 REST/WS / CLI / WASM | docs/analysis/10-服务端 |
| 脚本 / 资产 / CI | docs/analysis/13-脚本与数据 |
| 遗留风险 / 完成度 / 架构结论 | docs/analysis/14-架构审查结论 |
| 第二轮多智能体审查修正记录 | docs/analysis/16-架构整理报告-2026-09 |
| 存档格式版本契约 | docs/SAVE-FORMAT.md |
| 决策分级与自动策略 | docs/DECISION-POLICY.md |
| 多局容量模型 | docs/CAPACITY-MODEL.md |

### 0.3 文档同步规则（改码必改文档）

- **API 签名/字段/端点变化** → 同步修订 docs/analysis/ 对应文档（同一任务内完成，不拖欠）；
- **新增 API** → 在对应文档的 API 表按分类补录（Struct/Enum/Trait/Function/Interface）；
- **遗留清单变化** → 更新 docs/analysis/14（标注 ✅ 已闭环或从清单移除）；
- **存档格式变化** → bump `GameState::CURRENT_VERSION` + 同步 docs/SAVE-FORMAT.md；
- **文档与源码冲突** → 先报队长裁决；不擅自改文档掩盖代码事实，也不擅自改码绕过文档；
- **审查/核对类任务**（pro）不改文档，只输出结论；文档修订由队长或指派队员执行。

---

## 1. 模型通道可用性与角色分工（当前）

| 角色 | 通道 / 模型 | 并发 | 职责 |
|---|---|---|---|
| **队长（Captain）** | **当前模型** | **1 并发** | 建队、拆任务、排依赖、监控状态、转派失败任务、合并结果、文档裁决 |
| **审查/reviewer（pro）** | xiaobai / deepseek-v4-pro-vip | **5 并发** | 代码审查、设计把关、核对、复审、终审、结论质量评估 |
| **编写队员（flash）** | xiaobai / deepseek-v4-flash-vip | **4 并发** | **实际编写代码**：实现、接线、测试、文档同步 |

**要点**：
- **pro 不写生产代码**，只做审查/核对（reviewer 职责）——避免"自己写自己审"；
- **flash 是生产主力**（写码），按 Docs-First 流程执行任务并在完成后同步文档；
- **队长 1 并发只协调**：拆任务、监控、合并，不占生产/审查并发槽；
- 思考强度：队员建队时显式传 `reasoning_effort: "max"`（已实测可用）；
- GPT 通道（gpt-5.6-sol-pro）当前不可用（余额耗尽），恢复后可作终审（见 §6.4）。

---

## 2. 并发核算规则

```
队长（当前模型）:         1 并发（只协调，不写码不审查）
pro（deepseek-v4-pro）:   5 并发（审查/reviewer：review/核对/终审）
flash（deepseek-v4-flash）:4 并发（生产写码：实现/接线/测试/文档同步）
```

**推论**：
- 同一时刻最多 **4 个写码队员（flash）并行** + **5 个审查（pro）并行**；
- 典型满配：4 flash 写 4 条线 + 2~3 pro 审前一轮 + 2 pro 审后一轮；
- 一人一任务（AgentTeams 强制）：想并行 = 加队员，不是给同一队员排多个任务；
- 队长若占用某通道的并发槽（如用 xiaobai 跑队长），该通道并行度减 1——**队长保持当前模型**。

---

## 3. 推荐队伍构成

### 3.1 标准生产队（满配 9 名：1 队长 + 4 flash 写码 + 4 pro 审查）

| 角色 | 模型 | 并发 | 职责 |
|---|---|---|---|
| **队长（Captain）** | 当前模型 | 1 | 建队、拆任务、排依赖、监控、转派、合并、文档裁决 |
| **flash-dev-1 ~ 4** | deepseek-v4-flash-vip | 4 | 写码主力：按 Docs-First 实现、接线、测试、文档同步 |
| **pro-review-1 ~ 4** | deepseek-v4-pro-vip | 4~5 | 审查/reviewer：代码审查、核对 flash 产出、设计把关、终审 |

### 3.2 小型队（2 写码 + 2 审查，常见配置）

| 角色 | 模型 | 并发 | 职责 |
|---|---|---|---|
| 队长 | 当前模型 | 1 | 同上 |
| flash-dev-1 ~ 2 | deepseek-v4-flash-vip | 2 | 写码 |
| pro-review-1 ~ 2 | deepseek-v4-pro-vip | 2 | 审查/核对 |

### 3.3 纯审查队（不写码，如文档/代码复审）

| 角色 | 模型 | 并发 | 职责 |
|---|---|---|---|
| 队长 | 当前模型 | 1 | 拆审查域、汇总结论 |
| pro-review-1 ~ 5 | deepseek-v4-pro-vip | 5 | 分域独立审查 + 交叉核对 |
| （可选）flash-dev-1 ~ 2 | deepseek-v4-flash-vip | 2 | 扫描盘点、跑测试（只读操作） |

---

## 4. 任务组织模式（Docs-First 版，已验证有效）

### 4.1 依赖链 + 流水线（推荐：先文档后代码）

```
t0 文档定位（队长派活时完成：指定 docs/analysis/ 文档 + 文件:行号）
  └→ t1 写码（flash-1，按文档实现）     ┐ 可并行 4 条线
  └→ t2 写码（flash-2，独立模块）       ┘
        └→ t3 审查（pro-1，核对 t1/t2 产出 vs 文档 vs 源码）
              └→ t4 文档同步（flash，按审查结论修订文档）
                    └→ t5 回归（flash，跑门禁）
```

### 4.2 双主线并行（两条线互不依赖时）

```
线 A：后端实现（flash-1）     线 B：后端实现-独立模块（flash-2）
        └→ 统一交给 pro-review 审查
```

- 写码并发上限 4（flash），审查并发上限 5（pro）——两类任务互不抢占；
- 例：LIVE 后端接入（flash-1）‖ R2 journal 双通道（flash-2）‖ 文档预审（pro-1）并行开工。

### 4.3 任务描述必备要素（队长写任务时，Docs-First 强制项）

1. **文档定位**：指定 docs/analysis/ 对应文档 + 章节（队员第一步读它）；
2. **背景/现状**：该模块已有什么、缺什么（精确到文件路径，队员不用全仓扫描）；
3. **交付契约**：明确端点签名/JSON 示例/事件类型/协议接入点，双方按契约开发；
4. **边界**：明确不与并行任务冲突的文件范围（例：A 改 live.rs，B 只动 journal 段）；
5. **验证命令**：`cargo test -p xxx` / `cargo clippy -D warnings`；
6. **文档同步要求**：改动涉及哪些文档、需补录/修订哪些条目（见 §0.3）；
7. **交付物**：代码 diff 摘要 + 文档修订说明 + 任务输出摘要。

### 4.4 验证门禁（写码队员完成后必跑）

| 层 | 命令 | 门禁 |
|---|---|---|
| 后端 | `cargo test --workspace --locked` | 全绿（604+，随有效测试增加而增长） |
| 后端 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 警告 |
| 后端 | `cargo fmt --all -- --check` | 0 违规 |
| 后端 | `cargo build -p csc-wasm --target wasm32-unknown-unknown` | wasm 可编译（CI 实际目标为完整 triple） |

---

## 5. 交付物路径约定

- **队员可写**：项目 `docs/` 目录（含 `docs/analysis/` 与 `docs/DEV-*.md`，已实测可写）。
  命名：`docs/DEV-<子系统>-<主题>.md`；
- **队员可能不可写**：`.agent-teams/` 团队目录（部分成员环境只读）——**不要**要求队员把交付文档
  写进 `.agent-teams`，让它写 `docs/DEV-*` 或 `docs/analysis/`，或在任务输出里给出完整内容；
- **队长收尾**：把队员散落的交付文档归拢到 `docs/analysis/` 或 `docs/DEV-*.md`，并更新
  `docs/analysis/README.md` 索引（新增文档必须挂进索引表）。

---

## 6. 踩坑记录（本仓库实战经验）

### 6.1 队员子代理"无限崩溃循环"

- **症状**：任务 attempt 计数快速飙升（几十~上百），每次"failed before finished + 空消息"，无实质产出。
- **常见原因**：
  1. 模型通道**到期/欠费**（本仓库 2026-08-21 曾因 xiaobai deepseek 未续费，全部队员一启动就崩）。
  2. 队员子代理**上下文损坏**（长上下文后单次失败，调度器反复拉起同一个坏会话）。
- **处置**：
  - 检查通道余额/可用性 → 换可用模型重建队员。
  - 单个队员坏：`agent_teams_remove_member` 移除 → `agent_teams_add_member` 加全新队员（上下文干净）→ claim 任务接续。**代码已落盘不丢进度**。
  - 判断"纯空转"vs"间歇抖动能推进"：看 attempt 是否伴随文件 mtime 变化/状态是否进入 in_progress；间歇抖动（偶发失败但持续产出）不要换人，换人会丢上下文白烧额度。

### 6.2 调度器自动转派

- 移除成员后，其任务回到待办池，调度器可能自动转给其它空闲成员（不一定是你要的人）。
- 想指定接班人：先 `agent_teams_remove_member`（旧人）或直接 `agent_teams_reassign_task(task, assignee)` 指定。

### 6.3 单成员同时只能持有一个任务

- 即使模型并发高（pro 5 / flash 4），AgentTeams 仍强制**一人一任务**。想并行 = 加队员，不是给同一队员排多个任务。
- `agent_teams_reassign_task` 在目标队员已有任务时会报错 "member is busy"，等其空闲再转。

### 6.4 GPT 通道注意事项（历史记录）

- gpt-5.6-sol-pro 是**按量计费**（余额曾一次性跑完 LIVE/R1/R2 全部任务即耗尽），当前不可用。
- 恢复后建议只做**终审/设计把关**（pro 类职责），不做大批量实现。
- 失败特征：长上下文 + 高耦合改动任务时最易崩；换新队员接续（读落盘代码）比反复重试快。

### 6.5 通道"启动即崩"鉴别（2026-08-22 实测）

- **症状**：队员每次 spawn 立即失败（attempt 秒涨 30+，全空消息）；同一天上午同一通道还正常。
- **鉴别**：TCP 可达 ≠ 可用（`Test-NetConnection <host> -Port <port>` 通但队员全崩）；
  同会话同机器换一个通道对照测试——**跨通道对照是最快鉴别法**。
- **兜底**：换可用通道建队；恢复前先单队员试跑一轮，别直接开满并发大任务。
- **教训**：建队即崩 ≠ 换人重试能解决——先跨通道对照；队长应把"通道可用性"列为开工前三确认之一。

### 6.6 文档与代码不一致（Docs-First 常见坑）

- **症状**：队员按文档写码，发现文档签名与源码不符（编译失败或逻辑对不上）。
- **处置**：
  - 先核对 `docs/analysis/16-架构整理报告-2026-09.md`（六大冗余模式 + Sprint 路线图）——多数旧文档错误已登记；
  - 仍不符 → 报队长裁决：先修文档（flash 任务）还是先按源码改码（改完同步文档）；
  - **不要**静默按源码改码却不更新文档（违反 §0.3），也不要死磕过期文档不改码。

---

## 7. 一键建队模板（队长可复制）

### 7.1 标准开发队（写码 + 审查）

```text
agent_teams_create: name=csc-<目标>, description=<目标+现状+约束+Docs-First>

# 写码队员（flash × 4，按 Docs-First 实现）
member flash-dev-1 ~ 4 = xiaobai / deepseek-v4-flash-vip（reasoning_effort: max）

# 审查队员（pro × 4~5，reviewer 职责）
member pro-review-1 ~ 4 = xiaobai / deepseek-v4-pro-vip（reasoning_effort: max）

任务链（每条任务描述必含 §4.3 七要素，第一步是文档定位）：
  t1 写码 A（flash-1，依赖文档定位）‖ t2 写码 B（flash-2）
  └→ t3 审查 A+B（pro-1，核对 产出 vs 文档 vs 源码）
  └→ t4 文档同步（flash，按审查结论修订）
  └→ t5 回归（flash，跑门禁）
```

### 7.2 纯审查队（文档/代码复审，pro 主力）

```text
agent_teams_create: name=csc-review-<目标>, description=<审查域划分+结论输出要求>

member pro-review-1 ~ 5 = xiaobai / deepseek-v4-pro-vip（分域独立审查 + 交叉核对）
member flash-dev-1 ~ 2 = xiaobai / deepseek-v4-flash-vip（只读扫描/跑测试，不写码）

任务链：
  t1~t5 分域审查（pro 并行，各产出 🔴🟠🟡 分级报告）
  └→ t6 交叉核对（pro，抽查 60%+ 条目）
  └→ t7 合并裁决（pro，最终修正清单）
  └→ 队长整合为 docs/analysis/16-* 报告
```

---

## 8. UI 视觉核验（Vision Bridge，替代 Ox 截图 MCP）

> 2026-08-27 由用户指定：**Ox 已弃用，改为直连 opencode-go 网关的 DeepSeek V4 Flash Vision Exp 搭视觉桥**。
> 本节记录用法，供队员/队长做 UI 视觉回归时查阅。主模型（deepseek-v4-flash-vip）无读图能力时，视觉核验一律走此桥。

- **视觉桥 = `scripts/vision-bridge.mjs`**：直连 opencode-go 网关（`https://opencode.ai/zen/go/v1`，
  auth.json `opencode-go` 通道）的 `deepseek-v4-flash-vision-exp`，对截图做视觉核验
  （布局/空白/重叠/溢出/配色），对齐 `docs/QA-AUTOTEST-prompt.md` §5 协议。
  - 单图：`node scripts/vision-bridge.mjs <img.png> ['自定义提示词']`
  - 批量：`node scripts/vision-bridge.mjs --dir docs/screenshots --out <json>`
  - 输出判定：`PASS / WARN / FAIL`，含 layout/blank/overlap/color 四维 note + issues 清单。
- **截图来源**：Chrome headless CDP（`chrome.exe --headless=new --remote-debugging-port=<port>`）生成
  页面截图存 `docs/screenshots/`；或浏览器手动保存。Ox（chrome-devtools-mcp 截图工具）不再使用。
- **流程**（对齐 `docs/QA-AUTOTEST-prompt.md`）：
1. 起服务端 `cargo run -p csc-server -- ../assets 8080`（当前仓库无前端源码；未来展示层接入后同源访问）。
     （Vite 代理同源，访问 `http://127.0.0.1:5173/` 或 5174）；用户后台有任务时先确认端口空闲、轻量启动。
  2. 每进入一个页面：先拿 DOM 结构（可 evaluate 或 a11y 树），再截图拿视觉，用视觉桥核验。
  3. 核验标准：无组件重叠、无横向溢出、无大面积空白（桌面 1440px / 移动 390px 视口各一轮）。
  4. 截图产物存 `docs/screenshots/`，核验 JSON 存 `docs/screenshots/vision-summary.json`。
- **已知坑**：vision 模型 reasoning 长时 content 可能为空 → 脚本已内置 max_tokens=4000 + 自动重试；
  网关对 urllib/裸 UA 返回 403（Cloudflare），必须带浏览器 UA（脚本已内置）。
- **边界**：视觉桥仅作视觉与交互核验的辅助手段；功能正确性仍以服务端确定性回归
（`cargo test --workspace`，同种子同决策=同世界）为准。

---

## 9. 开工前三确认（队长 checklist）

1. **通道可用**：flash/pro 通道余额与并发（见 §1）——必要时先单队员试跑一轮；
2. **文档就位**：本次改动涉及的 docs/analysis/ 文档已确认存在且最新（必要时先跑一轮文档核对）；
3. **任务描述完整**：每条任务含 §4.3 七要素（文档定位 / 背景 / 契约 / 边界 / 验证 / 文档同步 / 交付物）。

---

## 10. 单智能体完成编程任务的标准流程（主智能体工作法）

> 2026-08-27 补录。背景：此前直接用 DeepSeek 单通道跑"实现 → 验证失败 → 修复"循环，
> 结果是**大量重复造轮子**（不查既有实现就新建）+ **没有队伍协作**（写/审/回归全由同一个
> 会话承担，自己改的自己审不出问题）。本节记录主智能体（ZCode/Claude 类编码智能体）完成
> 一项编程任务的完整方法，DeepSeek 直连场景可直接照抄这套节奏。

### 10.1 核心原则：先计划、后动手，一次做对

"修复→验证→修复"死循环的根源是**动手前没有完成定位**：没读旧代码就写新代码，
自然造出已有的轮子；没有明确验收标准就去跑验证，跑挂了也只能盲修。
正确顺序是**七步，其中前四步（约占 50% 工作量）一行代码都不写**：

```
① 读文档定位     docs/analysis/ 对应子系统文档 + §0.2 定位表 → 找到目标 API 签名与文件:行号
② 对照源码核实   文档是地图不是事实——grep 既有实现，确认文档没有滞后（不符 → 报告裁决）
③ 摸清周边       读同模块相邻代码的命名/风格/错误处理习惯；列出"可复用什么、要新增什么"
④ 写改造计划     明确改动点清单（文件:行号）、边界（不动哪些并行中的文件）、验收命令
⑤ 按计划实现     只在计划内的位置动刀；严格复用③里确认过的既有函数/类型，不另起炉灶
⑥ 跑门禁验证     cargo test / clippy / fmt（§4.4），以输出为准不编造通过
⑦ 同步文档       新增/改名/签名变化 → 当轮更新 docs/analysis/（§0.3），不留文档欠账
```

### 10.2 防止重复造轮子（针对"轮子多"的三条硬规则）

1. **新建之前必 grep**：想新增函数/Struct/工具方法前，先全文搜索同名或同职责实现
（如 `fn parse_`、`derive(Deserialize)` 的现有 DTO、现成的查询/派生函数）。
   找到就复用/扩展，实在没有才新建——并在新代码里说明为什么既有实现覆盖不了；
2. **信文档、核源码**：本项目 docs/analysis/ 已按层拆好（§0.2 定位表），轮子清单就在文档里，
   先查再写能省掉大部分"我以为没有所以我自己写了一个"的情况；
3. **一个概念只允许一个实现**：审查阶段专门加一项检查——"本次 diff 是否引入了与既有代码
   语义重复的辅助函数/组件/DTO？"发现即合并（这是 pro 审查的标准核查项之一）。

### 10.3 打破"修复→验证→修复"循环的关键动作

- **验证标准前置**：写码前就把验收命令和预期行为写进任务（§4.3 第 5 要素），
  而不是等第一版跑挂了再想"到底什么算对"；
- **失败先归因、再改**：门禁报错时先把完整报错读一遍并归类（编译错？逻辑断言？
  并行任务的边界冲突？），禁止看到红色就随手补丁——大多数"越修越糟"来自对
  第一条真实错误的误判；
- **同一处修两次仍不过 = 升级问题**：回到①重新定位（大概率是②漏了某个源码事实），
  不要进入第三次盲修；必要时停下来报告队长/用户裁决，而不是继续烧 token；
- **小步提交**：每完成一个计划内改动点就跑一次相关子集测试（`cargo test -p xxx`），
  把大爆炸式的"最后统一验证"变成每步都有反馈的短循环。

### 10.4 与 AgentTeams 配合的关系

- 本节流程是**单个队员（flash 写码）/ 主智能体独跑时**的最小纪律；
- 有队伍时，把 ①~④ 作为任务前置由队长派活时给出（§4.3 七要素就是把这份纪律
  写进任务描述），⑤~⑦ 由队员执行，另配 pro 审查兜住 ⑦ 与 10.2-规则3；
- **DeepSeek 直连独跑 ≠ 放弃分工**：即使是单会话，也要在同一会话内人为切换角色——
  先当审查者给自己列 checklist（是否有重复实现？是否漏跑门禁？文档是否同步？），
  再回来当作者执行。缺人审的危害远大于慢。

### 10.5 独跑时的任务模板（可复制给 DeepSeek）

```text
任务：<一句话目标>

文档定位：<docs/analysis/XX 章节>
现状：<模块已有什么、缺什么，精确到文件路径>
契约：<端点签名/JSON 示例/事件类型>
必复用：<已确认存在的函数/类型清单（来自 grep 结果）>
边界：<不允许触碰的文件/模块>
验收：<cargo test -p xxx 全绿 / cargo clippy -D warnings 0 违规 ...>
文档同步：<涉及 docs/analysis/XX 的哪些条目需要补录/修订>

第一步永远是读上面"文档定位"指向的内容，读完先输出你的实现计划再写代码。
```
