# csc-domain —— M0 模块交付

Kotlin `me.iverins.csc.domain/`（8 文件 / 447 行）的 Rust 转写。
对照蓝本：`backend/ARCHITECTURE-MAPPING.md` §3-§4。

## 交付物

| 交付项 | 位置 | 状态 |
|---|---|---|
| 类型定义 | `src/*.rs`（8 模块 + AttrRange） | ✅ 全量 |
| 核心逻辑 | 查表函数（TierTable/tier_profiles/from_ranking/venue 推导） | ✅ |
| 单元测试 | 各模块 `#[cfg(test)]`（边界/语义/serde 往返） | ✅ 29 用例 |
| Kotlin 对照说明 | 本文件 §对照 | ✅ |

## 类型映射

| Kotlin | Rust | 说明 |
|---|---|---|
| `data class City` | `struct City` | 含 String → `Clone`（非 `Copy`） |
| `enum class Region` | `enum Region` | 6 变体 |
| `data class MatchResult(winnerSig, loserSig, tier, venue=tierVenue(tier))` | `struct MatchResult { winner_sig, loser_sig, tier, venue }` + `MatchResult::new()` 推导默认 venue | 默认参数 → `new` 构造 + pub 字段覆盖 |
| `enum class MatchVenue` | `enum MatchVenue` | 3 变体 |
| `enum class TeamTier` + companion 常量/`teamTierOf` | `enum TeamTier` + 关联常量 + `from_ranking` | — |
| `enum class Tier` / `data class TierRanges` / `object TierTable` | `enum Tier` / `struct TierRanges` / `struct TierTable` + 静态 match 表 | **Map → `&'static` match**（零堆分配） |
| `IntRange` | `AttrRange { lo, hi }` | 闭区间语义显式化，JSON 友好 |
| `data class TierProfile` + 顶层函数 | `struct TierProfile` + `tier_profiles`/`tier_prize_pool`/... | `TIER_PROFILES: Map` → 函数表 |
| `data class Tournament`（15 字段带默认值） | `struct Tournament` + `Tournament::new`（6 必填） | 默认参数 → new + pub 字段覆盖 |
| `enum class Organizer` / `InvitePolicy` | `enum` | 6 / 4 变体 |
| `sealed interface TournamentFormat`（data object/data class） | `enum TournamentFormat`（单元/带 payload 变体） | **sealed → tagged enum** |
| `enum class TourneyTier` | `enum TourneyTier` | 5 变体 |

## 关键设计决策

1. **命名**：Kotlin camelCase 字段 → Rust snake_case（`winnerSig`→`winner_sig`）；
   枚举变体 CamelCase（`MAJOR`→`Major`、`TIER0`→`Tier0`），serde `rename_all = "SCREAMING_SNAKE_CASE"`
   使 **JSON 名与 Kotlin 常量名完全一致**（`"MAJOR"`/`"TIER0"`/`"VRS_GLOBAL"`），跨语言对照零歧义。
2. **查表去 Map**：`TierTable.ranges`、`TIER_PROFILES` 从 `Map<Enum, T>` 改为
   `match → &'static T`——查表语义不变（`getValue` 等价 match 穷尽），零堆分配、可 const。
3. **默认参数 → `new()` + pub 字段**：Kotlin 的命名参数默认值在 Rust 没有直接对应；
   用 `new(必填…)` 构造 + 全 pub 字段 + `..` 结构体更新/字段覆盖，调用点语义等价。
4. **serde 契约**：全部类型 `Serialize + Deserialize`（GameState 存档地基，M7 使用）；
   枚举 externally-tagged（默认），变体名对齐 Kotlin。

## 语义等价验证（对照 Kotlin 行为）

| 用例 | Kotlin 断言 | Rust 测试 |
|---|---|---|
| `teamTierOf` 边界 | 1/12→T1、13/32→T2、33/120→T3、121/0/-5→T4 | `ranking_boundaries` + `ranking_out_of_range_below_one` |
| TierTable aim 中心 | 90/86/82/78/73 单调递减 | `tier_centers_decrease_monotonically` |
| Tier0 风格空间 | awp.hi=95、leader.lo=60 | `tier0_awp_spread_allows_role_differentiation` |
| tierVenue 默认 | MAJOR/T1→LAN、T2/QUALIFY→ONLINE | `venue_default_follows_tier` / `tier_venue_default_matches_kotlin` |
| 奖池表 | 1M/1M/500K/100K/10K | `prize_pool_table_matches_kotlin` |
| Tournament 默认值 | slots=16/invites=12/qualifier=4/LAN/TBD/3 天/单败 | `defaults_match_kotlin` |
| 赛制默认参数 | swiss(3/8/1/3/5)、double(4/3/3/5) | `swiss_playoff_defaults_match_kotlin` 等 |
| serde 往返 | （原型无序列化契约） | 全类型 `serde_roundtrip` |

## 编译与测试

```bash
cd backend
cargo test -p csc-domain        # 29 用例全绿
```
