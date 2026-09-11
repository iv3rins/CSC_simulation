#!/usr/bin/env node
/**
 * 选手模型校准：从真实数据拟合「新选手实力分布」→ assets/rating_profile.json
 * ============================================================================
 *
 * 数据源：
 *   - scripts/data/csapi_players_agg.json（672 名真实选手窗口聚合 Rating）
 *   - assets/roles_baseline.json（真实选手年龄 + HLTV 角色）
 *   - assets/standings_global_2026_01_05.json（世界名单，用于过滤建模样本）
 *
 * 产出（RatingProfile，Rust `csc_entities::baseline::RatingProfile` 消费）：
 *   - global ：全职业 Rating 分布（新选手采样的总体）
 *   - roles  ：按角色分布（AWP/ENTRY/SUPPORT/LURKER/IGL 有真实样本；RIFLER
 *              为 HLTV「其他」兜底角色，无直接样本 → 用 global）
 *   - ages   ：按年龄段分布（小样本，仅供未来扩展/检验，不直接参与采样）
 *   - rookie ：新秀采样参数（均值偏移 + 标准差缩放）——青训新秀按全职业
 *              分布起步、略打折扣（潜力未兑现），天才尾由潜力系统负责
 *
 * 用法：node scripts/calibrate_profile.mjs
 */
import { writeFileSync } from "node:fs";
import { flattenRoster, repoRoot, readJson, readStandings } from "./lib/assets.mjs";
import path from "node:path";

const ROOT = repoRoot();
const agg = readJson("scripts/data/csapi_players_agg.json").players;
const baseline = readJson("assets/roles_baseline.json").players;
const standings = readStandings();

const roster = new Set(flattenRoster(standings));
const bmap = new Map(baseline.map((p) => [p.player, p]));

const avg = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : NaN);
const sd = (xs) => (xs.length > 1 ? Math.sqrt(xs.reduce((a, b) => a + (b - avg(xs)) ** 2, 0) / xs.length) : 0);
const r3 = (x) => Math.round(x * 1000) / 1000;

// 全职业总体：所有窗口 ≥3 场的真实选手（含弱队 → 真实职业分布）
const global = { mean: r3(avg(agg.map((p) => p.rating))), sd: r3(sd(agg.map((p) => p.rating))) };

// 按角色（仅世界名单内、有 baseline 角色的选手；HLTV 角色 → 游戏 Role 名）
const hltv2role = { AWPer: "AWP", Opener: "ENTRY", Support: "SUPPORT", Closer: "LURKER" };
const roles = {};
for (const p of agg) {
  const b = bmap.get(p.player);
  if (!b || !roster.has(p.player)) continue;
  const role = hltv2role[b.role.split("-")[0]] ?? (b.role === "IGL-Opener" || b.role === "IGL-AWPer" || b.role === "IGL-Closer" || b.role === "IGL" ? "IGL" : null);
  if (!role) continue;
  (roles[role] = roles[role] || []).push(p.rating);
}
// IGL 兜底：HLTV 角色里 IGL-* 前缀（上面 split('-')[0] 已覆盖，但显式再兜一次）
const roleOut = {};
for (const [k, v] of Object.entries(roles)) roleOut[k] = { mean: r3(avg(v)), sd: r3(sd(v)) };
roleOut.RIFLER = global; // HLTV「其他」→ 游戏默认角色，无直接样本 → 全职业

// 按年龄段（仅世界名单内、有年龄的选手；小样本仅供检验）
const bands = [
  { min: 17, max: 19 }, { min: 20, max: 23 }, { min: 24, max: 27 }, { min: 28, max: 30 }, { min: 31, max: 60 },
];
const ages = bands.map((b) => {
  const xs = agg.filter((p) => {
    const a = bmap.get(p.player)?.age;
    return roster.has(p.player) && a != null && a >= b.min && a <= b.max;
  }).map((p) => p.rating);
  return { min: b.min, max: b.max, n: xs.length, mean: r3(avg(xs)), sd: r3(sd(xs)) };
});

const out = {
  generated_at: new Date().toISOString(),
  source: "csapi.de 窗口 2025-10~2026-08 全职业聚合 + roles_baseline 年龄/角色",
  sample_size: agg.length,
  global,
  roles: roleOut,
  ages,
  // 新秀采样：全职业分布 + 均值折扣（青训起步略低于职业均值）+ 标准差微调
  rookie: { mean_offset: -0.02, sd_scale: 1.0, clamp: [0.5, 1.7] },
};

writeFileSync(path.join(ROOT, "assets", "rating_profile.json"), JSON.stringify(out, null, 2) + "\n", "utf8");
console.log(`已写入 assets/rating_profile.json（样本 ${agg.length} 名）`);
console.log(`  global: ${global.mean} ± ${global.sd}`);
console.log("  roles:", Object.entries(roleOut).map(([k, v]) => `${k}=${v.mean}±${v.sd}`).join(" "));
console.log("  ages:", ages.map((a) => `${a.min}-${a.max}:${a.mean}±${a.sd}(n=${a.n})`).join(" "));
