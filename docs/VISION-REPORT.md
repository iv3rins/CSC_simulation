# CSC 视觉验收报告（Vision Bridge 首轮核验）

> 日期：2026-08-27
> 核验方式：**DeepSeek V4 Flash Vision Exp 视觉桥**（`scripts/vision-bridge.mjs`，opencode-go 网关）
> 核验对象：`docs/screenshots/` 既有 25 张页面截图
> 协议对齐：`docs/QA-AUTOTEST-prompt.md` §5（布局 / 空白 / 重叠溢出 / 配色 四维）

---

## 1. 背景与变更

- **Ox（chrome-devtools-mcp 截图工具）已弃用**，用户指定改用 opencode-go 网关的
  `deepseek-v4-flash-vision-exp` 搭建视觉桥（主模型 `deepseek-v4-flash-vip` 无读图能力）。
- 视觉桥已验证可用：单图/批量核验均稳定（HTTP 200、cost=0），脚本见 `scripts/vision-bridge.mjs`，
  使用说明已同步至 `AGENTS.md` §8。

## 2. 全量判定汇总

| 判定 | 数量 | 说明 |
|---|---|---|
| PASS | 15 | 布局/空白/重叠/配色四维全部合格 |
| WARN | 9 | 存在轻微布局/空白/遮挡问题，不影响核心功能 |
| FAIL | 1 | 布局严重缺陷（11-after-enter-live.png） |
| ERROR | 0 | 无 API 错误 |

**通过率：15/25 PASS（60%）；若将 WARN 视为"可接受但有优化空间"，合格率 24/25（96%）。**

## 3. 逐图核验明细

| 截图 | 判定 | 结论摘要 |
|---|---|---|
| 01-home-onboarding.png | WARN | 布局配色合格，内容区存在较大空白（未创建选手的空状态，可接受） |
| 02-control-page.png | PASS | 布局、配色、内容展示正常 |
| 03-create-form-filled.png | PASS | 创建表单填写页符合验收标准 |
| 04-after-create.png | PASS | 生涯中心符合 HLTV 视觉规范 |
| 05-schedule-page.png | PASS | 我的赛程页正常 |
| 06-live-event-center.png | PASS | LIVE 赛事中心符合预期 |
| 07-live-session-embedded.png | PASS | LIVE 会话内嵌页正常 |
| 09-auto-advance.png | WARN | 右下角浮动通告遮挡赛程信息；职业地位页"影响力"模块布局不一致 |
| 10-live-prompt.png | WARN | 右侧布局偏左、顶部标题被裁切、内容区空白较多 |
| 11-after-enter-live.png | **FAIL** | **内容被压缩在左侧窄列、右侧大面积空白，疑似容器宽度计算错误** |
| 12-matchday-prompt.png | PASS | 比赛日 LIVE 提示页合格 |
| 13-live-event-center-full.png | PASS | LIVE 中心完整页合格 |
| 14-live-session-open.png | PASS | LIVE 会话打开页正常 |
| 15-after-direct-click.png | PASS | 直接点击进入后正常 |
| 16-live-prompt-matchday.png | WARN | 顶部标题截断、底部浮层遮挡 |
| 17-live-prompt-via-autorun.png | WARN | 内容区大面积空白、信息密度不足 |
| 18-live-session-opened.png | WARN | 左侧卡片与顶部导航重叠遮挡 |
| 19-live-round-1.png | PASS | 回合 1 正常 |
| 20-live-round-2.png | PASS | 回合 2 正常（仅顶部细微截图瑕疵） |
| 21-after-exit-live.png | WARN | 内容区大面积空白、信息密度低 |
| 22-after-exit-page.png | WARN | 主内容区大面积空白、页面空洞 |
| 30-game18-home.png | WARN | 右下角 Toast 遮挡底部赛事预告卡片 |
| 31-p1-live-session.png | PASS | 玩家 1 LIVE 会话合格 |
| 32-p1-event-page-live.png | PASS | 玩家 1 赛事页 LIVE 合格 |
| 33-p1-live-progress.png | PASS | 玩家 1 LIVE 进度合格 |

## 4. 问题归类与根因分析

### 4.1 唯一 FAIL（11-after-enter-live.png）
- **现象**：主体内容被压缩在左侧窄列，右侧大面积空白，左侧有竖排文字被截断，中间出现垂直橙色线条。
- **疑似根因**：容器 flex/grid 布局未正确占满宽度，或该截图拍摄于 LIVE 进入瞬间的渲染中间态
  （侧边栏/内容区切换时宽度计算错位）。
- **处置建议**：在 `frontend/src` 定位 LIVE 进入后的容器样式，复现"进入 LIVE → 立即截图"确认是否为
  瞬时态；若稳定复现则修复容器宽度（`max-width: 1240px; margin: 0 auto` 类约束）。

### 4.2 WARN 共性（9 张）
| 类型 | 出现截图 | 说明 |
|---|---|---|
| 内容区空白/信息密度低 | 01, 10, 17, 21, 22, 30 | 多为空状态或 LIVE 中间态，建议补数据卡片/占位 |
| 顶部标题/导航被裁切 | 10, 16, 18 | 顶部文字与导航重叠截断，建议调层级/z-index |
| 浮层/Toast 遮挡内容 | 09, 16, 30 | 右下角提示浮层遮挡赛程卡片，建议调位置或自动消失 |
| 布局偏左/未居中 | 10 | 内容区未按桌面 max 1240px 居中 |

## 5. 结论

- **视觉桥搭建成功**，DeepSeek V4 Flash Vision Exp 读图能力正常，四维核验（布局/空白/重叠/配色）
  输出稳定，可作为 Ox 的正式替代，用于后续 UI 视觉回归（含 1440px / 390px 视口各一轮）。
- **既有截图存在 1 个 FAIL（11-after-enter-live）与 9 个 WARN**，多为 LIVE 进行中/空状态的
  中间态截图，核心功能页面（赛程、赛事中心、创建流程、回合计分）全部 PASS。
- **建议后续动作**：
  1. 复现并修复 11-after-enter-live 的容器宽度问题（若稳定复现）；
  2. 优化 9 张 WARN 的浮层遮挡与空白密度；
  3. 按 QA-AUTOTEST §5 协议补拍 1440px / 390px 双视口全页面截图，用视觉桥做一轮正式视觉回归。

## 6. 产物路径

- 视觉桥脚本：`scripts/vision-bridge.mjs`
- 核验数据：`docs/screenshots/vision-summary.json`
- 本报告：`docs/VISION-REPORT.md`
- 使用说明：`AGENTS.md` §8（已同步更新）
