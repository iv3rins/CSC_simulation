# CSC — CS2 职业选手生涯模拟器

[![CI](https://github.com/iv3rins/CSC_simulation/actions/workflows/ci.yml/badge.svg)](https://github.com/iv3rins/CSC_simulation/actions/workflows/ci.yml)
[![GitHub stars](https://img.shields.io/github/stars/iv3rins/CSC_simulation?style=flat-square)](https://github.com/iv3rins/CSC_simulation/stargazers)
[![GitHub forks](https://img.shields.io/github/forks/iv3rins/CSC_simulation?style=flat-square)](https://github.com/iv3rins/CSC_simulation/network/members)
[![GitHub issues](https://img.shields.io/github/issues/iv3rins/CSC_simulation?style=flat-square)](https://github.com/iv3rins/CSC_simulation/issues)
[![Top language](https://img.shields.io/github/languages/top/iv3rins/CSC_simulation?style=flat-square)](https://github.com/iv3rins/CSC_simulation)
[![License: not specified](https://img.shields.io/badge/license-not%20specified-lightgrey?style=flat-square)](#许可证)

> 玩家扮演一名 CS2 职业选手，完成整个职业生涯：从青训新秀到世界冠军，
> 经历训练养成、场内决策、伤病、转会、代言与退役。
> **确定性模拟内核 + 事件驱动决策流**：同种子 + 同决策序列 = 完全一致的世界，
> 可复现、可回放、可做服务端权威校验。

---

## 技术栈

| 层 | 技术 | 位置 | 状态 |
|---|---|---|---|
| **后端** | Rust（workspace 16 crates，纯同步逻辑库，零 IO） | `backend/` | ✅ 模拟内核 + 服务端 + CLI + WASM |
| 服务端 | Rust + axum（REST/WS + channel 决策源桥接） | `backend/crates/csc-server` | ✅ 已建（M9） |
| CLI | 交互式命令行（决策点渲染 + stdin 决策源） | `backend/crates/csc-app` | ✅ 已建（M10） |
| WASM | 本地单机模式 v1（auto 推演 + 存档/事件流） | `backend/crates/csc-wasm` | ✅ 已建（M10） |
| ~~前端~~ | ~~TypeScript（Solid，纯表现层）~~ | ~~`frontend/`~~ | 🗑 **已移除（2026-09）**：前端展示层已从仓库删除，前后端彻底分离；UI 由独立展示层项目按 `docs/analysis/` 协议契约重建 |

后端核心（`backend/crates/csc-core` 及以下）**零异步、零 IO**：同一份代码可编译
`wasm32` 跑本地单机模式，也可挂在服务端跑权威模式。

## 目录结构

```
cs_career_simulation/
├── backend/                    # Rust 后端（产品）
│   ├── ARCHITECTURE-MAPPING.md # Kotlin→Rust 转写蓝本（依赖图/类型映射/设计决策，历史文档）
│   ├── Cargo.toml              # workspace（16 crates）
│   ├── crates/
│   │   ├── csc-core/           # 总引擎：装配/推进编排/GameState 聚合根/查询层
│   │   ├── csc-tournaments/    # 赛事系统：排程/赛制/对局决策循环/结算
│   │   ├── csc-systems/        # 转会/人口/伤病/队内关系/经济 五引擎
│   │   ├── csc-simulation/     # 纯函数规则层（胜率/成长/体能/经济/化学/比赛）
│   │   ├── csc-entities/       # 实体 arena（ID 引用）+ 属性领域
│   │   ├── csc-decision/       # 决策点/决策源/决策日志（两级决策流）
│   │   ├── csc-events/         # 世界事件流 WorldJournal（事件溯源）
│   │   ├── csc-career/         # 生涯档案（按 PlayerId 键控）
│   │   ├── csc-vrs/            # VRS 排名（官方两层模型：静态 Seed + 动态 ELO）
│   │   ├── csc-domain/         # 跨系统值对象（零依赖）
│   │   ├── csc-time/           # 模拟时钟 + 时间窗口
│   │   ├── csc-util/           # xoshiro256** 确定性 RNG + 稳定 ID + SimError
│   │   ├── csc-server/         # axum REST/WS 服务端（M9）
│   │   ├── csc-app/            # 交互式 CLI（M10）
│   │   └── csc-wasm/           # WASM 绑定层（M10）
│   └── tools/                  # 跨语言 golden 序列生成脚本
└── assets/                     # 外部数据资产（standings/roles_baseline）
```

## 快速开始

```bash
# 全量测试（覆盖可复现性/存档往返/各子系统不变量）
cd backend
cargo test --workspace

# 静态检查（0 警告为门禁）
cargo clippy --workspace --all-targets

# 长程门禁（20 年浸泡 + 吞吐基准；debug 约 10s）
cargo test -p csc-core --test soak -- --ignored

# 交互式游玩（决策点渲染 + stdin 决策）
cargo run -p csc-app -- ../assets

# 启动服务端（REST/WS；供任何展示层/CLI 对接）
cargo run -p csc-server -- ../assets 8080

```

CI（`.github/workflows/ci.yml`）同时跑后端 `cargo test --workspace` +
`cargo clippy -- -D warnings` 。

### 世界规模与存档压缩

- 开局世界：128 支队伍（真实 TOP40 + HLTV 次级别队伍，640 名真实昵称选手）。
  原始 40 队文件可重新扩编：
  ```bash
  node scripts/expand_standings.mjs --teams 128   # 或 160（超出 128 会用确定性占位名补齐）
  ```
- 存档默认走 gzip：客户端下载 `.json.gz`，服务端 `GET /games/{id}/save/gzip` 流式压缩，
  `POST /games/{id}/load/gzip` 解压读档。12 个月实测 9.28MB JSON → 0.89MB gzip（约 1/10）。
- **文案资产化（2026 结构拆分）**：全部叙事/直播/评语文案的单一事实源是
  `assets/text/zh-CN.json`（约 260 键，`模块.场景.变体` 命名 + `{0}` 占位符）。
  后端经 `csc-text` crate 编译期嵌入为默认包（零 IO、wasm32 兼容），服务端/CLI
  启动时读取 `assets/text/zh-CN.json` 按键覆盖（改文案不用改代码）；

### 服务端 REST 协议速览（供任何展示层对接）

```
POST   /games                        { seed?, policy?:"auto"|"human", player_name? } → { game_id }
GET    /games/{id}/state             → GameState（完整快照；存档/调试用，长生涯可达数十 MB）
GET    /games/{id}/view              → ClientState（轻量视图；网页端高频刷新用）
GET    /games/{id}/calendar?year=N  → 年度完整赛历（计划 + 全部队伍赛果摘要）
GET    /games/{id}/replay            ?event=&date=&index= → { series }（单场直播懒加载）
GET    /games/{id}/summary           → 世界概览
GET    /games/{id}/journal?since=N   → { events, next_seq }（增量事件流）
GET    /games/{id}/archive           → 生涯档案
GET    /games/{id}/save | POST /load → 存档往返（JSON）
GET    /games/{id}/save/gzip | POST /load/gzip → gzip 压缩存档（长生涯体积约 1/10）
POST   /games/{id}/advance           { months? } → auto: [StepSummary] / human: 202
POST   /games/{id}/advance-season    {} → 模拟到下一个赛季边界（FIFA 生涯模式主入口）
GET    /games/{id}/transfers/offers  → 主动转会候选
POST   /games/{id}/transfers/sign    { team_signature } → 立即签约
POST   /games/{id}/actions/statement { tone } → 主动发言（CONFIDENT/HUMBLE/PROVOKE）
POST   /games/{id}/actions/match-plan{ style } → 主动 Major BP（AGGRESSIVE/BALANCED/CONSERVATIVE）
GET    /games/{id}/decisions/pending → 待决策批次（兼容旧 monthly 模式）
POST   /games/{id}/decisions         { decisions:[{point_id, option_id}] } → 提交
GET    /games/{id}/training/options  → 训练计划选项（玩家主动触发）
POST   /games/{id}/training          { focus } → 排定下月训练计划
POST   /games/{id}/finance           { action:"BUYOUT"|"INVEST"|"SKIN", param? } → 资金运用（买断自己换队 / 投资自己 / 购买 CS 饰品；v7）
WS     /games/{id}/ws                decisions/step/journal 事件 + decide/advance 指令
```

**FIFA 生涯模式要点**：展示层主流程不再逐月推进，而是 `POST /advance-season`
一路模拟到赛季边界——赛季内由服务端 `CareerDecisionSource` 稳妥自动处理，
不弹决策事件；赛季间由玩家在「球员办公室」**主动选择**换队 / Major BP /
发言 / 训练。旧 monthly + WS 决策流保留为兼容协议（供 CLI/展示层）。
**训练改为主动触发**（2026）：训练不再进月度批次；玩家任意时间
`POST /games/{id}/training` 排定计划，下月月初应用（存档 v3 可复现）。
`step.season_end` 在 month%12==0 时为 `true`，展示层据此暂停自动推进并弹赛季总结。

### 轻量视图性能契约（2026 修订；原供网页端高频刷新）

`GameState` 是**完整存档聚合根**（20 年生涯 JSON ≈68MB），网页端绝不按月轮询它：

- `GET /view`：主角 + roster + 主角赛事（**不下发逐图 series**）+ 主角年度标量。
  实测 15 年生涯 51.9MB → **1.0MB**（约 2%），10 年 34.4MB → 0.7MB；
- `GET /journal?since=`：事件流增量同步（WS 断线兜底）；
- `GET /replay`：单场直播回放懒加载（`view` 只下发 `replayable` 布尔向量）；
- 服务端 `advance` 批次内不再逐月 clone 完整快照：完整 `GameState` 只在批次
  结束/存档请求时构建；Human 月步只更新轻量视图。
- 性能画像：`cargo run -p csc-core --release --example client_profile -- ../assets`。

## 架构

### 依赖方向（cargo 编译期强制无环）

```
第 0 层  csc-domain（零依赖）  csc-util（RNG/ID/SimError）  csc-time（时钟）
第 1 层  csc-entities（arena + ID）  csc-events（事件流）
第 2 层  csc-simulation（纯函数规则）  csc-vrs（排名库）  csc-career（档案）
第 3 层  csc-decision（决策点 + 日志）
第 4 层  csc-systems（transfer/population/injury/chemistry/economy）
第 5 层  csc-tournaments（赛事系统，消费 systems）
第 6 层  csc-core（总引擎 + GameState + 查询层）
```

### 三条铁律

1. **确定性核心 + 外部决策注入**：引擎内部唯一随机源 `Xoshiro256StarStar`
   （与 Kotlin 位级一致，可存档）；玩家意图经 `DecisionSource` 接口注入；
   **种子 + 决策日志 = 完全一致的世界**。
2. **引擎是编排壳，公式在规则层**：系统包引擎只做状态变更与编排，
   一切数值规则在 `csc-simulation/*`（纯函数、无状态）。
3. **csc-core 只接线**：装配（`Engine::load_from_standings`）/ 推进
   （`advance_month` 8 阶段管线）/ 存档（`snapshot`/`restore`）。

### 两级决策流（游戏性的核心机制）

```
世界级批次（每月）                    比赛级现场（每图）
训练重点 / 转会窗（每 4 月）/         场内干预（风格/暂停）/
伤病处理 / 代言邀约                   队友失误反应（鼓励/指责/无视）
      └──────────────┬──────────────┘
                     ▼
        DecisionRecorder（决策日志 + 事件镜像）
```

### 存档契约（GameState v1）

- 全量纯值快照：实体 arena + VRS 库 + 赛事结果 + 年度累计器 + RNG 状态 + 决策日志 + 事件流 + 档案
- `version` 字段 + `deny_unknown_fields` + `migrate()` 迁移挂载点——格式演进显式化
- 外部输入一律 `Result<T, SimError>`（坏资产/坏存档显式报错，不 panic、不静默）

## 已完成（2026-08）

- ✅ M0–M10：domain→util 全量转写 + server/app/wasm（约 19K 行）
- ✅ **ID 全覆盖**：事件/决策点/档案/锁全部携带 `PlayerId`/`TeamId`（名字降级为展示字段）
- ✅ **存档版本化**：`GameState.version` + 未知字段拒绝 + 迁移挂载点
- ✅ **概率判定整数化**：`roll_bp` 基点判定替换浮点分支（跨语言事件序列可对齐）
- ✅ **SimError + 资产语义校验**：`parse_json_checked`（负积分/空阵容等显式报错）
- ✅ **浸泡测试**（12 月默认 + 20 年门禁，捕获并修复 3 个长程缺陷）：
  1. `next_i32_bound` 负随机数（Kotlin 重写版同源缺陷，名字池负下标越界）；
  2. `run_player_series` 不填 `loser_sig` → VRS ELO 结算收到空签名；
  3. 新秀名字池（40 个）耗尽后查重循环死转 → 撞名加数字后缀
- ✅ **服务端**（csc-server）：每局一个模拟线程 + WS 决策面板协议 + 存档 API
- ✅ **CLI**（csc-app）：stdin 决策源交互闭环（UX 设计稿到位前的可玩入口）
- ✅ **WASM v1**（csc-wasm）：auto 推演 + 存档/事件流（交互模式待步骤化状态机）
- ✅ ~~前端~~（M11，历史交付，2026-09 移除）：11 页 + 决策面板 + 直播回放 + WS 闭环；`/view` 轻量视图 +
  增量 journal + 懒加载 replay 支撑长生涯网页端游玩
- ✅ NPC 转会市场（对手 AI）+ series 对阵决策（同走 `DecisionSource` 管线）
- ✅ **2026 真实赛历密度**：全年 25 场 T1 桶（2 Major + 18 S + 5 A，对齐 HLTV
  Top Tier Calendar）+ 60 场 T2 + 96 场 T3；月度世界批次时间戳收束到月终日
- ✅ ~~HLTV 风格前端统一~~（历史交付，随前端移除归档）：浅灰画布/白卡片/HLTV 蓝 `#2d7dd2`；队标映射补全
  128 支开局队伍（真实 TOP40 + HLTV 次级别队伍）并修复短别名误命中；自动模拟自动提交决策，不再卡弹窗；
  世界新闻两行式布局修复文字溢出
- ✅ **存档压缩 + 世界扩充**：128 支开局队伍（640 名真实昵称选手；`scripts/expand_standings.mjs`
  可配置规模）；`/save/gzip` 与 `/load/gzip` 存档端点（实测 9.3MB JSON → 0.89MB gzip，
  约 1/10）；修复 128 队规模暴露的 NPC 转会窗后 VRS 排名缓存滞后

## 路线图（按依赖排序）

| 阶段 | 内容 | 状态 |
|---|---|---|
| M11 | ~~TypeScript 前端（Solid）~~：决策面板 / 赛事回放 / 生涯档案（2026-09 移除） | ✅ 已交付（历史） |
| M12 | FIFA 式赛季循环 + ~~前端精简~~ + 赛事分级修复 + 主动操作（换队/BP/发言） | ✅ 已交付（历史） |
| WASM v2 | 步骤化状态机（`EngineStep`）→ 浏览器内交互模式 | ⏳ 未建 |
| 内容管道 | 数据驱动赛事日历 / 平衡参数表 | ⏳ 未建 |
| 人生层 | 关系网络 / 媒体舆论 / 场外生活 / 经济开支 / 随机叙事事件 | ⏳ 待设计 |

## 文档索引

- [backend/ARCHITECTURE-MAPPING.md](backend/ARCHITECTURE-MAPPING.md) — 转写蓝本（历史文档）：crate 依赖图、类型映射、关键设计决策（D1–D7）
- [backend/crates/csc-core/README.md](backend/crates/csc-core/README.md) — 核心引擎用法
- 各 crate 自带 README（构建/对照说明）

## 许可证

当前仓库尚未包含 `LICENSE` 文件，因此 GitHub 不会将其标记为 MIT、Apache-2.0 等正式开源协议。
确定协议后，请在仓库根目录添加对应的 `LICENSE` 文件，并同步更新顶部徽章。
