# 数据文件说明（scripts/data/）

## hltv_detailed_players.csv（原始来源 #1）

- 来源：GitHub `StrandedPond/hltv_scraper`（GitHub Actions 每日/每周自动更新）
  https://github.com/StrandedPond/hltv_scraper/blob/main/hltv_detailed_players.csv
- 内容：121 名选手 × 14 张图（All + 13 图池）的生涯统计
  （`name, team, map, overall_rating, ...` 宽表）
- 抓取窗口：全生涯累计（HLTV 玩家页 `?csVersion=CS2`）

## ratings_all.csv（中间产物）

- 由 hltv_detailed_players.csv 过滤 `map=All` 行生成：
  ```powershell
  Import-Csv hltv_detailed_players.csv |
    Where-Object { $_.map -eq "All" } |
    Select-Object name, overall_rating |
    Export-Csv ratings_all.csv -NoTypeInformation -Encoding UTF8
  ```
- 然后合并进评级先验：
  ```bash
  node scripts/scrape_hltv.mjs --input scripts/data/ratings_all.csv --columns name,overall_rating --merge
  ```

## csapi_players_agg.json（预训练数据集，原始来源 #2）

- 来源：https://www.csapi.de（HLTV 数据中转 API，免费、无需密钥、每日刷新；
  HLTV 官网直连被 Cloudflare 拦截，此 API 为沙箱内可达的替代数据源）
- 抓取管线：`node scripts/fetch_csapi.mjs`
  - 分页拉取 `/matches/?offset=N`（共 2,566 场，窗口 2025-10-14 ~ 2026-08-13）
  - 逐场拉取 `/matches/{id}/stats` 记分牌（10 人 K/D/ADR/KAST/Rating）
  - 聚合为每名选手的窗口统计：`rating / kd / adr / kast / winrate / matches`
  - **对手强度加权**（对手 HLTV 排名 Top10→1.5 / Top20→1.2 / Top40→1.0 /
    Top80→0.7 / 其余→0.5）：模仿 HLTV 赛季 Rating 的赛事加权，避免强队
    刷低级别赛事导致 Rating 虚高、IGL/角色选手被误抬档位
  - 限速自保护：令牌桶 2.5 req/s + 429 指数退避（实测 API 约 30~40 请求/
    10 秒滚动窗口）+ 磁盘缓存 `csapi_stats_cache.json` 断点续传
- 用途：训练/校准模拟器的实力先验（`assets/player_ratings.json` 由此重写）、
  拟合 Top20 评分权重、检验真实选手的年龄-实力曲线
- 注意：该 API 为社区中转镜像，数据以抓取时点为准；重跑 `--limit N` 可冒烟

## csapi_stats_cache.json（抓取缓存，可删除）

- 逐场记分牌的本地缓存（matchId → stats），断点续传用；删除后下次全量重抓

## 覆盖状态

- 资产合并后：`player_ratings.json` 覆盖 standings 200 名阵容（目标 100%，
  窗口 ≥3 场的活跃选手全部纳入；缺阵/退役选手保留手编值）
- 数据许可：HLTV 公开统计页面数据，经第三方公开仓库/中转 API；本项目仅取
  `(昵称, rating)` 等数值先验，不复制原始页面内容


## hltv_ranking_2026_08_10.md / hltv_top20_2025.md / hltv_events_archive_2026_08.md

- 来源：HLTV 对应页面的只读 Markdown 镜像（直接抓取时被 Cloudflare 挑战拦截；
  同一 URL 的内容可回原站核对）
- 用途：`node scripts/model_hltv_strength.mjs` 解析 2026-08-10 世界排名（213 队、
  积分与 roster）、TOP20 2025 与赛事档案，产出
  `assets/hltv/strength_model_2026_08_10.json` 与
  `docs/HLTV-2026-STRENGTH-MODEL.md`

## rating_profile.json（新选手实力分布模型，由校准脚本生成）

- 生成管线：`node scripts/calibrate_profile.mjs`
  - 输入：`csapi_players_agg.json`（全职业分布）+ `roles_baseline.json`
    （年龄/角色）+ standings（世界名单）
  - 输出：`assets/rating_profile.json`——`global`（全职业均值±σ）、
    `roles`（按角色分布：AWP/ENTRY 最高、IGL 最低）、`ages`（年龄段分布）、
    `rookie`（新秀采样参数：均值偏移 -0.02 + 标准差缩放）
- 用途：Rust `RatingProfile` 在年度人口更新时按真实职业分布采样新秀起始档位
  （多数青训 Tier3/4，~1.6% 直接 Tier0——donk 级天才），替代旧的
  「新秀固定 Tier4」；潜力长尾（19% 天才比例）由潜力系统单独负责
- 验证：`node scripts/verify_ratings.mjs`（评级先验）+ population 单元测试
  （新秀档位分布）+ 20 年长程推进（213 名新秀实力 45~96 拉开）
