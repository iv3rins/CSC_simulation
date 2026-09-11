# 文字生涯整体纠偏计划 · 2026-09-11 16:12

依据：AGENTS Docs-First，analysis/23–26；当前基线633 passed/3 ignored，fmt/clippy未通过。用户要求16:30前交付。完整T02/T03涉及赛事、赛制、对阵、地图、干预多层调用栈，无法在本窗口可靠完成；不能以月初回放或自动代答代替。保留完整产品目标，本次交付部分可验收实现，未完成任务保持未完成。

## 一、调用链与最终架构

现有Engine.advance_month→settle_month→SeasonLoop/TournamentEngine结算→赛事赛制循环→conductor地图/node/blunder→DecisionSource.decide；再month_plan→settle→month_finish→WorldDecisionBatch。仅新增core阶段枚举无法退出内层函数栈。

最终需要：Engine step拥有年月阶段和剩余目标；赛事continuation拥有赛事/轮次/对阵；conductor continuation拥有series/maps/node/比分/战术；batch拥有已产生points/date/transfer_due。每层返回Awaiting而非阻塞，RNG仅由执行边界拥有，已产候选与已完成图入存档；同种子同选择暂停恢复等价。此最终执行方案尚需进一步拆分活变量，T00全量G0和T02/T03不得标完成。

## 二、本次实际文件边界与复用

|负责人|独占范围|本次实现|不宣称完成|
|---|---|---|---|
|execution|core batch/engine/state、必要world上下文与08/09/SAVE文档|WorldDecisionBatch collect/submit；复用run；候选状态事务交换、坏输入无副作用；修演员绑定|完整月份/赛事continuation、T02/T03总体验收|
|narrative|core/narrative、entities/narrative、剧情JSON及对应测试|合法后继、到期、结构校验；效果先全验再改；实际outcome；保留13场景|真实签约弧、完整多种子端到端可达性|
|主线程|server game/routes/assets、协议和集成测试|单局命令处理序章选择；持久receipt；内容装配；真实API与能力开关|旧human全部决策可靠提交、世界暂停|
|frontend|新web/及DEV前端文档|Solid/TS/Vite六页真实查询、阅读/选择/保存；24视觉token|不可用的世界推进，不用mock冒充|

按AGENTS团队并行规则，独立修复与展示层准备可并行；原26的集成/交付依赖不放宽，不把后项准备完成当作前项完成。原WIP=1是DeepSeek默认约束，本次队长裁决为独占文件并行，但正确性与验收标准不改。

## 三、本次新API契约

POST /games 增story:boolean，story缺省false兼容；story=true默认human，加载真实narrative资产并实例化首个合法场景。story不可经旧advance/advance-season绕过关键选择。

GET /games/{id}/career → {game_id,story,generation,status,capabilities:{advance_world:false},scene:null|{instance_id,scene_id,title,chapter_title,date,location,actor_role,actor_name,paragraphs,choices:[{id,label,impact_certain,impact_possible}]},progress,pending,view}。

POST /career/choices → {generation,request_id,scene_instance_id,choice_id}。响应{generation,request_id,scene_instance_id,choice_id,status:"committed",outcome:{immediate,tracking}}。权威命令循环先查持久receipt的完整载荷，再检查当前scene；构建候选Engine并全验效果，写盘完整包后交换Engine/session，再确认；失败不改变活Engine/receipt。无持久目录则明确拒绝committed提交，不把内存回执称持久成功。

GET /career/history → {items:receipts,next_cursor:null}（首版容量64）；GET /career/operations/{request_id}返回已知receipt，未知404。容量满拒绝新提交、明确需要后续归档实现，不淘汰后把旧请求当新请求。此有界限制必须用户可见，不宣称多年扩展完成。

POST /career/continue {}只实例化已满足事实的后继，世界step未完成时无可选返回409与明确reason；不推进月、不自动回答、不伪造比赛。结果需玩家主动继续才寻找下场景。

## 四、状态、持久化与版本

Engine仍是模拟事实唯一拥有者，路由不写world。新CareerSession只保存transport generation和完整receipts，放在PersistedGame envelope之外层字段，不污染GameState模拟指纹；原snapshot为本次提交的真实状态。正常磁盘恢复恢复session；显式POST load读取旧GameState成功后重置generation和receipt，旧请求拒绝。story资源重启从GameLoader重装。内容版本与progress必须一致；旧空progress可升级，新有进度版本不匹配拒绝，不随机换文本。

候选Engine通过snapshot+restore_result复用装配，当前首版全快照是明确性能代价，不复制另一套引擎算法。存盘复用gzip与临时文件替换，写入完整envelope校验；保存路径失败拒绝提交。完整断电持久与大存档优化另列风险。

GameState当前v10草稿已存在，不降版本。新剧情字段serde(default)；版本迁移/文档同步如实记录。本次不保证错误的旧AwaitingDecision重放语义可恢复，不对该能力开放UI。

## 五、先验收再实施

1. 现有5个审查探针明确反例已证实；建立回归不得放宽断言。
2. batch collect后不重抽，submit失败world/VRS/RNG/journal/log不变，成功run同原自动路径。
3. narrative分支A不入B、未到期不触发、坏引用/重复/无出口拒绝、效果失败不消耗场景。
4. server同ID同载荷返回完全同receipt（场景消费后也成立）；不同payload或generation拒绝；落盘失败world不变；恢复磁盘receipt可重试。
5. story请求旧advance被拒，能力false真实反映未完成；UI不模拟后果。
6. 全量Rust门禁、web typecheck/build、真实HTTP序章/保存/重启、六页检查。在截止前报告实际结果，未验证不写通过。
