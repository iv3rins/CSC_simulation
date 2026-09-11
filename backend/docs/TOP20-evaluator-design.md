# HLTV TOP20 评判标准 · 引擎落地方案（设计稿）

> 状态：渐进实施中
> 目标：把 HLTV TOP20 的「数据基准 + 赛事权重 + 个人荣誉 + 关键局表现 + 样本惩罚」
> 综合计分，**以引擎当前实际能拿到的数据**逐条还原，并给出需要补的数据缺口。
>
> 核心哲学（与 HLTV 对齐）：
> **「入围看个人数据与样本量，排名高低看大比赛淘汰赛表现与 MVP/EVP 数量。」**
>
> ## 实施进度
>
> | 步骤 | 内容 | 状态 |
> |---|---|---|
> | 1 | `YearlyRatingTracker` 键控 `String` → `PlayerId` | ✅ 已完成（2026 实施，测试全绿） |
> | 2~8 | SeriesStage / Elite / 加权 / EVP / 计分 / 评语 | ⬜ 待实施（见第 4 节细化） |

---

## 0. 现状诊断（为什么当前「没 MVP 也能 TOP1」）

当前 TOP20 落地在 `csc-tournaments/src/yearly_rating.rs::award()`：

```rust
// 现状：全年所有玩家按「场均 Rating = 累计 rating / 累计图数」降序，truncate(top)
```

三个缺陷：

| # | 缺陷 | 后果 |
|---|---|---|
| B1 | **无最低参赛图数门槛**（对比 MVP 有 `maps >= 2`） | 只打 1~几张图、单图爆发的玩家空降 TOP1 |
| B2 | **纯算术场均**，不分赛事层级 | Major 的一张图和 T2 预选的一张图同权 |
| B3 | **键控用 `String`（playerName）** 而非 `PlayerId` | 同名不同队（文档承认合法）串数据 |

本设计正是补这三个洞，并按 HLTV 口径扩展。

---

## 1. 数据现实盘点（设计的地基）

| HLTV 输入项 | 引擎现状 | 缺口 |
|---|---|---|
| HLTV Rating 2.0 | ✅ `RatingCalculator::rating_of_counts`（真实公式） | 无 |
| ADR | ⚠️ `PlayerLine.adr` 是**估值**，且 `rating_of_counts` 又忽略它、用 K/D 反推 | 不真实，但可接受 |
| KAST% | ⚠️ 同 ADR，估值 + 被忽略 | 同上 |
| Impact（首杀/残局/多杀） | ❌ 完全没有首杀/残局/多杀数据 | **大缺口** |
| 淘汰赛阶段标识 | ❌ `SeriesResult`/`MapScore` 无 `stage`/`is_playoff` 字段 | **大缺口**（Major 只写了字符串日志） |
| MVP | ✅ `award_event_honours` 已实现 | 无 |
| EVP | ❌ 枚举里无 EVP，无四强/EVP 判定 | **缺口** |
| 赛事层级 | ⚠️ `TourneyTier` 5 档：Major/SuperElite/T1/T2/Qualify | **HLTV 的 "Elite" 档缺失** |

> 结论：HLTV 公式的「Playoff_Impact」和「Honor(EVP)」两项，引擎**目前算不出来**。
> 必须**先补数据缺口**，再谈公式。下面分「数据前置」「公式」「落地顺序」三层。

---

## 2. 数据前置（先于公式，3 个必须补的结构）

### 2.1 `SeriesResult` 增加淘汰赛阶段标识（补 Playoff 数据）

```rust
// csc-simulation/src/series.rs
pub struct SeriesResult {
    // ...现有字段...
    /// 阶段：小组赛 / 淘汰赛（None = 单败淘汰整场即淘汰赛，或旧数据）
    pub stage: SeriesStage,          // 新增
}

#[derive(...)]
pub enum SeriesStage {
    Group,      // 小组赛 / 瑞士轮
    Playoff,    // 淘汰赛（含决赛）
    Unknown,    // 旧存档 / 无法判定（迁移期兜底）
}
```

- 赛制引擎产出处打标：`run_bracket` 里 Major 的 `swiss.series` → `Group`，`playoff.series` → `Playoff`；
  `SingleElimPlayoff` 整体 → `Playoff`；`DoubleElimGroups` 小组段 → `Group`，淘汰段 → `Playoff`。
- **存档迁移**：该字段不能破坏 `GameState` v1 的 `deny_unknown_fields` 契约，需要
  `GameState::CURRENT_VERSION` bump 到 v2 + `migrate` 补默认值（`Unknown`）。
  ——这正是之前架构审查里「迁移挂载点是空的」的第一次实操。

> **⚠️ 关键实施约束（2026 实测确认）**：`SeriesResult` 的**底层生产函数**
> `MatchSimulator::simulate_series` / `simulate_quick_series` 是**赛制无关**的（它们
> 不知道自己处于小组赛还是淘汰赛）。因此 `stage` 不能在这些函数内部推导，必须由
> **赛制引擎调用点**传入。具体传播链是：
>
> ```
> format/*::run(…) 的 match_runner 闭包  ──知道阶段──►  run_lod_series(…, stage)
>   └─►  conductor.rs::run_lod_series  ──透传──►  simulate_series(…, stage)
>         └─►  match_simulator.rs::simulate_series / simulate_quick_series 补 stage 入参
> ```
>
> 影响面清单（22 处 `SeriesResult { … }` 构造，其中生产构造 4 处 + 测试 18 处）：
> - 生产：`match_simulator.rs`（×2：simulate_series / simulate_quick_series）、
>   `conductor.rs`（run_lod_series 装配点）；
> - 调用侧：`engine.rs::run_bracket` 的 2 个 `match_runner` 闭包、`run_lod_series` 签名、
>   各 `format/*` 的 `runner()` 测试闭包。
> - 建议夹带「赛制引擎阶段」而非散点零打：`run_bracket` 里根据 `scheduled.format`
>   分支显式传入 `SeriesStage::Group` / `SeriesStage::Playoff`。

### 2.2 `TourneyTier` 补 `Elite` 档（补权重阶梯）

HLTV 阶梯是 Major / Super-Elite / Elite / T1，引擎缺 `Elite`（EPL/BLAST 决赛圈这一档）。

```rust
pub enum TourneyTier {
    Major,        // 1.50
    SuperElite,   // 1.30
    Elite,        // 1.15  ← 新增
    T1,           // 1.00
    T2,           // 0.00（不计入 TOP20）
    Qualify,      // 0.00
}
```

- 影响面：`is_elite()`、`tier_prize_pool`、`calender.rs` 赛事模板、存档兼容等都要连带改。
- 若短期不想动赛事体系，可**先用 SuperElite 内部再分档**（如 `Elite` weight 由 SuperElite 派生），
  但长期应独立成档。

### 2.3 `YearlyRatingTracker` 键控升级为 `PlayerId`（修 B3）

```rust
// 现在：HashMap<String, f64>          ← 名字键控，串数据
// 改为：HashMap<PlayerId, f64>        ← 稳定 ID 键控（B3 架构原则）
```

- `record` 签名里已经在遍历 `PlayerCharacter`，有 `id`，改动是收敛性的。
- 同步影响 `YearlyStat` 的采集（`stats()`）与 `award()` 返回，`settlement.rs` 的反解从
  `player_by_name` 改为 `world.player(pid)`（顺带消除 `expect("选手不存在")` 的 panic 面，见下）。

---

## 3. TOP20 综合评分公式（引擎版）

```
Top20_Score
  = ( Weighted_Rating × 0.50
    + Honor_Points   × 0.30
    + Playoff_Impact × 0.20 )
  × Sample_Penalty
```

### 3.1 赛事权重（对齐 HLTV，引擎 5 档映射）

| Tier | W_tier |
|---|---|
| Major | 1.50 |
| SuperElite | 1.30 |
| Elite | 1.15 |
| T1 | 1.00 |
| T2 / Qualify | 0.00（不参与） |

### 3.2 Weighted_Rating（加权场均，替代算术场均）

```
Weighted_Rating = Σ ( map_rating × W_tier ) / Σ ( map_count × W_tier )
                （按玩家全年 T1+ 每图累计；map_count 已按图数计）
```

- 现有 `rating` 累计的是「每图 rating 之和」，改成「每图 rating × W_tier 之和」，
  分母改成「每图 × W_tier 之和」。改动集中在 `YearlyRatingTracker`，收益是 Major 的好数据
  不再被 T2 预选赛稀释。

### 3.3 Honor_Points（MVP + EVP）

```
Honor_Points = Σ MVP_Score(tier) + Σ EVP_Score(tier)
```

| Tier | MVP | EVP |
|---|---|---|
| Major | 10.0 | 5.0 |
| SuperElite | 7.5 | 3.5 |
| Elite | 5.0 | 2.0 |
| T1 | 3.0 | 1.0 |

- **EVP 判定规则（新增）**：非 MVP 选手，若所在队进入**四强**且本人该赛事场均 Rating
  达到阈值（如 ≥1.10 或队内第一），授予 EVP。
- 需要在 `IndividualHonourType` 增加 `Evp` 变体，并在 `award_event_honours` 里
  补 EVP 颁发分支（当前只颁 MVP）。

### 3.4 Playoff_Impact（淘汰赛硬仗分，补缺口后可用）

- 仅统计 Major/SuperElite/Elite 的**淘汰赛阶段**（`SeriesStage::Playoff`）图。
- `Playoff_Rating` = 淘汰赛图加权场均（同 3.2 的加权，只取 Playoff 图）。
- 硬仗系数：`Playoff_Rating >= Overall_Rating` → 加分；`Playoff_Rating < 0.95` → 惩罚。
- 无淘汰赛样本（全程小组赛）→ 该子项按中性值处理，不虚高也不硬扣。

### 3.5 Sample_Penalty（样本惩罚，修 B1）

```
Sample_Penalty = min(1.0, t1_plus_maps / Target_Maps)
```

- `Target_Maps` 建议由引擎实际产出量校准（先按 HLTV 口径的 "70" 起步，
  再对照一个赛季实际产图数调整——因为引擎日历的赛程密度与真实职业赛季不同）。
- **基础硬门槛**：`t1_plus_maps < Min_Maps`（如 20）直接**不入围**（对应 HLTV 伪代码的 `continue`）。
  这取代并强化了 MVP 的 `maps >= 2`。

---

## 4. 落地顺序（依赖关系决定）

| 序 | 步骤 | 依赖 | 产出 |
|---|---|---|---|
| 1 | `YearlyRatingTracker` 键控 → `PlayerId` | 无 | 修 B3，全量迁移 |
| 2 | `SeriesResult` 加 `stage` + 赛制引擎打标 | 存栏 v2 迁移 | 补 Playoff 数据 |
| 3 | `TourneyTier` 加 `Elite`（含权重/日历/奖金连带） | 无（建议与 2 并行） | 补权重阶梯 |
| 4 | `YearlyRatingTracker.record` 改加权累计 | 1、3 | Weighted_Rating 发动机 |
| 5 | EVP 判定 + 荣誉枚举 `Evp` | 3、4 | Honor_Points 完整 |
| 6 | `Top20_Evaluator`（新模块）组装公式 + Sample_Penalty | 1~5 | 最终计分 |
| 7 | 替换 `settlement::year_end_awards` 的排名来源 | 6 | 端到端闭环 |
| 8 | HLTV 风评语引擎（下节） | 6 | 玩家代入感 |

---

## 5. 玩家端评语机制（代入感）

基于 `PlayerRankResult` 携带的中间量（Rating / MVP 数 / EVP 数 / Playoff Rating）条件触发：

1. **年度最佳**：Major/SuperElite MVP 数最高 + Playoff Rating 统治级 → 「年度最佳选手」。
2. **高 Rating 低排名**：`Rating 前 5 但 Score 排名显著靠后` → 「拥有 1.25 的恐怖 Rating，
   但缺乏关键赛事深入能力及奖牌，限制了他的排名。」
3. **大赛型选手**：`Playoff_Rating 高 + EVP>0` → 「凭借 Major 淘汰赛（1.31 Rating）的惊艳
   表现与 EVP 奖牌，成功挤入前十。」
4. **样本不足被罚**：`Sample_Penalty < 1.0` → 「出场样本不足，排名被打折。」

评语生成建议做成**纯函数**（输入 `PlayerRankResult` → 输出字符串），与打分同模块、
可单测，不耦合 UI。

---

## 6. 与既有架构的一致性约束（务必遵守）

- **可复现性**：打分必须只用「已经确定性记录的数据」，**不引入新随机源**。
  所有中间量（rating 累计、MVP/EVP、stage）都来自既定推进路径，同种子 → 同 TOP20。
- **存栏契约**：步骤 2、3 改结构必须 bump `CURRENT_VERSION` + `migrate` 补默认值，
  否则破坏「未知版本/未知字段拒绝」的既有测试。
- **错误处理**：TOP20 打分是纯内部结算，不接触外部输入 → 内部用 `assert!`；
  但 MVP/EVP 反解 `PlayerId` 时禁止 `expect`，应 `if let Some` 或 `ok_or`（对齐 D6）。
- **ID 全覆盖**：新模块所有键、所有返回结果全部用 `PlayerId`/`TeamId`，名字只做展示。

---

## 7. 一句话总结

> 引擎已经有了 Rating 公式和 MVP，但缺 **Elite 档、淘汰赛阶段标识、EVP、样本门槛、
> 稳定 ID 键控** 五样东西。按「先补数据 → 再加权 → 再组公式 → 最后评语」的顺序，
> 就能把「没 MVP 也 TOP1」的现行逻辑升级为 HLTV 口径的 TOP20。

---

## 附录 A：第 1 步实施记录（已完成）

**目标**：`YearlyRatingTracker` 累计键从 `String(playerName)` 升级为 `PlayerId`（修 B3）。

**改动文件**：
- `crates/csc-tournaments/src/yearly_rating.rs` — `rating/maps/kills/deaths/wins` 五张
  HashMap 键改 `PlayerId`；`record`/`stats`/`award` 签名与实现同步；模块头注释更新。
- `crates/csc-tournaments/src/settlement.rs` — `yearly_stats()` 返回
  `HashMap<PlayerId, YearlyStat>`；`year_end_awards` 从 `player_by_name` 反解改为
  `world.player_mut(pid)` 直接反解（顺带消除 `expect("选手不存在")` 的 panic 面）。
- `crates/csc-tournaments/src/engine.rs` — `yearly_stats()` 返回类型同步。
- `crates/csc-core/src/settlement.rs` — `archive_year` 参数改为 `HashMap<PlayerId, YearlyStat>`，
  `stats.get(&pid)` 替换 `stats.get(&pc.name)`。

**收益**：
1. 同名不同队（青训队共用 org 名）不再串数据——直接消灭了 TOP20 串数据一类 bug；
2. `year_end_awards` 去掉了「名字反解」中间层，谱系更可靠（对齐 B3 稳定 ID 原则）；
3. 消除一处 `expect("选手不存在")` panic 面（改为 `if let Some` 防御）。

**验证**：`cargo build` / `cargo test --workspace` / `cargo clippy --workspace --all-targets` 全绿。

**后续衔接**：第 4 步（加权累计）直接受益——键已是稳定 ID，加 `× W_tier` 权重时无需
再碰键控体系，改动局部化在 `record`/`stats`/`award` 内部。

