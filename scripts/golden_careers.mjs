#!/usr/bin/env node
/**
 * 黄金生涯集执行器：对多个固定种子跑 20 年 release 产品指标，聚合成 JSON。
 * 用法：node scripts/golden_careers.mjs [--seeds 10] [--years 20] [--out /tmp/golden.json]
 * 前置：cargo build -p csc-core --release --example golden_career
 */

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const seeds = Number(opt("--seeds", "10"));
const years = Number(opt("--years", "20"));
const outPath = resolve(root, opt("--out", "scripts/data/golden_careers.json"));
const bin = resolve(root, "backend/target/release/examples/golden_career");
const assets = resolve(root, "assets");

if (!existsSync(bin)) {
  console.error("先构建：cargo build -p csc-core --release --example golden_career");
  process.exit(1);
}

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : 0);
const median = (xs) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

const results = [];
for (let seed = 1; seed <= seeds; seed++) {
  const start = Date.now();
  let raw;
  try {
    raw = execFileSync(bin, [assets, String(seed), String(years)], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  } catch (e) {
    console.error(`seed ${seed} 失败`, e.message);
    continue;
  }
  const data = JSON.parse(raw);
  results.push(data);
  console.log(`seed ${seed} done ${((Date.now() - start) / 1000).toFixed(1)}s`);
}

const summary = {
  generated_at: new Date().toISOString(),
  config: { seeds: results.length, years },
  performance: {
    total_s_mean: mean(results.map((r) => r.performance.total_ms / 1000)),
    month_p50_mean_ms: mean(results.map((r) => r.performance.month_p50_ms)),
    month_p95_mean_ms: mean(results.map((r) => r.performance.month_p95_ms)),
    snapshot_mb_mean: mean(results.map((r) => r.performance.snapshot_json_bytes / 1e6)),
    snapshot_mb_max: Math.max(...results.map((r) => r.performance.snapshot_json_bytes / 1e6)),
  },
  career: {
    retired_rate: results.filter((r) => r.career.retired).length / results.length,
    transfers_mean: mean(results.map((r) => r.career.transfers)),
    injury_events_mean: mean(results.map((r) => r.career.injury_events)),
    seasons_no_maps_mean: mean(results.map((r) => r.career.soft_lock_seasons_no_maps)),
    peak_rating_mean: mean(results.map((r) => r.career.peak_rating)),
    peak_rating_max: Math.max(...results.map((r) => r.career.peak_rating)),
    total_earnings_mean: mean(results.map((r) => r.career.total_earnings)),
    honours_mean: mean(results.map((r) => r.career.honours)),
  },
  decisions: {
    total_mean: mean(results.map((r) => r.decisions.total)),
    kind_means: {},
    life_consecutive_repeats_mean: mean(results.map((r) => r.decisions.life_consecutive_repeats ?? 0)),
  },
  world: {
    top3_champion_share_mean: mean(results.map((r) => r.world.top3_champion_share)),
    top3_champion_share_min: Math.min(...results.map((r) => r.world.top3_champion_share)),
    participants_mean: mean(results.map((r) => r.world.distinct_participants)),
    retirement_events_mean: mean(results.map((r) => r.world.event_kinds.Retirement ?? 0)),
    championship_events_mean: mean(results.map((r) => r.world.event_kinds.Championship ?? 0)),
  },
  seeds: results,
};

const kinds = new Set(results.flatMap((r) => Object.keys(r.decisions.by_kind)));
for (const k of kinds) summary.decisions.kind_means[k] = mean(results.map((r) => r.decisions.by_kind[k] ?? 0));

writeFileSync(outPath, JSON.stringify(summary, null, 2));
console.log(`summary → ${outPath}`);
console.log(JSON.stringify(summary.performance, null, 2));
console.log(JSON.stringify(summary.career, null, 2));
console.log(JSON.stringify(summary.decisions, null, 2));
console.log(JSON.stringify(summary.world, null, 2));
