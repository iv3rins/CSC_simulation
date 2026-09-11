# CSC 生涯前端

Solid + TypeScript + Vite。需要同时运行 Rust 服务端；正式页面只读取真实游戏，没有演示数据回退。

```powershell
cd web
npm ci
npm run dev
```

打开 http://127.0.0.1:5173/ 。Vite 把 `/games`（含 WebSocket）与 `/health` 代理到 `http://127.0.0.1:8080`。端口占用会明确失败，不自动切换端口。关闭启动命令所在终端或 Ctrl+C 即停止。

```powershell
npm run typecheck
npm run build
```

六页为生涯、赛程、战队、世界、档案、设置；新建显式使用 story/human，选择显式确认。当前服务端 `capabilities.advance_world=false` 时不可推进日期，只能阅读当前可用场景；不能完整跑完职业生涯。保存使用现有 JSON 接口，导入前请先下载当前存档。

`smoke.mjs` 连接独立 Chrome CDP 9223，输出六页 DOM 和 1440/390 截图；`CSC_GAME_ID` 必须指向现有局，脚本只查询。`interact.mjs` 则明确创建一个独立测试局并提交一个选择，检查返回结果与保存。它们要求已经启动本地真实服务及 Chrome，不会替换 API 为 mock。测试资料保留在 `docs/screenshots/career-v1/`。

截图核验不等于所有剧情/回档/断线情境已经验收，完整任务状态见项目 DEV 文档。
