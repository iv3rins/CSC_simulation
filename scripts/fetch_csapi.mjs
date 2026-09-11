#!/usr/bin/env node
/**
 * csapi.de 数据抓取器：拉取全部可访问比赛 + 逐场记分牌 → 聚合选手数据
 * ============================================================================
 *
 * 数据源：https://www.csapi.de（免费、无密钥、每日更新；HLTV 数据的中转 API，
 *         HLTV 直连被 Cloudflare 拦截，实测本 API 沙箱可直连）。
 *
 * 产出：
 *   1. scripts/data/csapi_players_agg.json —— 「预训练数据集」：每名选手的
 *      窗口聚合（比赛数 N / 场均 Rating / K/D / ADR / KAST / 胜负表现），
 *      与模拟器 Top20Evaluator 的输入特征同构（rating/maps/playoff/…）；
 *   2. assets/player_ratings.json —— 用真实数据重新生成评级先验（全覆盖）。
 *
 * 用法：
 *   node scripts/fetch_csapi.mjs                # 全量抓取 + 聚合 + 重写评级
 *   node scripts/fetch_csapi.mjs --limit 100    # 只抓前 100 场（冒烟测试）
 *   node scripts/fetch_csapi.mjs --concurrency 8
 *   node scripts/fetch_csapi.mjs --no-ratings   # 只产出聚合数据集，不改 player_ratings.json
 *
 * 依赖：Node ≥ 18（零 npm 依赖）。
 */

import { writeFileSync, readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { repoRoot } from "./lib/assets.mjs";

const ROOT = repoRoot();
const DATA_DIR = path.join(ROOT, "scripts", "data");
const AGG_OUT = path.join(DATA_DIR, "csapi_players_agg.json");
const CACHE_OUT = path.join(DATA_DIR, "csapi_stats_cache.json");
const RATINGS_OUT = path.join(ROOT, "assets", "player_ratings.json");
const STANDINGS_GLOB = path.join(ROOT, "assets", "standings_global_2026_01_05.json");

const API_BASE = "https://api.csapi.de";
const PAGE_SIZE = 100;
const args = process.argv.slice(2);
const limit = (() => {
  const i = args.indexOf("--limit");
  return i >= 0 ? parseInt(args[i + 1], 10) : Infinity;
})();
const concurrency = (() => {
  const i = args.indexOf("--concurrency");
  return i >= 0 ? parseInt(args[i + 1], 10) : 6;
})();
const noRatings = args.includes("--no-ratings");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * 令牌桶限速：实测 csapi 对突发请求返回 429（滚动窗口约 30~40 请求 / 10 秒，
 * 恢复很快）。桶容量 25、每 10 秒补满 → 持续速率 ≈ 2.5 req/s。
 * 注意：**并发 >1 的「齐射-退避」模式会触发更狠的限流**（4 并发实测
 * 吞吐坍缩到 ~0.1 req/s），全量抓取请用 `--concurrency 1`（串行）。
 */
let tokens = 25;
let lastRefill = Date.now();
async function acquire() {
  for (;;) {
    const now = Date.now();
    tokens = Math.min(25, tokens + ((now - lastRefill) / 10_000) * 25);
    lastRefill = now;
    if (tokens >= 1) {
      tokens -= 1;
      return;
    }
    await sleep(150);
  }
}

/** 简单并发池：把 tasks 按 concurrency 摊开执行，返回结果（保持顺序）。 */
async function pool(items, worker, size) {
  const out = new Array(items.length);
  let next = 0;
  async function run() {
    while (next < items.length) {
      const i = next++;
      out[i] = await worker(items[i], i);
    }
  }
  await Promise.all(Array.from({ length: Math.min(size, items.length) }, run));
  return out;
}

/** 抓取全部比赛元数据（分页；offset 越界 500 时停止）。 */
async function fetchAllMatches() {
  const matches = [];
  let offset = 0;
  while (matches.length < limit) {
    const url = `${API_BASE}/matches/?offset=${offset}`;
    let res;
    try {
      res = await fetch(url, { signal: AbortSignal.timeout(20_000) });
    } catch (e) {
      console.error(`分页失败 offset=${offset}: ${e.message}`);
      break;
    }
    if (res.status === 500 || res.status === 404) {
      console.log(`offset=${offset} → 数据边界（HTTP ${res.status}），停止分页`);
      break;
    }
    if (!res.ok) {
      console.error(`offset=${offset} → HTTP ${res.status}，停止`);
      break;
    }
    const page = await res.json();
    if (!Array.isArray(page) || page.length === 0) break;
    matches.push(...page);
    console.log(`已拉取 ${matches.length} 场比赛（offset=${offset}，最早 ${page.at(-1)?.date}）`);
    offset += page.length;
    await sleep(120);
  }
  return matches.slice(0, limit);
}

/** 单场记分牌缓存（断点续传：只缓存成功值；失败留空下次重试）。 */
const statsCache = (() => {
  const c = existsSync(CACHE_OUT) ? JSON.parse(readFileSync(CACHE_OUT, "utf8")) : {};
  let dirty = 0;
  const save = () => {
    writeFileSync(CACHE_OUT, JSON.stringify(c), "utf8");
    dirty = 0;
  };
  return {
    get(id) {
      return Object.prototype.hasOwnProperty.call(c, id) ? c[id] : undefined;
    },
    set(id, v) {
      c[id] = v;
      if (++dirty >= 50) save();
    },
    flush: save,
    size: () => Object.keys(c).length,
  };
})();

/** 抓取单场记分牌：{ id, name:"All", team1:{players:[...]}, team2:{players:[...]} }（带限速+重试）。 */
async function fetchMatchStats(matchId) {
  const cached = statsCache.get(matchId);
  if (cached !== undefined) return cached;
  for (let attempt = 1; ; attempt++) {
    await acquire();
    try {
      const res = await fetch(`${API_BASE}/matches/${matchId}/stats`, {
        signal: AbortSignal.timeout(25_000),
      });
      if (res.status === 429) {
        const wait = Math.min(1_000 * 2 ** (attempt - 1), 30_000) + Math.random() * 500;
        console.error(`  ⏳ 429 限流(${matchId}) 第 ${attempt} 次重试，等 ${Math.round(wait)}ms`);
        await sleep(wait);
        continue;
      }
      if (!res.ok) {
        if (attempt < 4) {
          await sleep(1_000 * attempt); // 5xx 短暂退避后重试
          continue;
        }
        console.error(`  ✗ ${matchId} → HTTP ${res.status}`);
        return null;
      }
      const data = await res.json();
      const unwrapped = Array.isArray(data) ? data[0] : data; // API 返回单元素数组
      statsCache.set(matchId, unwrapped);
      return unwrapped;
    } catch (e) {
      if (attempt >= 4) {
        console.error(`  ✗ ${matchId} → ${e.message}`);
        return null;
      }
      await sleep(800 * attempt); // 网络/超时退避
    }
  }
}

/**
 * 对手排名 → 强度权重（模仿 HLTV 赛季 Rating 的赛事/对手加权，避免
 * 强队刷低级别赛事导致 Rating 虚高、IGL/角色选手被误抬档位）：
 *   对手 Top10 → 1.5   Top20 → 1.2   Top40 → 1.0   Top80 → 0.7   其余 → 0.5
 */
function weightFor(oppRank) {
  if (oppRank == null) return 0.5;
  if (oppRank <= 10) return 1.5;
  if (oppRank <= 20) return 1.2;
  if (oppRank <= 40) return 1.0;
  if (oppRank <= 80) return 0.7;
  return 0.5;
}

/** 聚合：每名选手的窗口统计 + 每场胜负归属（全部按对手强度加权）。 */
function aggregate(matches, statsList) {
  const players = new Map(); // name → agg
  const byName = (name) => {
    if (!players.has(name)) players.set(name, { name, matches: 0, wsum: 0, rating_wsum: 0, k_w: 0, d_w: 0, adr_wsum: 0, kast_wsum: 0, wins_w: 0 });
    return players.get(name);
  };
  for (let i = 0; i < matches.length; i++) {
    const m = matches[i];
    let st = statsList[i];
    if (Array.isArray(st)) st = st[0]; // 兼容旧缓存中的数组形态
    if (!st || !m) continue;
    const winnerName = m.winner?.name ?? null;
    for (const side of [st.team1, st.team2]) {
      if (!side?.players) continue;
      const opp = side === st.team1 ? m.team2?.rank : m.team1?.rank;
      const w = weightFor(opp);
      const sideWon = side.name === winnerName;
      for (const p of side.players) {
        if (!p?.name || p.rating == null) continue;
        const agg = byName(p.name);
        agg.matches += 1;
        agg.wsum += w;
        agg.rating_wsum += w * p.rating;
        agg.k_w += w * (p.k ?? 0);
        agg.d_w += w * (p.d ?? 0);
        agg.adr_wsum += w * (p.adr ?? 0);
        agg.kast_wsum += w * (p.kast ?? 0);
        if (sideWon) agg.wins_w += w;
      }
    }
  }
  const list = [...players.values()]
    .filter((p) => p.matches >= 3) // 至少 3 场才纳入（防单场爆发噪声）
    .map((p) => ({
      player: p.name,
      matches: p.matches,
      rating: Math.round((p.rating_wsum / p.wsum) * 1000) / 1000,
      kd: p.d_w > 0 ? Math.round((p.k_w / p.d_w) * 1000) / 1000 : null,
      adr: Math.round((p.adr_wsum / p.wsum) * 10) / 10,
      kast: Math.round((p.kast_wsum / p.wsum) * 10) / 10,
      winrate: Math.round((p.wins_w / p.wsum) * 1000) / 1000,
    }))
    .sort((a, b) => b.rating - a.rating || b.matches - a.matches);
  return list;
}

/**
 * 用聚合数据重写 player_ratings.json（并入现有底稿保手编条目，新数据优先）。
 *
 * **大小写归一化**：Rust 侧 `RatingBaseline::rating_of` 是大小写敏感的精确匹配，
 * 而 standings 名单是混合大小写（"EliGE"、"ZywOo"）——csapi 返回小写昵称。
 * 因此按 standings roster 的大小写统一键名：roster 选手用 roster 原名，
 * 不在 roster 的选手保留底稿原名 / csapi 小写名。
 */
function writeRatings(aggList) {
  const base = existsSync(RATINGS_OUT) ? JSON.parse(readFileSync(RATINGS_OUT, "utf8")) : { players: [] };
  const standings = existsSync(STANDINGS_GLOB) ? JSON.parse(readFileSync(STANDINGS_GLOB, "utf8")) : null;
  const rosterCase = new Map(); // lowercase → roster 原名
  if (standings) {
    for (const k of Object.keys(standings.rankings ?? {})) {
      for (const n of standings.rankings[k].roster ?? []) rosterCase.set(n.toLowerCase(), n);
    }
  }
  const map = new Map(); // lowercase → { name(展示), rating }
  for (const p of base.players ?? []) map.set(p.player.toLowerCase(), { name: p.player, rating: p.rating });
  for (const p of aggList) {
    // 窗口强度加权 Rating 就是「当前实力先验」——覆盖手编/旧爬取值
    const key = p.player.toLowerCase();
    map.set(key, { name: rosterCase.get(key) ?? map.get(key)?.name ?? p.player, rating: p.rating });
  }
  const out = {
    generated_at: new Date().toISOString(),
    source: "csapi.de /players/stats window 2025-10~2026-08 (opponent-rank weighted)",
    player_count: map.size,
    players: [...map.entries()]
      .map(([, v]) => ({ player: v.name, rating: v.rating }))
      .sort((a, b) => b.rating - a.rating || a.player.localeCompare(b.player)),
  };
  writeFileSync(RATINGS_OUT, JSON.stringify(out, null, 2) + "\n", "utf8");
  return out.player_count;
}

async function main() {
  console.log(`[1/3] 拉取比赛元数据（limit=${limit === Infinity ? "∞" : limit}）…`);
  const matches = await fetchAllMatches();
  console.log(`  共 ${matches.length} 场`);

  console.log(`[2/3] 拉取逐场记分牌（并发 ${concurrency}，缓存 ${statsCache.size()} 场，限速 4 req/s）…`);
  const statsList = await pool(
    matches,
    async (m, i) => {
      if (i % 25 === 0) console.log(`  记分牌 ${i}/${matches.length}（缓存 ${statsCache.size()}）`);
      return fetchMatchStats(m.id);
    },
    concurrency,
  );
  statsCache.flush();
  const ok = statsList.filter(Boolean).length;
  console.log(`  成功 ${ok}/${matches.length}`);

  console.log("[3/3] 聚合选手数据…");
  const agg = aggregate(matches, statsList);
  writeFileSync(AGG_OUT, JSON.stringify({ generated_at: new Date().toISOString(), source: "csapi.de matches+stats", players: agg }, null, 2) + "\n", "utf8");
  console.log(`  聚合数据集 → ${AGG_OUT}（${agg.length} 名选手）`);
  console.log("  预览 TOP10：");
  agg.slice(0, 10).forEach((p) => console.log(`    ${p.player}: r=${p.rating} (${p.matches} 场, ADR ${p.adr}, KAST ${p.kast}%)`));

  if (!noRatings) {
    const n = writeRatings(agg);
    console.log(`  评级先验 → ${RATINGS_OUT}（${n} 名选手）`);
  }
  console.log("完成 ✓");
}

main().catch((e) => {
  console.error(`\n✗ 失败：${e.message}`);
  process.exit(1);
});
