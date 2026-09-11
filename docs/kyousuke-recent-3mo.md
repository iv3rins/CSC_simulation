# kyousuke — 最近 3 个月数据（2026-06-08 ~ 2026-09-05）

> 来源：csapi.de（HLTV 数据中转 API，真实实时源）。选手：Maksim "kyousuke" Lukin（HLTV id 24177），现役 Falcons。
> 抓取时间窗：`2026-06-08` → `2026-09-05`（Falcons 在该窗内的最后一场为 09-05）。全部 16 场 Falcons 比赛 kyousuke 均有出场。

## 窗口总览（16 场 / 全队 bo3+ 整场 "All" 记分）

| 指标 | 数值 |
|---|---|
| 出战场数 | 16 / 16 |
| 总击杀 / 总死亡 | 665 / 600（K/D **1.108**） |
| 场均 Rating | **1.203** |
| 场均 ADR | 84.0 |
| 场均 KAST | 75.1% |

## 逐月拆解（场均 Rating）

| 月份 | 场次 | 场均 Rating |
|---|---|---|
| 2026-06（IEM Cologne Major 2026 主赛事） | 6 | 1.183 |
| 2026-07（BLAST Bounty S2） | 1 | 1.260 |
| 2026-08（EWC + BLAST Open Porto 资格/小组） | 7 | 1.206 |
| 2026-09（BLAST Open Porto 2026 淘汰赛） | 2 | 1.225 |

## 单场明细（按日期升序）

| 日期 | 赛事 | 对阵 | bo | K-D | ADR | KAST% | Rating |
|---|---|---|---|---|---|---|---|
| 06-11 | IEM Cologne Major 2026 | vs G2 | 3 | 52-53 | 81.7 | 80.8 | 1.14 |
| 06-12 | IEM Cologne Major 2026 | vs BetBoom | 3 | 21-29 | 61.6 | 66.0 | 0.85 |
| 06-14 | IEM Cologne Major 2026 | vs Natus Vincere | 3 | 56-45 | 90.8 | 80.3 | 1.16 |
| 06-19 | IEM Cologne Major 2026 | vs Vitality | 3 | 52-50 | 83.7 | 77.8 | 1.43 |
| 06-20 | IEM Cologne Major 2026 | vs Spirit | 3 | 60-57 | 81.7 | 75.9 | 1.18 |
| 06-21 | IEM Cologne Major 2026 | vs FURIA | 5 | 55-43 | 93.0 | 76.2 | 1.34 |
| 07-22 | BLAST Bounty 2026 S2 | vs 100 Thieves | 3 | 50-38 | 84.4 | 80.3 | 1.26 |
| 08-12 | Esports World Cup 2026 | vs K27 | 1 | 15-10 | 89.3 | 78.9 | 1.59 |
| 08-13 | Esports World Cup 2026 | vs Astralis | 3 | 36-28 | 84.1 | 75.0 | 1.20 |
| 08-20 | Esports World Cup 2026 | vs The MongolZ | 3 | 44-22 | 113.8 | 81.6 | 1.78 |
| 08-21 | Esports World Cup 2026 | vs Legacy | 3 | 23-32 | 62.3 | 62.8 | 0.71 |
| 08-27 | BLAST Open Porto 2026 | vs Lynn Vision | 3 | 46-47 | 88.2 | 77.4 | 1.05 |
| 08-29 | BLAST Open Porto 2026 | vs Legacy | 3 | 50-44 | 94.3 | 72.1 | 1.32 |
| 08-31 | BLAST Open Porto 2026 | vs MOUZ | 3 | 34-41 | 67.0 | 61.1 | 0.79 |
| 09-04 | BLAST Open Porto 2026 | vs G2 | 3 | 36-26 | 88.8 | 80.5 | 1.41 |
| 09-05 | BLAST Open Porto 2026 | vs Spirit | 3 | 35-35 | 78.6 | 75.6 | 1.04 |

## 要点

- 高点：08-20 对阵 The MongolZ（EWC）Rating **1.78**、ADR 113.8；08-12 vs K27 Rating 1.59。
- 低点：08-21 vs Legacy Rating 0.71、ADR 62.3；06-12 vs BetBoom 0.85。
- 全窗稳定在 **Rating ~1.20 一线**（对比项目内 32 场全窗口聚合 1.178，本 3 个月窗口略高），为队伍稳定第一梯队火力点。

## 相关文件
- `scripts/data/kyousuke_recent.json` — 结构化数据（含每场全部记分字段）
- `scripts/data/kyousuke_recent.mjs` / `kyousuke_scope.mjs` — 抓取/聚合脚本
