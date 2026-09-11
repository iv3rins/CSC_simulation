#!/usr/bin/env node
/**
 * 资产 schema 校验工具（P0-②）：为 5 类 JSON 资产建立 schema 版本字段 + 启动期强校验。
 *
 * 用法：
 *   node tools/verify_assets.mjs [assets_dir]
 *   （默认 assets_dir = "assets"，以仓库根为基准）
 *
 * 设计约定（见 docs/ASSET-SCHEMA.md）：
 *   - 5 类资产：standings_*.json / roles_baseline.json / player_ratings.json /
 *     rating_profile.json / text/zh-CN.json
 *   - schema_version 缺失 = 视为 v1（现存资产免改即通过）；新增字段一律向前兼容。
 *   - 浅校验（类型 + 区间）；语义校验由后端启动期解析器（如 RoleBaseline::from_json_str）
 *     双保险兜底。
 *   - 纯 node: 模块，零 npm 依赖；错误输出"文件 → 规则 → 值"清单并 exit(1)。
 *
 * 与 scripts/ 风格一致：硬断言 + exit(1)（参考 scripts/verify_ratings.mjs）。
 */
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ASSETS_DIR = resolve(REPO_ROOT, process.argv[2] ?? "assets");

const errors = [];
const checked = [];

/** 记录错误：文件 → 规则 → 值 */
function fail(file, rule, detail) {
  errors.push(`  ${file}\n    规则: ${rule}\n    实际: ${detail}`);
}

function isPlainObject(v) {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

function isFiniteNumber(v) {
  return typeof v === "number" && Number.isFinite(v);
}

/** 读取并解析 JSON；失败直接计入错误并返回 null。 */
function loadJson(relPath) {
  const abs = join(ASSETS_DIR, relPath);
  try {
    return JSON.parse(readFileSync(abs, "utf8"));
  } catch (e) {
    errors.push(`  ${relPath}\n    规则: JSON 可解析\n    实际: ${e.message}`);
    return null;
  }
}

/* ------------------------------------------------------------------ */
/* 1. standings_*.json                                                 */
/* ------------------------------------------------------------------ */
function verifyStandings(file) {
  const doc = loadJson(file);
  if (doc === null) return;
  const tag = `${file} [schema_version=${doc.schema_version ?? "v1(缺失)"}]`;

  if (!isPlainObject(doc)) return fail(file, "顶层必须是对象（含 schema_version 与 rankings）", typeof doc);
  const { rankings } = doc;
  if (!Array.isArray(rankings) || rankings.length === 0) {
    return fail(file, "rankings 必须是非空数组", rankings === undefined ? "缺失" : `${typeof rankings}(${Array.isArray(rankings) ? rankings.length : "?"})`);
  }
  for (const [i, e] of rankings.entries()) {
    if (!isPlainObject(e)) {
      fail(file, `rankings[${i}] 必须是对象`, typeof e);
      continue;
    }
    if (!isFiniteNumber(e.ranking) || e.ranking < 1 || e.ranking > 1000) {
      fail(file, `rankings[${i}].ranking ∈ [1,1000]`, JSON.stringify(e.ranking));
    }
    if (!isFiniteNumber(e.points) || e.points < 0 || e.points > 10000) {
      fail(file, `rankings[${i}].points ∈ [0,10000]`, JSON.stringify(e.points));
    }
    if (typeof e.teamName !== "string" || e.teamName.length === 0) {
      fail(file, `rankings[${i}].teamName 非空字符串`, JSON.stringify(e.teamName));
    }
    if (!Array.isArray(e.roster) || e.roster.length === 0 || e.roster.some((n) => typeof n !== "string" || n.length === 0)) {
      fail(file, `rankings[${i}].roster 非空字符串数组`, JSON.stringify(e.roster));
    }
  }
  checked.push(tag);
}

/* ------------------------------------------------------------------ */
/* 2. roles_baseline.json                                              */
/* ------------------------------------------------------------------ */
function verifyRolesBaseline(file) {
  const doc = loadJson(file);
  if (doc === null) return;
  const tag = `${file} [schema_version=${doc.schema_version ?? "v1(缺失)"}]`;

  if (!isPlainObject(doc) || !Array.isArray(doc.players) || doc.players.length === 0) {
    return fail(file, "顶层对象 + players 非空数组", isPlainObject(doc) ? JSON.stringify(doc.players) : typeof doc);
  }
  for (const [i, p] of doc.players.entries()) {
    if (!isPlainObject(p)) {
      fail(file, `players[${i}] 必须是对象`, typeof p);
      continue;
    }
    for (const key of ["player", "team", "role", "ctRole", "tRole"]) {
      if (typeof p[key] !== "string" || p[key].length === 0) {
        fail(file, `players[${i}].${key} 非空字符串`, JSON.stringify(p[key]));
      }
    }
    if (p.age !== undefined && (!isFiniteNumber(p.age) || p.age < 15 || p.age > 60)) {
      fail(file, `players[${i}].age 若存在 ∈ [15,60]（缺失 = 未知年龄，合法）`, JSON.stringify(p.age));
    }
  }
  checked.push(tag);
}

/* ------------------------------------------------------------------ */
/* 3. player_ratings.json                                              */
/* ------------------------------------------------------------------ */
function verifyPlayerRatings(file) {
  const doc = loadJson(file);
  if (doc === null) return;
  const tag = `${file} [schema_version=${doc.schema_version ?? "v1(缺失)"}]`;

  if (!isPlainObject(doc) || !Array.isArray(doc.players) || doc.players.length === 0) {
    return fail(file, "顶层对象 + players 非空数组", isPlainObject(doc) ? JSON.stringify(doc.players) : typeof doc);
  }
  if (!isFiniteNumber(doc.player_count) || doc.player_count !== doc.players.length) {
    fail(file, "player_count 必须等于 players.length", `${JSON.stringify(doc.player_count)} vs ${doc.players.length}`);
  }
  for (const [i, p] of doc.players.entries()) {
    if (!isPlainObject(p)) {
      fail(file, `players[${i}] 必须是对象`, typeof p);
      continue;
    }
    if (typeof p.player !== "string" || p.player.length === 0) {
      fail(file, `players[${i}].player 非空字符串`, JSON.stringify(p.player));
    }
    if (!isFiniteNumber(p.rating) || p.rating < 0 || p.rating > 5) {
      fail(file, `players[${i}].rating ∈ [0,5]`, JSON.stringify(p.rating));
    }
  }
  checked.push(tag);
}

/* ------------------------------------------------------------------ */
/* 4. rating_profile.json                                              */
/* ------------------------------------------------------------------ */
function verifyRatingProfile(file) {
  const doc = loadJson(file);
  if (doc === null) return;
  const tag = `${file} [schema_version=${doc.schema_version ?? "v1(缺失)"}]`;

  if (!isPlainObject(doc)) return fail(file, "顶层必须是对象", typeof doc);
  const { global: g, sample_size } = doc;
  if (!isPlainObject(g) || !isFiniteNumber(g.mean) || !isFiniteNumber(g.sd) || g.sd < 0) {
    fail(file, "global.mean/sd 有限数值且 sd ≥ 0", JSON.stringify(g));
  }
  if (!isPlainObject(doc.roles) || Object.keys(doc.roles).length === 0) {
    fail(file, "roles 非空对象", typeof doc.roles);
  }
  if (!Array.isArray(doc.ages) || doc.ages.length === 0) {
    fail(file, "ages 非空数组", JSON.stringify(doc.ages));
  }
  if (!isFiniteNumber(sample_size) || sample_size <= 0) {
    fail(file, "sample_size 正整数", JSON.stringify(sample_size));
  }
  checked.push(tag);
}

/* ------------------------------------------------------------------ */
/* 5. text/zh-CN.json                                                  */
/* ------------------------------------------------------------------ */
function verifyText(file) {
  const doc = loadJson(file);
  if (doc === null) return;
  const tag = `${file} [schema_version=${doc.schema_version ?? "v1(缺失)"}]`;

  if (!isPlainObject(doc)) return fail(file, "顶层必须是对象", typeof doc);
  const walk = (node, path) => {
    if (isPlainObject(node)) {
      for (const [k, v] of Object.entries(node)) walk(v, `${path}.${k}`);
    } else if (Array.isArray(node)) {
      if (node.some((v) => typeof v !== "string")) {
        fail(file, `${path} 数组元素必须是字符串`, `${node.find((v) => typeof v !== "string")} (${typeof node.find((v) => typeof v !== "string")})`);
      }
    } else if (typeof node !== "string") {
      fail(file, `${path} 值必须是字符串或字符串数组`, `${typeof node}`);
    }
  };
  walk(doc, "text");
  checked.push(tag);
}

/* ------------------------------------------------------------------ */
/* 入口                                                               */
/* ------------------------------------------------------------------ */
verifyStandings("standings_global_2026_01_05.json");
verifyRolesBaseline("roles_baseline.json");
verifyPlayerRatings("player_ratings.json");
verifyRatingProfile("rating_profile.json");
verifyText("text/zh-CN.json");

console.log(`资产 schema 校验：${ASSETS_DIR}`);
for (const t of checked) console.log(`  ✓ ${t}`);
if (errors.length > 0) {
  console.error(`\n✗ ${errors.length} 处违规：`);
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log(`\n全部通过（${checked.length} 类资产）。schema_version 缺失 = v1（向后兼容，见 docs/ASSET-SCHEMA.md）。`);
