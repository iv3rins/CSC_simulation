# 文字生涯实现交付 · 2026-09-11 16:30前

## 交付范围

**项目整体尚未完成。** 本轮交付真实可运行的序章/资料展示与若干架构修复；完整比赛/世界可恢复推进仍未完成，因此网页明确禁止推进世界日期。这是尚未实现的能力，不是已完成玩法的一种模式。

已完成代码：

1. WorldDecisionBatch collect/submit分离，候选world/VRS/RNG/journal/decision事务，失败不污染原状态；旧run复用。不是完整月/日step。
2. 剧情所选后继持久化、承诺到期检查、重复/坏引用/循环校验，效果全验后应用，实际数值后果和结构化历史；内容v2共13场景。真实转会签约与生产流程三弧可达性未完成。
3. 新序章API在单局命令循环执行，候选Engine写盘成功后交换；同请求在已消费场景/正常磁盘恢复后返回同receipt；显式load创建新generation。新场景回执与旧Human决策通道是不同交付范围，后者遗留未被掩盖。
4. GameState v11、磁盘包v3及文档同步；拒绝无法安全恢复的旧伪Awaiting档，普通旧档按迁移规则读取。
5. 新web六页：生涯、赛程、战队、世界、档案、设置。真实API、显式确认、结果保留、保存下载/导入入口、字号和减动效，遵守24号深墨蓝/暖纸/金色与阅读优先布局。

## 验证证据

- Rust workspace：上一轮完整门禁 **649 passed / 0 failed / 3 ignored**，Clippy/fmt/WASM全通过；最终中文结果与浮点展示收尾后再次复验，最终结果见下方“最终门禁”。
- 序章集成：`tests/career_api.rs` 2项通过，覆盖并发相同请求、消费后重试、异载荷拒绝且状态相同、磁盘包重建运行实例、旧generation拒绝、写盘失败保留场景/world/receipt。不是仅检查HTTP状态码。测试用actor shutdown模拟进程丢失，不能用DELETE game（该操作按既有语义删除存档）。
- 前端：npm typecheck/build通过；浏览器真实建局→选radio不提交→确认→权威结果→切页返回保留结果→JSON save200/v11。
- 新建页两视口及六页1440/390共12张游戏截图，DOM无API报错/页面横向溢出。证据在 `docs/screenshots/career-v1/`，`interaction.json`和`dom-game.json`。截图证明布局，不能替代所有交互验收。
- 最终日志：`docs/analysis/implementation-2026-09-11-final-{tests,clippy,fmt,wasm,build}.log`。前端记录见DEV前端页面计划与web日志。

## 运行

本轮已启动后端8080和前端5173，打开 http://127.0.0.1:5173/ 。创建新生涯可体验序章与六页资料；`capabilities.advance_world=false`，完成序章后会明确说明推进尚未开放。

正常重新启动（两个终端）：

```powershell
cd D:\repo-by-iverins\cs-career-simulation\backend
cargo run -p csc-server --locked -- ..\assets 8080
```

```powershell
cd D:\repo-by-iverins\cs-career-simulation\web
npm ci
npm run dev
```

停止自行启动的终端用Ctrl+C。本轮后台服务的确切进程和停止方式在最终运行记录补充。不要结束不属于本轮的用户进程。存档默认在既有CSC_RUNTIME_DIR/OS临时目录逻辑下；可通过网页设置下载JSON留存。

## 必须继续的任务

- T02/T03：全层级world/tournament/series/map continuation；T04完整暂停时仍能查询/保存/恢复/退出。
- T05：旧Human完整协议的权威提交、幂等与持久化；新场景receipt目前最多64条，满后拒绝，归档方案未完成。
- T07/T09：带PlayerId的正式参赛事实、真实去留签约/拒绝分支、3弧至少12个场景通过实际driver到达。当前保守关闭无可靠事实支持的赛后触发。
- T11/T12：完整生涯游玩、UI读档/断网全链、200%字体及更多断点、长期性能/容量。全快照候选事务是当前性能代价，未完成长档优化。

本轮没有提交/推送Git，没有恢复用户删除的旧frontend或图片。原有未提交修改保留。

## 最终门禁与运行记录

2026-09-11 16:28 最终复验完成：**649 passed / 0 failed / 3 ignored；Clippy exit0；fmt exit0；WASM build exit0；server build exit0**。全部依据final日志，未忽略新增失败。

最终服务已用验证后的二进制重启：后端PID26336（8080）、前端Node PID25876（5173）。本轮后台停止命令为 `Stop-Process -Id 26336,25876`；执行前应核对进程仍对应本轮服务，防止进程号以后被复用。Chrome PID8636为本轮独立截图辅助。

前端还完成了真实UI存档导入（下载→选择文件→确认→成功→清除草稿），证据 `docs/screenshots/career-v1/load-interaction.json`。全量剧情、断线所有交错、长期性能仍未验收，不能据此标项目完成。
