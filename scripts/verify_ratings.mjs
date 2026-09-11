#!/usr/bin/env node
/**
 * 验证 player_ratings.json 预训练产物：覆盖 / 档位分布 / 关键选手合理性。
 * 用法：node scripts/verify_ratings.mjs
 */
import { flattenRoster, readStandings, readJson } from "./lib/assets.mjs";

const standings = readStandings();
const ratings = readJson("assets/player_ratings.json");
const baseline = readJson("assets/roles_baseline.json");

const tier = (r) => (r >= 1.22 ? "Tier0" : r >= 1.12 ? "Tier1" : r >= 1.04 ? "Tier2" : r >= 0.96 ? "Tier3" : "Tier4");

const unique = [...new Set(flattenRoster(standings))];

const byName = new Map(ratings.players.map((p) => [p.player, p.rating]));
const missing = unique.filter((n) => !byName.has(n));
console.log(`选手总数: ${ratings.players.length} | 阵容覆盖: ${unique.length - missing.length}/${unique.length}`);
if (missing.length) console.log(`缺阵(窗口内<3场或退役): ${missing.join(", ")}`);

console.log("\n阵容选手 Rating TOP15:");
[...byName.entries()]
  .filter(([n]) => unique.includes(n))
  .sort((a, b) => b[1] - a[1])
  .slice(0, 15)
  .forEach(([n, r]) => console.log(`  ${n.padEnd(14)} ${r.toFixed(3)}  ${tier(r)}`));

console.log("\n档位分布（200 阵容）:");
const dist = {};
for (const n of unique) {
  const r = byName.get(n);
  const t = r ? tier(r) : "(无数据→槽位逻辑)";
  dist[t] = (dist[t] ?? 0) + 1;
}
console.log(" ", JSON.stringify(dist));

console.log("\n关键选手合理性抽查（对照真实认知）:");
const known = [
  ["donk", "≈1.30~1.45 世界第一档"],
  ["ZywOo", "≈1.25~1.35 顶级"],
  ["m0NESY", "≈1.20~1.30 顶级"],
  ["sh1ro", "≈1.15~1.30 一线核心"],
  ["zont1x", "≈1.00~1.10 一线"],
  ["karrigan", "≈0.85~0.95 IGL 偏低"],
  ["chopper", "≈0.90~1.00 IGL 偏低"],
];
for (const [n, expect] of known) {
  const r = byName.get(n);
  console.log(`  ${n.padEnd(10)} ${r ? r.toFixed(3) : "无数据".padEnd(5)}  ${tier(r ?? 0).padEnd(6)} 期望: ${expect}`);
}

// 年龄-实力相关（预训练数据可检验的维度）
console.log("\n年轻天才(≤19岁) vs 老将(≥30岁) 平均 Rating:");
const age = new Map(baseline.players.map((p) => [p.player, p.age]));
const young = [], old = [];
for (const n of unique) {
  const a = age.get(n);
  const r = byName.get(n);
  if (a == null || r == null) continue;
  if (a <= 19) young.push(r);
  if (a >= 30) old.push(r);
}
const avg = (xs) => (xs.length ? xs.reduce((s, x) => s + x, 0) / xs.length : NaN);
console.log(`  ≤19岁 (n=${young.length}): ${avg(young).toFixed(3)} | ≥30岁 (n=${old.length}): ${avg(old).toFixed(3)}`);

// ===== 硬断言：数据合理性门禁（2026-08-18 增加，此前为纯打印无校验）=====
// 校验失败即退出非零码，CI 才能把它当真正门禁（对应 TASK 三十二节）。
let failures = 0;
const fail = (msg) => {
  failures += 1;
  console.error(`  ✗ ${msg}`);
};

// 1) range assertion：全部 rating 落在合理区间 [0.5, 2.0]
for (const p of ratings.players) {
  if (!(p.rating >= 0.5 && p.rating <= 2.0)) {
    fail(`rating 越界 ${p.player}=${p.rating}（应 ∈ [0.5, 2.0]）`);
  }
}

// 2) fixture validation：player_ratings 对 standings 阵容的覆盖率（缺阵窗口/退役可缺席，但覆盖应 ≥ 70%）
const coverage = unique.length - missing.length;
if (coverage / unique.length < 0.7) {
  fail(`阵容覆盖过低 ${coverage}/${unique.length}（应 ≥ 70%）`);
}

// 3) 档位分布：世界顶级梯队（Tier0+Tier1）不应为空
if (!(dist["Tier0"] || 0) && !(dist["Tier1"] || 0)) {
  fail("档位分布中没有 Tier0/Tier1 顶级选手");
}

// 4) expected ordering：关键选手 rating 必须达到其梯队下限（对照真实认知）
const must = [
  ["donk", 1.22],
  ["ZywOo", 1.22],
  ["m0NESY", 1.12],
  ["sh1ro", 1.12],
];
for (const [n, min] of must) {
  const r = byName.get(n);
  if (r == null || r < min) {
    fail(`关键选手 ${n} rating=${r ?? "无数据"} 低于期望下限 ${min}`);
  }
}

// 5) 确定性合理性：年轻天才(≤19)平均 rating 不应明显低于老将(≥30)（年龄成长曲线）
const yAvg = avg(young);
const oAvg = avg(old);
if (young.length && old.length && yAvg < oAvg * 0.9) {
  fail(`年轻天才平均 rating(${yAvg.toFixed(3)}) 显著低于老将(${oAvg.toFixed(3)})`);
}

if (failures > 0) {
  console.error(`\n❌ verify_ratings 硬断言失败 ${failures} 项`);
  process.exit(1);
}
console.log("\n✅ verify_ratings 硬断言全部通过");
