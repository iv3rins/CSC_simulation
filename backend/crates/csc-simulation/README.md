# csc-simulation —— M3 模块交付

Kotlin `simulation/`（13 文件）+ `hltv/`（RatingCalculator）的 Rust 转写。
对照蓝本：`ARCHITECTURE-MAPPING.md` §3-§7（M3）。

## 交付物

| 交付项 | 位置 | 状态 |
|---|---|---|
| 比赛协议：PlayerLine/MapScore/SeriesResult（**ID + 签名快照**） | `src/series.rs` | ✅ |
| 胜率计算器（单场发挥采样 + 赛事等级放大 + 指令/心态） | `src/win_rate.rs` | ✅ |
| 战斗模拟器（MR12/加时/击杀守恒事件链） | `src/match_simulator.rs` | ✅ |
| 波动/体况/成长/训练/士气/化学/经济/印记规则 | `src/{volatility,condition,growth,training,form,chemistry_model,finance,mark_effects}.rs` | ✅ |
| 场内指令/打法/失误类型 | `src/directives.rs` | ✅ |
| HLTV Rating 2.0（hltv 包并入） | `src/rating.rs` | ✅ |
| 转会门槛（eligibleTeams 待 M4） | `src/transfer_rules.rs` | ✅ |
| 共享工具：gaussian/pickIndex/bernoulli/sigmoid | `csc-util/src/{sampling,math_utils}.rs` | ✅ |
| **跨语言 golden 对照**（winRate/simulateMap/rating/overtime 位级） | 测试 + `tools/gen_sim_golden.kt` | ✅ |
| Kotlin 对照说明 | 本文件 | ✅ |

## 关键转写决策

### 1. 比赛协议 ID 化
`SeriesResult.teamA/teamB: Team`（实体引用）→ **`team_a_id/team_b_id` + 签名快照**；
参赛队伍用 `SeriesTeam<'a> { id, signature, roster: Vec<&PlayerCharacter> }`（World 组装）。
纯值、可序列化、跨引擎传递零别名。

### 2. 随机消耗顺序逐位对齐（跨语言可复现）
| 路径 | Kotlin 消耗 | Rust |
|---|---|---|
| winRate | self 每选手 1 gaussian（2 nextDouble）→ opp 每选手 1 gaussian | 同序 |
| simulateMap | winRate → 1 nextDouble 加时判定 → 1 nextDouble 胜负（或加时逐回合）→ A/B weights（各 5 gaussian）→ 每击杀 2 nextDouble → 每助攻 1 | 同序 |
| rollKind/rollBlunder | `nextInt(10)`/`nextInt(4)`（单参数原语） | `next_i32_bound` |

### 3. 语义核对中修正的转写偏差
- **签名格式**：Kotlin `Signatures.teamSignature` = `队名|名1,名2`（**无空格**）——初版 Rust 用了 `" | "`，golden 抓到后修正（entities 的 `Team::signature` + 测试）；
- **GrowthModel 会拉低超潜力属性**：Kotlin `minOf(potential, v + round(rate·(potential−v)))` 对超潜力 v 收敛到潜力（与 TrainingModel 的"只增不减"不同——两处语义差异是 Kotlin 原型事实，转写保持 100% 一致并注释说明）；
- **加时败方得分**：`12 + 当轮 ot 得分`（16-14 合法），初版测试误断言败方恒 12；
- **`overtime_chance` 用 `powi(12)`**（整数幂，消除 libm pow 差异）；`statValue` 的平方用乘法（M2 同策略）。

### 4. 可复现性口径验证（重要结果）
`golden_win_rate_80_vs_70` **位级通过**：Windows JVM 的 `Math.ln/sqrt/cos` 与 Rust `f64::ln/sqrt/cos` 在测试序列上逐位一致——统计口径在实测中升级为位级。若未来跨平台（Linux/glibc vs macOS）出现 1-2 ulp 差异，golden 测试会红灯，届时按 D2 降级为统计断言（测试内已注释说明）。

## 语义等价验证（golden 对照 Kotlin）

| 用例 | Kotlin 权威 | Rust 测试 |
|---|---|---|
| winRate(A80 vs B70, T1, seed42) ×5 | 位模式 `3fea171b...` | `golden_win_rate_80_vs_70` |
| simulateMap(seed42) | 13:7、胜者、A0: 26/16/5 | `golden_simulate_map` |
| overtimeChance(0.5) | 位模式 ≈0.1611 | `golden_overtime_chance` |
| ratingOf(20,15,5,30)/adr/kast | 位模式 | `golden_rating_formulas` |
| 击杀守恒（10 人 K/D/A） | 赢队击杀=输队死亡 | `kills_conservation` |
| MR12/加时/bo 系列 | 比分形状 | `bo1_series_result`/`overtime_scores_above_13` |

## 编译与测试

```bash
cd backend
cargo test          # 187 用例全绿（domain 28 + entities 52 + simulation 66 + time 19 + util 22）
cargo clippy --workspace   # 0 警告
```

## 遗留（后续模块消化）

- `TransferRules::eligible_teams` 依赖 `csc-vrs::VrsEntry` → **M4 后补**
- `Team.roster` 的 arena 组装（SeriesTeam 构造）→ **M8 core**（World 方法已就绪）
