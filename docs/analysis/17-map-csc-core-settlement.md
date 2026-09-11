# 17-map-csc-core-settlement.md（底层探针·完整读取）

> 读取方式：逐行完整读取（5 段，段间重叠），非 grep 预测。读取日期：2026-09-05。

## 1. 文件职责
- 模块文档原文：`跨年结算编排（Kotlin YearlySettlement.kt 转写）：日历年变化时的年度世界演化。职责：颁奖 → 生涯归档 → 成长 → 人口 → 薪资 → 代言复位 → 合同结转，阶段顺序是业务契约。`
- 一句话：**年度结算编排器**——每年结束时按固定阶段序驱动整个世界前进一步。无跨调用状态，全部依赖参数化注入。

## 2. 公开面
| 项 | 签名 | 行号 |
|---|---|---|
| struct | `YearlySettlement`（无字段） | 40 |
| const | `NPC_ANNUAL_COST: i64 = 30_000` | 55 |
| impl 方法 | `pub fn settle(ctx, prev_year, rng)` | 61 |
| 自由函数 | `pub fn honour_type_name(t) -> &'static str` | 524 |

## 3. 方法地图（含调用关系）

### 3.1 `settle`（61）— 唯一公开入口
- **作用**：执行一次跨年结算。@param prev_year 上一年度。
- **调用链**（严格顺序 = 业务契约）：
  1. `ctx.tournaments.yearly_stats()` — 先采集年度统计（award 会清空累计器）
  2. `year_end_awards(..., prev_year, 20)` — 颁上年度 TOP20
  3. `Self::archive_year(...)` — 生涯档案逐赛季记录
  4. `Self::update_player_career_layer(...)` — 更新职业身份层（新字段）
  5. `Self::reset_season_goals(...)` — 赛季目标重置
  6. `Self::apply_yearly_growth(world, rng)` — 年龄+1 + 成长曲线 + 生理衰退
  7. `Self::apply_population_turnover(...)` — 退役 + 新秀补位
  8. `Self::apply_annual_payroll(world)` — 薪资支出
  9. `Self::apply_sponsor_rollover(...)` — 代言复位
  10. `Self::contracts_expire_one_year(world)` — 合同结转

### 3.2 `update_player_career_layer`（71）— private
- **作用**：从已结算事实更新 CareerInfo 新字段（PlayerStatus），不参与比赛/Rating/VRS/TOP20/NPC。
- **内部计算**：
  - `relation` = 选手队伍化学平均关系（player→team→roster→average_relation），无队默认 50.0
  - `leadership` = leader*0.45 + communication*0.25 + mentality*0.15 + team_spirit*0.15（clamp 0-100）
  - `performance` = maps_all>0 ? 50+(all_rating-1)*100 : 沿用旧 performance
  - `major_count` / `top20` = 年度 Major 参赛数 / TOP20 次数
  - `influence` = performance*0.28 + leadership*0.22 + relation*0.15 + reputation*0.15 + tenure 奖励 + major_bonus + top20*10
- **写入**：`player_status = status` + 按荣誉/里程碑写入 `CareerMemory`（FirstT1Contract/Top20/Mvp/FirstMajor/MajorPlayoff/MajorFinal/MajorKeyMoment/MajorChampion）。
- **调用者**：仅 `settle`（71: Self::update_player_career_layer）。

### 3.3 `evaluate_season_goal`（196）— private
- **作用**：赛季目标结算（可解释），由本赛季档案 vs 上赛季档案对比产生 (Option<bool>, String)。
- **7 种 SeasonGoal**：WinTitle / Top20 / ImproveRating / SecureStarting / SeekTransfer / RecoverForm。
- **调用者**：`archive_year`（内部）。

### 3.4 `reset_season_goals`（274）— private
- **作用**：目标年份 ≤ 上一年度 → 清空 goal + year。
- **调用者**：`settle`。

### 3.5 `archive_year`（289）— private
- **作用**：每个玩家上一年度归档为 `SeasonRecord`（含荣誉/出场/rating/财务/印记/赛季目标达成）。
- **要点**：rating 用 `RatingCalculator::rating_of_counts`（全年累计口径，非逐图平均）；归档后复位 injury_days_this_year。
- **调用者**：`settle`。

### 3.6 `apply_yearly_growth`（438）— private
- **作用**：全选手年龄+1 + 成长曲线；25 岁起生理衰退；玩家声誉年度回归 15%（FormModel::yearly_reputation_decay）。
- **调用者**：`settle`。

### 3.7 `apply_population_turnover`（455）— private
- **作用**：人口再生（委托 PopulationEngine::apply_annual_turnover），并把结果降噪写入 journal：主角保留单条 Retirement/RookieIntake，NPC 聚合为 LiveUpdate。
- **调用者**：`settle`。

### 3.8 `apply_annual_payroll`（501）— private
- **作用**：每队预算扣除「玩家薪资 + NPC 人头运营费」（先收集再扣，避免双借用）。可透支为负。
- **调用者**：`settle`。

### 3.9 `apply_sponsor_rollover`（525）— private
- **作用**：只复位代言评估年份（允许决策批次重新评估）；合同年限结转在 EconomyEngine::annual_sponsor_check 单点，**不在此重复扣**（注释：历史 bug 修复约定——同一批合同一年扣两年）。
- **调用者**：`settle`。

### 3.10 `contracts_expire_one_year`（541）— private
- **作用**：合同年限 -1（0 不再减）。
- **调用者**：`settle`。

### 3.11 `honour_type_name`（524）— pub 自由函数
- **作用**：荣誉类型 → 档案字符串（= Kotlin IndividualHonourType.name）。
- **调用者**：`archive_year`；**对外 pub**（需 grep 确认外部消费点）。

## 4. 疑点登记（待跨文件比对后判定）
- **D1**：`update_player_career_layer` 内的 `relation` 读取链（player→team→roster→average_relation→50.0）与 batch.rs/season.rs 的 cohesion 链**形似但语义不同**（这里取 average_relation 非 cohesion，且依赖 roster 名单）——此前"F3 cohesion 4 处重复"**需修正**：settlement 这处不是 cohesion 链，是 relation 链，与 batch/season 的"训练凝聚力"不同源。待读 batch/season 后精确比对。
- **D2**：leadership/performance/influence 公式仅此一份，位于 core 编排层。模块文档定位是"职业身份层更新"——属编排职责内计算，**不构成"规则层外泄"**（推翻此前 F2 判断）。但 leadership 加权是否与别处（如 csc-simulation 训练/比赛）有同口径副本，待查。
- **D3**：`honour_type_name` pub 但可能仅 archive 用——若外部无消费可收私有。

## 5. 测试覆盖（约 14 个 #[test]）
- honour_names_match_kotlin / contracts_and_sponsor_rollover / payroll_deducts_salary_and_npc_cost
- season_goal_*（win_title/top20/improve_rating/secure_starting/seek_transfer/recover_form_no_overflow/none）
- 质量：每个 season_goal 分支都有正反例断言，边界含等号（+0.03 恰好达成）——测试质量高。

## 6. 判定（对照此前探查结论）
| 此前结论 | 实测判定 |
|---|---|
| F2 公式外泄到 settlement | ❌ **推翻**：属编排职责内"职业身份层"计算，模块文档定位明确 |
| F3 cohesion 链 4 处含 settlement | ⚠️ **修正**：settlement 这处是 average_relation（不同链）；真正同类待 batch/season 精确比对 |
