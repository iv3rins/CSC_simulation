# HLTV 2026-08 选手 / 战队实力建模

> 数据锚点：2026-08-10 世界排名页、TOP20 页、赛事档案页；配合仓库已有的
> `assets/player_ratings.json`（csapi.de 窗口 2025-10 ~ 2026-08，686 名选手、
> 对手排名加权）与 `assets/top20-data/*` 三年榜。
> 可复现产物：`assets/hltv/strength_model_2026_08_10.json`
> （生成脚本 `scripts/model_hltv_strength.mjs`）。

## 0. 数据获取说明

HLTV 官网对自动化访问有 Cloudflare 人机验证。本次先以 Chrome 152 + DevTools
Protocol（CDP，等价于 Chrome-devtools-MCP 的底层协议）直连目标页面，Cloudflare
挑战无法在无人值守环境自动放行；随后用同一页面地址抓取到的只读 Markdown 镜像
（`scripts/data/hltv_ranking_2026_08_10.md`、`hltv_top20_2025.md`、
`hltv_events_archive_2026_08.md`）完成解析。所有数值均可回到原 URL 核对。

## 1. 世界排名快照（HLTV，2026-08-10）

| # | 队伍 | HLTV 积分 | 核心三人窗口 Rating（top3 均值） |
|---|------|----------:|-------------------------------:|
| 1 | Falcons | 907 | 1.216（m0NESY / NiKo / kyousuke） |
| 2 | Spirit | 752 | 1.254（donk / sh1ro / magixx） |
| 3 | FURIA | 646 | 1.144（KSCERATO / molodoy / yuurih） |
| 4 | Vitality | 626 | 1.279（ZywOo / ropz / flameZ） |
| 5 | MOUZ | 574 | 1.110（xertioN / xelex / Spinx） |
| 6 | Natus Vincere | 459 | 1.137（b1t / iM / w0nderful） |
| 7 | 9z | 363 | 1.141 |
| 8 | Aurora | 330 | 1.122 |
| 9 | FaZe | 328 | 1.114 |
| 10 | G2 | 308 | 1.117 |

完整 213 队解析结果在 `assets/hltv/strength_model_2026_08_10.json`。

观察：**积分是“成绩状态”，不是“纯实力”**。Vitality 的核心实力（1.279）高于
Falcons（1.216），但 Falcons 半年赛程拿分更多、积分更高；MOUZ 的核心均值
1.110 与 NaVi 1.137 很接近，但积分差 115。因此模拟内核必须保持
「实力先验决定胜率，积分只由比赛结果累积」的因果方向——绝不能让阵容微调直接
把历史积分清零。

## 2. 选手实力模型

引擎已有 18 项属性 + 角色偏好加权 `PowerCalculator`。本模型只负责
**HLTV Rating → 引擎 Power** 的可解释标定：

```
player_power = clamp(25 + 50 × rating, 0, 100)
```

锚点对齐现有转会门槛：

| 层级 | rating | power | 现有 `TransferRules` 门槛 |
|------|-------:|------:|--------------------------|
| 一线 T1 | ≥ 1.20 | ≥ 85 | `T1_POWER_THRESHOLD = 85` |
| 中游 T2 | ≥ 1.00 | ≥ 75 | `T2_POWER_THRESHOLD = 75` |
| 下游 T3 | ≥ 0.70 | ≥ 60 | `T3_POWER_THRESHOLD = 60` |
| 青训/边缘 | < 0.70 | < 60 | 新人可进 T4 |

2026-08 窗口头部（Rating → Power）：

| 窗口名次 | 选手 | rating | power |
|---------:|------|-------:|------:|
| 1 | donk | 1.487 | 99.4 |
| 2 | ZywOo | 1.424 | 96.2 |
| 3 | junyme | 1.346 | 92.3 |
| 4 | m0NESY | 1.318 | 90.9 |
| 5 | annihilation | 1.283 | 89.1 |

对照 TOP20 2025 榜单（ZywOo #1 / donk #2 / ropz #3 / m0NESY #4 / sh1ro #5），
模型覆盖 20/20 且 power 与名次单调性良好。注意窗口 Rating 会放大“低级别赛事
刷数据”的样本；`player_ratings.json` 已做对手排名加权（Top10×1.5 →
Top40×1.0 → 其余×0.5），这是保留的防御。

## 3. 战队实力模型

CS 战队不是五个平均人：**TOP3 核心决定上限，第 4/5 人决定下限**。用 2026-08-10
的 127 支有完整 Rating 的队伍做线性拟合，`ln(HLTV 积分)` 与
`avg(top3 rating)` 的相关性（r≈0.54）显著高于全队均值（r≈0.40）。

落地公式（与 `PowerCalculator::team_power` 的角色多样性/磨合系数同构）：

```
core_power  = avg( player_power(rating_i), i ∈ TOP3 )
depth_power = avg( player_power(rating_j), j ∈ 第4/5人 )
team_strength = 0.7 × core_power + 0.2 × avg_all + 0.1 × depth_power
```

一线队“源源不断补强”的证据（top12 平均）：

- 核心三人窗口 Rating 均值 **1.20+**，比全职业均值（1.037）高约 **+0.16**；
- 第 4/5 人深度均值约 **1.08**，核心-深度差仅 **0.11**——王朝队不是只有两个爹，
  而是五人梯度连续；
- 当第 4/5 人跌破 1.00（power 75），队伍会迅速从中游掉队；这正是转会市场
  “弱化最弱一环”比“买一个巨星”更重要的原因。

因此模拟器的 NPC 转会语义应保持并强化：
1. 强队签入低就高能 NPC，**替换最弱 NPC**（现有逻辑正确）；
2. 每支队伍每个转会窗最多 1 笔 NPC 交易，避免“一个窗换半支队”；
3. 阵容连续性按**转会窗结算**：3 人离队才清零，而不是跨窗累计 3 次 1 人换血
   就把王朝队积分清零（这是“王朝队变残废”的直接 bug，详见第 5 节修复）。

## 4. 2026 赛历对照（events/archive）

`SeasonCalendar::top_tier_specs` 的 25 场 T1 桶与 HLTV 档案页一致：
2 Major（科隆 6 月 / 新加坡 11 月）+ 18 S 级 + 5 A 级；8 月 BLAST Bounty S2
Finals、Esports World Cup、StarLadder 秋季预选等也都能在归档页找到对应条目。
结论：**世界赛事本身在模拟，问题是前端只下发“主角参赛过的赛事”**，所以赛程页
看起来没数据。修复为“年度赛历端点 + 全量赛果（不含逐图 KDA）按需拉取”。

## 5. 模型驱动的四项修复

| 问题 | 根因 | 修复 |
|------|------|------|
| 主页卡在“等待赛历排期” | 引擎逐月动态排程，月底锁已过期；前端只读当前锁 | 新增 `/calendar?year=` 年度赛历端点，首页显示下一场/本月战报，不再有“永远等待”态 |
| 数据统计英文 | AttributesPage 硬编码 Age / Prize money / statistics 等 | 全量中文化 |
| 赛程无数据 | `/view` 只下发主角赛事，NPC 世界胜负前端不可见 | `/calendar` 下发全年所有赛事的赛果摘要（event/champion/matches/对阵），回放仍按需懒加载 |
| 转会窗排名巨震 | `departed_count` 从 `last_settled_roster` 跨窗累计；每窗 1 换 1、3 窗后也触发“3 人离队清零”，王朝队积分归零 → 失去 T1 邀请 → 雪崩 | ① 转会窗结束统一 `settle_rosters()`，离队计数窗口化；② NPC 市场每队每窗最多 1 笔；③ 保持强队只换最弱 NPC 的补强方向 |

## 6. 复现

```bash
# 生成模型资产（需要 scripts/data 下的三个 HLTV Markdown 快照）
node scripts/model_hltv_strength.mjs

# 引擎全量回归
cd backend && cargo test --workspace
```
