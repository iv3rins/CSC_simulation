# 17-map-csc-tournaments-settlement.md（底层探针·完整读取）

> 读取方式：逐行完整读取（4 段，段间衔接）。读取日期：2026-09-05。

## 1. 文件职责
- 模块文档：`赛事结算层（Series Settlement，Kotlin SeriesSettlement.kt 转写）——从赛事引擎拆出的结算职责单点。赛事引擎只负责「跑」，全部「算账」收敛在这里。`
- 一句话：**赛事结算单点**（生涯回写/疲劳/日志/VRS/奖金/荣誉/TOP20/年度累计），结算公式的唯一事实源。

## 2. 公开面
| 项 | 签名 | 行号 |
|---|---|---|
| struct | `SeriesSettlement`（yearly_rating + top20_history 私有） | 48 |
| 访问器 | yearly_rating_snapshot / yearly_rating / yearly_rating_mut / restore_yearly_rating / top20_history / restore_top20_history | 55-88 |
| 方法 | `settle_series` / `settle_quick_series` / `award_event_honours` / `award_prize_money` / `record_participation` / `yearly_stats` / `year_end_awards` | 99-480 |
| 自由函数 | `runner_up_of`（私有）/ `career_player_name`（私有） | 33/615 |

## 3. 关键发现

### 3.1 质量亮点（比预想好）
- **`runner_up_of`（33）已收敛**：注释明示"此前 award_event_honours 与 award_prize_money 各写一份"，现已共用 + 守卫（最后一场必须含冠军，否则 None）。
- **MVP/EVP 判定委托 `HonourEvaluator::decide`（纯函数）**：award_event_honours 只做荣誉落地与日志。
- **封装边界明确**：yearly_rating/top20_history 私有，只经访问器读写（P1 收窄）。
- **注释记录历史修复约定**（经济可持续 3 万/人、NPC 也进 TOP20 竞争、比分胜者在前防叙事矛盾）——代码承载了大量试玩发现。

### 3.2 真实重复点（修正此前 F1 判断）
**此前 F1 说"胜者判定 15 处裸写"**——完整读取修正为：
- 该文件内胜者三态判定出现 **3 处同构块**（非 15 处）：
  1. `settle_quick_series` L134-146：`a_won/b_won` + 双方 roster 士气回写
  2. `apply_career_stats` L503-516：胜者 roster 加胜场 + 士气回写
  3. `record_match_played` L580-597：winner/loser 推导（含单图胜者）
- 1 与 2 的"士气回写双 roster 循环"**结构几乎相同**（差异：1 只在 a_won||b_won 时做，2 同时算胜场）——**同文件内可提取"apply_morale_to_rosters(world, series, a_won, b_won)"辅助**。
- `record_match_played` 的"胜者在前比分/胜者ID推导"也是同一三态语义的第 3 种表达。

### 3.3 分层确认
- 奖金公式（60/40 分池、15%/8% 玩家分成）在此文件——**符合"算账全在 settlement"铁律** ✅
- MVP 评分权重在 Top20Evaluator（独立模块）✅
- 疲劳消耗委托 ConditionModel ✅
- **未发现"公式外泄"**——与此前 F2 猜测相反，本文件是公式的正确归属地。

## 4. 方法地图简表
| fn | 行 | 调用者 | 调用了 |
|---|---|---|---|
| settle_series | 99 | conductor/engine（精确路径） | apply_career_stats/apply_fatigue/record_match_played/vrs.apply_match_result |
| settle_quick_series | 123 | conductor（粗略路径） | FormModel/record_yearly_and_career/apply_fatigue/record_match_played/vrs |
| award_event_honours | 179 | 赛事结算编排 | HonourEvaluator::decide/FormModel/yearly_rating.record_honour/journal |
| award_prize_money | 346 | 同上 | runner_up_of/EconomyEngine::award_prize_share |
| year_end_awards | 430 | 跨年结算（csc-core） | Top20Evaluator::evaluate_with_wildcards/yearly_rating.clear |
| apply_career_stats | 492 | settle_series | record_yearly_and_career/FormModel |
| record_yearly_and_career | 527 | settle_series + settle_quick_series | yearly_rating.record |
| apply_fatigue | 563 | settle_series + settle_quick_series | ConditionModel::fatigue_cost |
| record_match_played | 574 | settle_series + settle_quick_series | journal.record |
| runner_up_of | 33 | award_event_honours + award_prize_money | — |

## 5. 疑点登记
- **S1**：settle_quick_series 与 apply_career_stats 的"士气回写"循环同构，可提取辅助（低风险，纯提取）。
- **S2**：record_yearly_and_career 的 "line → roster 归属判定"（line.team_sig==team_a_sig ? a : b）与 award_event_honours 内 MVP 采集（L221）同构——跨函数重复，MVP 采集用的是 team_id 分支。

## 6. 测试
- 仅 1 个 `default_state_has_empty_rating`（内嵌）——**大量结算行为（settle_series/奖金/荣誉）无本文件单测**，依赖集成测试。这是测试缺口（结算是最易回归的层）。

## 7. 判定（对照 F1）
- ❌ 推翻"15 处裸写"——该文件是结算单点且已收敛 runner_up_of；
- ⚠️ 修正：真实问题是**同文件/跨文件的"士气回写+归属判定"同构**（S1/S2），可提取但危害小于"多事实源漂移"；
- 🔴 新发现：**结算层测试覆盖极弱**（1 个单测），结算回归靠集成——这是"一直出 BUG"的一个更可能根因。
