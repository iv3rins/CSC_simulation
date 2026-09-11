#!/usr/bin/env node
/**
 * HLTV 2026-08 选手/战队实力建模
 * ================================
 * 输入：
 *   - scripts/data/hltv_ranking_2026_08_10.md   （HLTV 世界排名页，2026-08-10）
 *   - scripts/data/hltv_top20_2025.md           （HLTV TOP20 2025 页）
 *   - assets/player_ratings.json                （csapi.de 窗口 Rating，686 名选手）
 *   - assets/standings_global_2026_01_05.json   （模拟世界初始名单，用于口径对照）
 * 输出：
 *   - assets/hltv/strength_model_2026_08_10.json
 *   - docs/HLTV-2026-STRENGTH-MODEL.md
 *
 * 模型核心（与引擎同构的 Rating → Power 线性映射）：
 *   player_power = clamp(25 + 50 * rating, 0, 100)
 *   team_power   = avg(player_power of TOP3 core) × 阵容完整度 × 角色多样性
 *
 * 该映射锚点：
 *   rating 1.20 → power 85（T1 转会门槛）
 *   rating 1.00 → power 75（T2 转会门槛）
 *   rating 0.70 → power 60（T3 转会门槛）
 */
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const rankingMd = readFileSync(path.join(ROOT, "scripts/data/hltv_ranking_2026_08_10.md"), "utf8");
const top20Md = readFileSync(path.join(ROOT, "scripts/data/hltv_top20_2025.md"), "utf8");
const ratings = new Map(
  JSON.parse(readFileSync(path.join(ROOT, "assets/player_ratings.json"), "utf8")).players
    .map((p) => [p.player.toLowerCase(), p.rating]),
);
const standings = JSON.parse(
  readFileSync(path.join(ROOT, "assets/standings_global_2026_01_05.json"), "utf8"),
).rankings;
const ratingProfile = JSON.parse(
  readFileSync(path.join(ROOT, "assets/rating_profile.json"), "utf8"),
);

const clamp = (v, lo = 0, hi = 100) => Math.max(lo, Math.min(hi, v));
const ratingToPower = (r) => clamp(25 + 50 * r);
const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

// —— 1. 解析 HLTV 世界排名页（213 队）——
const teamPattern = /#(\d+)\s*.*?\n\n([A-Za-z0-9 .&'-]+)\((\d+) HLTV points\)([\s\S]*?)(?=\n#\d|\s*$)/g;
const teams = [];
for (const m of rankingMd.matchAll(teamPattern)) {
  const body = m[4];
  const roster = [];
  for (const p of body.matchAll(/\]\(https:\/\/www\.hltv\.org\/player\/\d+\/([a-z0-9-]+)\)/g)) {
    if (!roster.includes(p[1])) roster.push(p[1]);
  }
  const rvals = roster.map((p) => ratings.get(p.toLowerCase())).filter((r) => r !== undefined);
  const sorted = [...rvals].sort((a, b) => b - a);
  const core = sorted.slice(0, 3);
  const depth = sorted.slice(3);
  teams.push({
    ranking: Number(m[1]),
    team: m[2].trim(),
    hltv_points: Number(m[3]),
    roster,
    known_ratings: rvals.length,
    avg_rating: round3(mean(rvals)),
    core_rating_top3: round3(mean(core)),
    depth_rating_4_5: round3(mean(depth)),
    min_rating: round3(sorted.at(-1)),
    player_power_avg: round1(mean(rvals.map(ratingToPower))),
    player_power_core: round1(mean(core.map(ratingToPower))),
  });
}

// —— 2. 解析 TOP20 2025（年度榜口径）——
const top20_2025 = [];
for (const line of top20Md.split("\n")) {
  const rank = line.match(/\*\*#(\d+)\*\*/);
  const nick = line.match(/\[\*\*([^*]+)\*\*/);
  if (!rank || !nick) continue;
  top20_2025.push({ rank: Number(rank[1]), player: nick[1], rating: ratings.get(nick[1].toLowerCase()) ?? null });
}
top20_2025.sort((a, b) => a.rank - b.rank);

// —— 3. 当前窗口 Rating 领跑者（2026 年度赛程进行中的实时先验）——
const ratingLeaders = JSON.parse(
  readFileSync(path.join(ROOT, "assets/player_ratings.json"), "utf8"),
).players
  .sort((a, b) => b.rating - a.rating)
  .slice(0, 20)
  .map((p, i) => ({ rank: i + 1, player: p.player, rating: p.rating, power: ratingToPower(p.rating) }));

// —— 4. 与模拟世界初始名单的口径对照 ——
const initialNames = new Set(standings.flatMap((t) => t.roster.map((n) => n.toLowerCase())));
const ratingTopTeams = teams
  .map((t) => ({
    ...t,
    initial_world_coverage: t.roster.filter((p) => initialNames.has(p.toLowerCase())).length,
  }))
  .filter((t) => t.core_rating_top3 !== null);

// —— 5. 一线队补强管线检验 ——
// 一线队之所以是一线队：核心三人 rating 显著高于全职业均值，并且第 4/5 人深度不塌方。
const globalMean = ratingProfile.global.mean;
const elitePipeline = ratingTopTeams
  .slice(0, 12)
  .map((t) => ({
    ranking: t.ranking,
    team: t.team,
    core_rating_top3: t.core_rating_top3,
    depth_rating_4_5: t.depth_rating_4_5,
    core_vs_global: round3(t.core_rating_top3 - globalMean),
    depth_gap: round3(t.core_rating_top3 - t.depth_rating_4_5),
  }));

const out = {
  generated_at: new Date().toISOString(),
  sources: {
    team_ranking: "https://www.hltv.org/ranking/teams/2026/august/10",
    top20: "https://www.hltv.org/players/top20/2025",
    rating_window: "assets/player_ratings.json（csapi.de 窗口 2025-10 ~ 2026-08，对手排名加权）",
  },
  model: {
    rating_to_power: "clamp(25 + 50 * rating, 0, 100)",
    transfer_thresholds: {
      T1: { power: 85, rating: 1.2 },
      T2: { power: 75, rating: 1.0 },
      T3: { power: 60, rating: 0.7 },
    },
    team_strength: "TOP3 核心实力均值为主（0.7），全队均值（0.2）与第 4/5 人深度（0.1）为稳定性项；比赛结果仍由胜率模型决定",
    continuity: "一线队补强路径 = 每窗最多 1 笔 NPC 交易 + 阵容连续性逐窗结算；强队淘汰最弱 NPC 而不是拆核心",
  },
  sample: {
    teams_parsed: teams.length,
    teams_with_ratings: ratingTopTeams.length,
    ratings_available: ratings.size,
  },
  top20_2025,
  rating_leaders_2026_08: ratingLeaders,
  elite_pipeline_top12: elitePipeline,
  teams,
};
mkdirSync(path.join(ROOT, "assets/hltv"), { recursive: true });
writeFileSync(
  path.join(ROOT, "assets/hltv/strength_model_2026_08_10.json"),
  JSON.stringify(out, null, 2) + "\n",
  "utf8",
);
console.log(`已写入 assets/hltv/strength_model_2026_08_10.json`);
console.log(`  解析队伍 ${teams.length}，有 Rating 先验 ${ratingTopTeams.length}`);
console.log(`  TOP20 2025 ${top20_2025.length} 人，Rating 领跑 ${ratingLeaders.length} 人`);
console.log("  Top8 core rating:", elitePipeline.slice(0, 8).map((t) => `${t.team}=${t.core_rating_top3}`).join(" "));

function round1(v) { return v === null || v === undefined || Number.isNaN(v) ? null : Math.round(v * 10) / 10; }
function round3(v) { return v === null || v === undefined || Number.isNaN(v) ? null : Math.round(v * 1000) / 1000; }
