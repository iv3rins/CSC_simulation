# 文字生涯前端页面计划（2026-09-11）

依据 analysis/24 全篇、23 API、25 复审及26 T10/T11，采用 game-ui-ux 的容器布局、safe area、语义键盘输入和事件刷新。主线程裁决允许独立展示工作并行，完整执行器未验收，不能据六页存在宣布T11通过。

## 页面与事实来源

| 页面 | 主内容 | DTO |
|---|---|---|
| 生涯 | 当前场景、显式选择、权威结果 | career.scene / choices receipt；view.world 身份 |
| 赛程 | 我的已确认赛事、对阵；年度计划区分未确认 | calendar.player_events / events/{name} |
| 战队 | 当前阵容、排名、凝聚力 | view.world.team / players |
| 世界 | 新闻与球队动态，紧凑排名 | news.items / world.stories / view.world.teams |
| 档案 | 已做选择与赛季记录 | career/history.items / archive.archive |
| 设置 | 正文字号、减少动效、存档下载/导入 | 本地偏好；save / load |

桌面三列，主导航184px、内容弹性、侧栏280px；手机单列与四入口底部导航，更多用原生details收纳。深墨蓝、暖纸、低饱和金；无外部图像、无假新闻或模拟前端业务。主体使用Grid/minmax，表格容器可滚，正文17–18px/1.85。所有按钮至少44px，radio只选择不提交。

## 接线与状态

单一fetch client处理JSON错误和超时；WS事件使权威查询失效，最多有限退避重连，不高频拉state。页面hash路由独立于game_id和场景草稿；读档清理旧草稿。场景提交请求带generation/request_id/scene_instance_id/choice_id，网络不确定时保留同一request_id供显式重试。结果在用户继续前保留；409显示错误并刷新权威状态，不替玩家重新选择。

当前协商career.capabilities.advance_world=false表示世界安全推进尚未完成；页面只允许请求选出已满足条件的场景，不触发旧advance或auto。不存在scene时显示真实受限状态。新建显式传story=true,policy=human。不存在接口时显示错误，不回退demo。

目录web/src/api、app、features、styles。npm scripts dev（Vite 5173）、typecheck（tsc --noEmit）、build（tsc && vite build）。Vite同源代理/games（含WS）、/health到127.0.0.1:8080。当前参考站截图证据仍沿24的限制，不宣称像素复刻。

验收：typecheck/build；真实建局与选择/结果/六页、存档。截图1440与390和DOM/横向溢出；未实测项不得标PASS。通过状态另见本文件末尾验收记录。

## 2026-09-11 16:25 验收记录

- npm install成功，依赖锁定：solid-js1.9.9 / TypeScript5.9.3 / Vite6.4.1 / vite-plugin-solid2.11.8。npm run typecheck与npm run build均exit0。
- 新建页1440×900和390×844：已读取DOM并亲看截图，首要动作清楚；scrollWidth均不超过视口，无横向溢出。
- 本地真实服务：浏览器创建story/human局（game1），获取序章两选项，radio仅选中，确认后收到权威结果；切世界再回来结果仍保留；GET save返回200、version11。证据screenshots/career-v1/interaction.json与scene/outcome截图。
- 六页用合法game_id0复核（修正0被JavaScript truthy误判）：1440/390共12张截图和DOM，所有页面无API错误、无页面横向溢出。亲看career390与world1440，暖纸阅读层与紧凑资料层符合24方向。详情见screenshots/career-v1/dom-game.json。
- UI实测时服务端旧版结果仍有team_first技术key与浮点长尾，已反馈主线程让权威效果文案修正；前端不自行解析key或计算后果。
- 未完成验收：所有三弧/世界推进、断线重启回执、200%字号、768/1920截图、手柄。这里不将T11或T12标成ACCEPTED，也不将世界推进受限解释成完整玩法已完成。
- 服务由主线程运行8080；web npm run dev运行5173（会话86430）。本子任务创建Chrome headless9223，仅供本地截图；未使用外部收费视觉服务。

- 16:27补充真实导入UI验收：隔离game1下载JSON→设置选择文件→确认载入→成功提示→回生涯无旧草稿、显示真实暂无场景。证据screenshots/career-v1/load-interaction.json，脚本web/load-smoke.mjs。未触碰其他测试局。
- 16:28最终服务重启后重复一次真实UI链路（game2），仍全部通过；interaction.json/scene-1440-game.png/outcome-1440-game.png已更新。结果现在是中文归档说明、士气+2、关系+4.0、凝聚力+0.1，无技术key与浮点长尾。导入证据仍对应独立game1。
