# 03 · 实体层：csc-entities

> 第 1 层：实体 arena（ID 即 Vec 索引）+ 属性领域 + 角色/队伍/伤病/印记 + 确定性生成器。
> 设计核心：**arena 不收缩**，退役 = 置标记（ID 永远稳定）；双端归属不变量由原子操作维护。

---

## 一、Struct：`World`（csc-entities/src/world.rs）

世界仓库（arena）：全部实体的持有与原子操作。

### 字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `players` | `Vec<PlayerCharacter>` | 选手 arena（Player 与 NPC 统一存储） |
| `teams` | `Vec<Team>` | 队伍 arena |
| `free_agents` | `Vec<PlayerId>` | 自由市场 |
| `next_player_id` / `next_team_id` | `u32` | = len()，序列化保一致 |

### 创建方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `new` | `pub fn new() -> Self` | 空世界 |
| `create_player` | `pub fn create_player(&mut self, tier: Tier, name: Option<&str>, rng: &mut Xoshiro256StarStar, age: Option<i32>, role: Option<Role>) -> PlayerId` | 创建主角（自由身） |
| `create_npc` | `pub fn create_npc(&mut self, tier, team: Option<TeamId>, role, name, rng, age) -> Result<PlayerId, WorldError>` | 创建 NPC（先校验队伍再登记，不跳号） |
| `push_character` | `pub fn push_character(&mut self, pc: PlayerCharacter, team: Option<TeamId>) -> Result<PlayerId, WorldError>` | 注册已生成角色（保留精确属性） |
| `create_team` | `pub fn create_team(&mut self, name: impl Into<String>, vrs_ranking: i32, vrs_value: i32) -> TeamId` | 创建空队伍 |

### 查询方法

| 方法 | 签名 | 说明 |
|---|---|---|
| `player` / `player_mut` | `(&self, id) -> Option<&PlayerCharacter>` | ID 查选手（越界 None） |
| `team` / `team_mut` | `(&self, id) -> Option<&Team>` | ID 查队伍 |
| `player_by_name` | `(&self, name: &str) -> Option<PlayerId>` | 昵称查（线性；退役者不可查） |
| `team_by_name` | `(&self, name: &str) -> Option<TeamId>` | 队名查 |
| `all_players` / `all_players_only` / `all_npcs` | `-> &[PlayerCharacter]` / `Vec<&…>` | 全部 / 仅主角 / 仅 NPC（退役者过滤） |
| `all_teams` | `-> &[Team]` | 全部队伍 |
| `all_free_agents` | `-> Vec<PlayerId>` | 自由市场 |
| `roster` | `(&self, team_id) -> Vec<&PlayerCharacter>` | 队伍阵容 |
| `signature_of` | `(&self, team_id) -> String` | 阵容签名（队名\|选手排序名） |

### 归属原子操作（维护 roster_ids ⟷ player.team 双端不变量）

| 方法 | 签名 | 说明 |
|---|---|---|
| `assign_player_to_team` | `(&mut self, player, team_id) -> Result<(), WorldError>` | 签入（幂等） |
| `release_player` | `(&mut self, player) -> Result<(), WorldError>` | 移出 |
| `add_free_agent` | `(&mut self, player) -> Result<(), WorldError>` | 放入自由市场 |
| `remove_free_agent` | `(&mut self, player)` | 移出市场（幂等） |
| `remove_npc` | `(&mut self, player) -> Result<(), WorldError>` | 退役标记 + 解除归属（**arena 不收缩**） |

### Enum：`WorldError`

`PlayerNotFound(PlayerId)` / `TeamNotFound(TeamId)`——原子操作不 panic，显式上抛。

---

## 二、Struct：`PlayerCharacter`（csc-entities/src/character.rs）

选手实体（主角与 NPC 统一）。关键字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `PlayerId` | 稳定 ID |
| `name` | `String` | 昵称（展示） |
| `age` | `i32` | 年龄 |
| `role` | `Role` | 定位（IGL/AWP/RIFLER/ENTRY/SUPPORT/LURKER） |
| `base` | `BaseAttributes` | 基础属性：`reaction/stability/endurance/stamina/health` |
| `skill` | `SkillAttributes` | 技术属性：`aim/leader/communication/clutch` |
| `pro` | `ProAttributes` | 职业属性：`mentality/confidence/team_spirit/loyalty/morale` |
| `weapon` | `WeaponAttributes` | 武器属性：`position/ak/awp/pistol/smoke/utility` |
| `potential` | `i32` | 潜力（成长上限） |
| `fatigue` | `f64` | 疲劳（0=满状态） |
| `injury` | `Option<Injury>` | 伤病 |
| `career` | `Option<CareerInfo>` | 生涯信息（主角有；NPC None） |
| `team` | `Option<TeamId>` | 归属队伍（None=自由身） |
| `retired` | `bool` | 退役标记 |

关键方法：`is_player()` / `is_npc()`（career 有无判定）、`career_mut()`。

---

## 三、Struct：`Team`（csc-entities/src/team.rs）

**实际 8 字段**（team.rs:18-36）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | `TeamId` | 稳定 ID |
| `name` | `String` | 队名 |
| `roster_ids` | `Vec<PlayerId>` | 阵容 |
| `vrs_ranking` / `vrs_value` | `i32` | VRS 缓存（season.rs 每月回写） |
| `budget` | `i64` | 预算（经济系统） |
| `chemistry` | `TeamChemistry` | 队内关系状态 |
| `current_tournament_id` | `Option<TournamentId>` | 当前参加的赛事（队伍级互斥锁） |

> ⚠️ **无 `cash`、无 `region` 字段**（pro-1 修正：文档旧版误列）。

方法：`new(id, name, vrs_ranking, vrs_value)`、`signature(&self, names: &[String]) -> String`
（Kotlin 格式）——**仅这两个方法，无 `roster_mut()`**。

---

## 四、Struct：`RandomPlayerGenerator`（csc-entities/src/generator.rs）

确定性选手生成器：

| 方法 | 签名 | 说明 |
|---|---|---|
| `generate_player` | `pub fn generate_player(tier, name, rng, age, role) -> PlayerCharacter` | 主角生成 |
| `generate_npc` | `pub fn generate_npc(tier, team, role, name, rng, age) -> PlayerCharacter` | NPC 生成 |

- 属性按 `TierProfile` 分布采样（Box-Muller 收敛到 `csc_util::gaussian` 单实现）；
- 新秀实力分布：`baseline::RatingProfile`（calibration 资产，可配置）。

---

## 五、实力计算：power.rs（csc-entities/src/power.rs）

综合实力计算（角色互补/武器加权）：

| 方法 | 签名 | 说明 |
|---|---|---|
| `player_power` | `pub fn player_power(pc: &PlayerCharacter) -> f64` | 单选手综合实力（power.rs:21） |
| `player_power_components` | `pub fn player_power_components(role: Role, base: &BaseAttributes, skill: &SkillAttributes, pro: &ProAttributes, weapon: &WeaponAttributes) -> f64` | 分组件计算（power.rs:27） |
| `role_composition` | `pub fn role_composition(roster: &[&PlayerCharacter]) -> [i32; 6]` | 角色构成统计（各 Role 计数，power.rs:80） |

> ⚠️ 旧文档的 `power_of / team_power / role_complement` **均不存在**。
> `power_of` 真身在 `csc-simulation/transfer_rules.rs:78`（= player_power 便捷入口）；
> `team_power` 只是 `csc-tournaments/conductor.rs:821` 的局部变量名。

---

## 六、其它关键类型

### 印记：`CareerMark` / `MarkType`（csc-entities/src/mark.rs）

选手生涯印记（如 Major 冠军、年度 TOP1），`CareerMark { kind, year, detail }`，
供档案/结局评定引用。

### 伤病：`Injury` / `InjuryKind` / `InjurySeverity`（csc-entities/src/injury.rs）

`Injury { kind: InjuryKind, severity: InjurySeverity, weeks_left: i32 }`；
`InjuryKind`: WRIST/FINGER/SHOULDER/BACK/LEG/EYE_STRAIN/ILLNESS；
`InjurySeverity`: MINOR/MODERATE/SEVERE。

### 角色画像：`RoleProfile`（csc-entities/src/role_profile.rs）

各 Role 的属性权重画像（供生成与成长）。

### 化学：`TeamChemistry`（csc-entities/src/chemistry.rs）

`cohesion / morale / relations` 等队内关系状态。

### 基线：`baseline.rs`

`RoleBaseline`（真实选手角色+年龄先验）、`RatingProfile`（新秀实力分布）、
`VrsMapper`（VRS 资产 → 实体装配）。
