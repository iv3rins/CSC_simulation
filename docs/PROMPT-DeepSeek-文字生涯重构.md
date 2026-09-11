# DeepSeek harness 执行 Prompt

> **2026-09-11 已替代**：当前工作树出现执行模型与验收偏移，请使用 [TASK 驱动纠偏 prompt](PROMPT-DeepSeek-TASK驱动纠偏.md)，先执行 [26 号任务书](analysis/26-文字生涯纠偏TASK-2026-09-11.md) 的 T00/T01。下文保留为历史实施说明，不再从旧 M1/M2 暂停点直接续写。

复制下面整段到可以访问本仓库的 DeepSeek harness。以下是实施指令，不是本次审查已完成的代码变更。

```text
你是这个项目的实现负责人。请直接完成代码实现、文档同步、测试、前后端联调和本地运行，不要只输出建议、伪代码、页面截图或待办清单。

项目：D:\repo-by-iverins\cs-career-simulation
目标：把现有 CS2 职业模拟器做成以个人故事、人物关系、关键选择与长期后果为核心的文字生涯游戏。借鉴 NBA 2K 生涯故事模式的体验组织，保留 CS2 的球队、赛事和规则，不改成篮球模拟器。

一、先读这些文件，读完先输出实施计划再写代码

1. AGENTS.md，尤其 Docs-First 与第 10 节单智能体流程。
2. docs/analysis/22-生涯叙事架构审查-2026-09-10.md
3. docs/analysis/23-文字生涯实施方案-2026-09-10.md
4. docs/analysis/24-前端视觉与交互规范-2026-09-10.md
5. docs/analysis/05、07、08、09、10 对应子系统文档。
6. docs/SAVE-FORMAT.md、docs/FRONTEND-API.md、docs/DECISION-POLICY.md。

22 是当日工作树的源码审查，不是空泛建议；23 的新增类型和 API 是待实现契约，不能当作当前已经存在；24 的布局/token 是实现约束。当前代码可能继续变化，必须按符号复核，不能盲用旧行号。

先检查 git status，保留所有已有 staged/unstaged/untracked 修改。前端和图片已被用户删除，不得 git reset/clean、整包恢复旧 frontend 或覆盖用户代码。默认在 web/ 新建独立展示 package；若工作环境已经明确提供独立前端仓库，则用该仓库并记录路径。保持根 assets/ 为业务资产源。

二、复用既有能力，不重新造一套

已存在 Engine、WorldDecisionBatch、MatchConductor、SeriesSettlement、DecisionPoint、AutoDecisionSource、Narrator、TextBundle、CareerMemory、CareerMark、season_goal、ChemistryEngine、CareerArchive、CareerEndingEvaluator、ClientState、双频道 journal、v9 存档与指纹测试。新建函数/类型前用 rg 搜索同职责实现。

后端继续使用 Rust workspace 与 Axum。规则和状态变更在后端，前端只负责呈现和提交意图。禁止在前端复算 Rating、成长、默认决策、转会资格、截止日期和比赛结果。禁止新建另一个 Node/Python 世界模拟后端。

三、按 M0 → M5 顺序完成首版

M0：复核基线与文档漂移。
- 按 22 的裁决先修正 v8/v9、旧前端策略、错误 API 签名、响应包装、VRS 接触回退等文档。
- 使用真实 CI 目标 wasm32-unknown-unknown。
- 审查时已有 604 passed / 0 failed / 3 ignored，fmt/clippy/wasm 通过；这只是历史基线，你必须重新验证，不能照抄结果。

M1：优先修正确性，先写能击中问题的回归测试。
- 批次身份不能在 mpsc 通道里丢失；REST/WS 最终在单局状态拥有者处校验/消费，防止双击与超时竞态把旧选择送进新批次。
- 完整校验 point/option 集合：未知、重复、漏项、候选外选项都拒绝；Human 不得静默默认。
- 先验证，再原子应用与写入日志；失败不改变世界/RNG/journal/pending。
- LIVE decide 当前先 pending.take 再验证候选，改成验证成功才消耗，并验证非法请求后仍可合法选择。
- request_id 幂等：同 ID 同载荷返回原结果，同 ID 异载荷拒绝；不能仅用按钮 disabled 代替后端幂等。

M2：实现持久暂停的执行状态。
- 拆分现有 collect/apply 和比赛决策边界，形成可序列化 continuation 与 step 结果。
- 返回 AwaitingDecision 后释放模拟线程，使查询/存档/关闭仍可工作。
- 把 pending、执行阶段、已消耗随机输入、剧情进度及必要比赛中间状态纳入存档。
- 关键故事选择默认持续等待，不因为读得慢或断网 5 分钟就被 Auto 代选；不能仅删除 timeout 后继续阻塞线程。
- 恢复后不得重新随机、重复训练/发钱/转会/结算比赛，旧网页的迟到命令必须失效。
- 故事 continue 由后端推进到下一个重要节点，每个步进边界都检查停点；禁止用旧 advance-season 无条件托管作为故事主页按钮。
- 保持旧 Auto/CLI 通过同一规则链可运行；保证阶段性改动可编译，勿一次推倒所有引擎。

M3：完成 3 条剧情弧、至少 12 个可达场景。
- 新秀归属、压力与信任、去留与承诺，按 23 的触发/四场景要求实现。
- 每条弧至少两个不同终态、一个延迟回访，有输球/无报价/伤病等失败或等待出口。
- 演员用稳定 ID，前置条件引用真实比赛/队伍/合同；没有事实就用概括叙述，禁止虚构已经拿到的冠军/合同。
- 复用关系、声望、目标与职业记忆；仅新增无法可靠派生的剧情进度、承诺、flags、按类型/对象冷却。
- 首次记忆和重复职业节点分开去重；旧档未知日期/人物不猜测。
- 场景效果是受限 Rust enum；禁止 eval、任意 JSON path 改世界、运行期 LLM 决定结果。
- 修复文案与主 RNG 耦合及换队后历史叙事归属漂移；所有模拟语义变化明确记录版本和预期指纹变化。
- 正式比赛先用现有 conductor/settlement 的结果，接赛前/图间选择和赛后叙事。独立 LiveSessionStore 尚未接入正式结果，首版不在主流程开放它，不能用临时 LIVE 比分当生涯成绩。

M4：按 24 实现新前端并接真实 API。
- 默认 Solid + TypeScript + Vite，选择兼容版本并提交 lockfile；不同时引入多套框架。
- 六个主页面：生涯、赛程、战队、世界、档案、设置。
- 生涯首页只有一个当前主事件和一个主要行动。场景有章节/日期/地点/人物/正文/选项/结果，右侧最多两个辅助区。
- 训练、财务与转会放入上下文，不做十几个等权重仪表盘卡片。
- 先写后端 DTO/接口测试/文档，再写前端类型；明确响应是 {pending:...}、{summaries:[...]} 等实际结构。新 /career 契约按 23 实现，REST/WS 共用命令执行，不分别应用副作用。
- 高频使用 /view 与小型 /career 视图，journal 分频道游标对账；禁止轮询完整 /state。
- 初次创建明确传故事用 human 策略，不能遗漏后端默认 auto 的差异。
- 前端没有接口时不悄悄使用 mock，生产界面不出现假按钮、虚假属性收益或硬编码比分。

四、审美与交互硬约束

参考 https://sondaven.com/en 与 https://www.hltv.org/。
Sondaven 取场景叙事与节奏；HLTV 取赛事资料层次与紧凑可读的数据组织。本次审查已读取两站文本及创作者/官方设计说明，但浏览器画面读取失败，没有像素级核验。你能访问时实际观察，不能访问就按 24 完成，不虚报视觉对齐。

- 使用 24 的深墨蓝框架、暖纸阅读面、低饱和金色强调及字号/间距 tokens。
- 桌面用 Grid/Flex 三列，移动端单列重排；不要绝对坐标排整页或 transform scale 缩小桌面。
- 中文故事正文约 18px、行高 1.85；场景标题 32–40px；表格数字对齐。
- 禁止紫蓝渐变、霓虹发光、玻璃拟态、随机 emoji、满屏圆角 KPI 卡片、大量无意义图表、酒店式长滚动营销首页。
- 没有合法图像时用原创线稿/中性徽记，不下载参考站品牌图片，不引用已删除路径。
- 点选和确认分开；提交中防重复；错误保留用户输入；冲突刷新当前 pending；响应丢失先对账，不能用新 request_id 自动重放。
- 路由加 overlay stack，正确处理焦点/返回；键盘可走完整生涯流程，手机触摸目标至少 44px，200% 字号仍可操作。
- 动效克制、支持减少动效；不强制打字机等待、不滚动劫持、不自动播放声音。
- 必须实现加载、空内容、待决策、提交中、结果、409 冲突、断线重连、读档失败、赛季总结和生涯结束状态。

五、存档、文档与测试

任何存档字段或语义变化都按 SAVE-FORMAT bump，并写旧档迁移测试；世界随机语义、内容版本与传输 generation 分别管理。不得更新 golden 值来掩盖无解释的模拟漂移。

每个阶段同步相关 docs/analysis 与 FRONTEND-API、SAVE-FORMAT、DECISION-POLICY；新增文档挂入 docs/analysis/README.md。22 已裁决的旧文档错误先修文档即可，不必反复请示；新的实质矛盾则说明证据、暂停相关部分并继续独立工作。

后端先跑有关 crate 回归，再在 backend/ 跑：
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo build -p csc-wasm --target wasm32-unknown-unknown --locked

前端建立并执行 typecheck、test、build、test:e2e。核心 E2E 必须接真实后端：建局→当前剧情→选择→真实结果→继续→正式比赛→赛后回访→保存→重启/刷新→恢复。补并发双提交、迟到请求、非法选项、跨月/跨年、剧情冷却、换队历史、待决策保存、网络中断恢复等用例。

视觉验收至少 1440×900 与 390×844，全页关键状态截图存 docs/screenshots/career-v1/，补查 1920×1080、768×1024 与 200% 字号。无读图能力时可用已有 vision-bridge，但失败要如实报告，不将失败记为 PASS。截图不能代替功能测试。

有条件时跑现有 release twenty_year 与 scripts/smoke.mjs，先阅读其行为并用隔离端口/数据目录，不碰用户存档。每项验证写实际命令、结果和限制，不虚报没跑过的门禁。

六、工作方式与完成定义

遵守 Docs-First：查文档→核源码→摸清复用→明确计划→实现→验证→同步文档。若 harness 的 pro/flash 工具和通道实际可用，按真实并发上限分开写码与审查；不可用则按 AGENTS 第 10 节独跑，不假装已经组队，不无限重试失效模型。

优先实现一条贯通“真实场景→选择→后果→保存恢复”的纵向链，再扩展到 3 弧/12 场景与 6 页。不要先写一百个空壳场景或漂亮 mock。功能/API 已经就绪后再扩页面，不并行修改同一个核心文件。

本轮必须完成 M0–M5 的首版；逐回合 LIVE 正式结算整合、语音/LLM、3D 和云部署留作后续专项。不能只做其中一页就声称完成，也不要为了这些后续专项拖住首版。

完成后启动本地前后端，先检查端口，勿停止他人服务。后端现有启动形式：在 backend/ 执行 cargo run -p csc-server -- ../assets 8080；前端用同源 REST/WS dev proxy。按实际配置选本地空闲端口，记录持久化目录。后台进程在 Windows 隐藏启动，不弹额外终端窗口。

最终交付：
1. 实际可访问地址、启动/停止命令、服务 PID 与数据目录。
2. 实现功能与关键文件，明确哪些能力是复用、哪些是新增。
3. 测试结果、截图与 docs/DEV-文字生涯-验收报告.md。
4. 存档迁移、API 变化与文档索引。
5. 尚未完成/环境阻塞的精确说明；不把历史审查结果当本轮验证结果。

现在开始读文档、核实代码、输出阶段计划，并持续实施到首版完成。
```
