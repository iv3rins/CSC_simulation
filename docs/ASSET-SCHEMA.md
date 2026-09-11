# ASSET-SCHEMA — 资产 schema 规格与版本表

> 用途：为 5 类 JSON 资产建立 **schema 版本字段 + 启动期强校验**（P0-②），对齐 CDDA `chkjson` 级别的资产门禁。
> 校验工具：`tools/verify_assets.mjs`（Node 零依赖）；语义校验由后端启动期解析器双保险兜底。
> 更新：2026-08（P2 收官轮）

## 1. 版本策略

| 规则 | 说明 |
|---|---|
| `schema_version` 字段 | 每类资产**顶层对象**可带 `"schema_version": N`（整数，从 1 起） |
| 缺失 = v1 | 现存资产一律免改即通过（向后兼容）；校验工具在输出中标注 `v1(缺失)` |
| 新增字段 | 一律向前兼容（后端解析侧 `serde(default)` 或脚本可选字段） |
| 变更流程 | 修改资产结构时必须：① bump `schema_version`；② 更新本文件版本表；③ 更新校验规则；④ 跑 `node tools/verify_assets.mjs assets/` |

## 2. 五类资产规格（v1 最小集）

### 2.1 standings_*.json（如 standings_global_2026_01_05.json）

```jsonc
{
  "schema_version": 1,          // 可选，缺失 = v1
  "rankings": [                 // 非空数组，≤1000 条
    {
      "ranking": 1,             // 整数 ∈ [1,1000]
      "points": 1948,           // 数值 ∈ [0,10000]
      "teamName": "FaZe Clan",  // 非空字符串
      "roster": ["karrigan"]    // 非空字符串数组
    }
  ]
}
```

### 2.2 roles_baseline.json

```jsonc
{
  "schema_version": 1,          // 可选
  "players": [                  // 非空数组
    {
      "player": "karrigan",     // 非空字符串
      "team": "FaZe Clan",      // 非空字符串
      "role": "IGL-Opener",     // 非空字符串
      "ctRole": "IGL-Opener",   // 非空字符串
      "tRole": "IGL-Opener",    // 非空字符串
      "age": 35                 // 可选；若存在 ∈ [15,60]（缺失 = 未知年龄，合法）
    }
  ]
}
```

### 2.3 player_ratings.json

```jsonc
{
  "schema_version": 1,          // 可选
  "generated_at": "...",        // 元信息（非强校验）
  "source": "...",
  "player_count": 686,          // 必须等于 players.length
  "players": [
    { "player": "donk", "rating": 1.487 }   // rating ∈ [0,5]
  ]
}
```

### 2.4 rating_profile.json

```jsonc
{
  "schema_version": 1,          // 可选
  "generated_at": "...",
  "source": "...",
  "sample_size": 100,           // 正整数
  "global": { "mean": 1.021, "sd": 0.103 },  // mean/sd 有限数值，sd ≥ 0
  "roles": { "ENTRY": {...} },  // 非空对象
  "ages": [ { "min": 17, "max": 19, "n": 6, "mean": 1.083, "sd": 0.195 } ],  // 非空数组
  "rookie": {...}
}
```

### 2.5 text/zh-CN.json

```jsonc
{
  "schema_version": 1,          // 可选
  "lang": "zh-CN",
  "text": {                     // 嵌套任意深度
    "narrator.tier.MAJOR": "Major（最高级别）",   // 值必须是字符串
    "lists.live.transfer.body": ["...", "..."]    // 或字符串数组
  }
}
```

## 3. 校验规则（浅校验边界声明）

- 工具只做**类型 + 区间**的浅校验（结构完整、类型正确、数值在界）。
- **语义校验**（如 rating 与真实选手对应、roles 与阵容对齐）仍由后端启动期解析器负责
  （`RoleBaseline::from_json_str`、`VrsDatabase::parse_json_checked` 等）——**后端解析器是唯一事实源**，工具不复制语义逻辑，避免双实现漂移。
- 错误输出格式：`文件 → 规则 → 实际值` 清单，任一类失败即 `exit(1)`（CI 门禁）。

## 4. 版本表

| 资产 | v1 | 变更记录 |
|---|---|---|
| standings_*.json | 2026-08（P2 收官轮定稿） | — |
| roles_baseline.json | 2026-08 | — |
| player_ratings.json | 2026-08 | — |
| rating_profile.json | 2026-08 | — |
| text/zh-CN.json | 2026-08 | — |

## 5. 接入点

| 接入 | 位置 | 说明 |
|---|---|---|
| 校验工具 | `tools/verify_assets.mjs` | 独立运行：`node tools/verify_assets.mjs assets/` |
| 后端启动期 | `csc-server/src/state.rs` `AssetsBundle::load_from_dir` | Rust 侧结构检查（类型 + 必填字段），复用既有解析器能力 |
| ~~前端资产同步~~（`frontend/scripts/sync_assets.mjs`） | 🗑 前端已移除（2026-09）：队标等展示图不再同步进 public |
| CI | `.github/workflows/ci.yml` | assets-verify 步骤：`node tools/verify_assets.mjs assets/` |

## 6. 负向验证样例

```bash
# 改坏 standings 的 points 类型 → 脚本报错 exit 1
node tools/verify_assets.mjs assets/   # 期望：✗ 1 处违规 + exit(1)
# 还原后 → 全绿 exit 0
```
