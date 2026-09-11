#!/usr/bin/env node
/**
 * 爬虫框架：采集 CS2 真实选手 Rating → assets/player_ratings.json（v2 数据集）
 * ============================================================================
 *
 * 目标：给后端 RatingBaseline（csc-entities/baseline.rs）提供真实评级先验，
 *       修复「真实选手名只是皮肤、zont1x 随机登顶」的输入失真。
 *
 * 数据源策略（按可靠性排序——**HLTV 直连有 Cloudflare 反爬，沙箱/CI 实测 403**）：
 *
 *   1. `--fetch-github <raw-url>`  从 GitHub raw 拉取现成的 HLTV 预爬数据集
 *      （JSON 或 CSV；GitHub raw 实测可达，最稳的自动化路径）
 *   2. `--input <file.csv|json>`   用户手动下载的数据集（Kaggle/GitHub 任意来源）
 *   3. `--scrape-hltv`             直爬 HLTV 统计榜（内置 UA/延迟/重试/退避；
 *      **被 403 拦截时给出明确的本地运行指引**）
 *
 * 用法示例：
 *   node scripts/scrape_hltv.mjs --fetch-github https://raw.githubusercontent.com/<user>/<repo>/<branch>/data.csv --columns player,rating
 *   node scripts/scrape_hltv.mjs --input downloads/hltv_2024.csv --columns player,rating
 *   node scripts/scrape_hltv.mjs --scrape-hltv            # 本地网络下直爬（尝试）
 *   node scripts/scrape_hltv.mjs --merge                  # 与现有 player_ratings.json 合并
 *   node scripts/scrape_hltv.mjs --out /tmp/x.json        # 自定义输出路径（CI/测试）
 *   node scripts/scrape_hltv.mjs --dry-run                # 只报告覆盖度，不写文件
 *
 * 依赖：Node.js ≥ 18（零 npm 依赖——fetch/fs 内置）。
 * 输出：assets/player_ratings.json（带 metadata 字段；后端 serde 只读 players[]，忽略其余）。
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// —— 常量 ——
const OUT_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "assets",
  "player_ratings.json",
);
const STANDINGS_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "assets",
  "standings_global_2026_01_05.json",
);

const HLTV_STATS_URL = (offset = 0) =>
  `https://www.hltv.org/stats/players?startDate=2024-01-01&endDate=2025-12-31&offset=${offset}`;

const USER_AGENTS = [
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
];

const REQUEST_DELAY_MS = 1500; // 礼貌延迟（两次请求之间）
const MAX_RETRIES = 4; // 重试次数
const BACKOFF_BASE_MS = 2000; // 指数退避基数
const TIMEOUT_MS = 20_000;
const RATING_MIN = 0.5;
const RATING_MAX = 1.6; // 2.0 评级量纲内的合理上限（防脏数据）

/** HLTV 昵称 → standings 名字的已知拼写差异（覆盖报告用）。 */
const ALIASES = {
  "kaiR0N-": "KaiR0N-",
  "KaiR0N": "KaiR0N-",
  "dev1ce": "device",
  "XANTARES": "XANTARES",
  "NertZ": "NertZ",
  "s1ren": "S1ren",
  "zorte": "zorte",
  "TRAVIS": "Travis",
  "F1KU": "f1ku",
  "INS": "INS",
  "saffee": "saffee",
  "WOOD7": "wood7",
  "Krimbo": "krimbo",
  "electronic": "electroNic",
  "heavygod": "HeavyGod",
};

// —— 参数解析 ——
const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const has = (name) => args.includes(name);

// —— 工具 ——
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function pick(arr) {
  return arr[Math.floor(Math.random() * arr.length)];
}

/** 指数退避重试包装。 */
async function fetchWithRetry(url, { retries = MAX_RETRIES, headers = {} } = {}) {
  let lastErr = null;
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      const res = await fetch(url, {
        headers: {
          "User-Agent": pick(USER_AGENTS),
          Accept: "text/html,application/json,text/csv,*/*",
          "Accept-Language": "en-US,en;q=0.9",
          ...headers,
        },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      if (res.status === 403 || res.status === 429) {
        lastErr = new Error(`HTTP ${res.status}（疑似反爬拦截/限流）`);
        // 403/429 也按退避重试（Cloudflare 有时对后续带 cookie 的请求放行）
      } else if (!res.ok) {
        lastErr = new Error(`HTTP ${res.status}`);
      } else {
        return res;
      }
    } catch (e) {
      lastErr = e;
    }
    const backoff = BACKOFF_BASE_MS * 2 ** attempt;
    console.warn(`  ⚠ 请求失败（${lastErr.message}），${backoff}ms 后重试 ${attempt + 1}/${retries}…`);
    await sleep(backoff);
  }
  throw lastErr;
}

/** 名字归一化：去首尾空白、保留大小写（standings 用精确大小写键控）。 */
function normalizeName(raw) {
  const s = String(raw ?? "").trim().replace(/\s+/g, " ");
  return ALIASES[s] ?? s;
}

/** 评级清洗：非法值丢弃（返回 null）。 */
function cleanRating(raw) {
  const v = parseFloat(String(raw).replace(",", "."));
  if (Number.isNaN(v) || v < RATING_MIN || v > RATING_MAX) return null;
  return Math.round(v * 1000) / 1000;
}

// —— 数据源 1：CSV / JSON 文件解析 ——
function parseCsv(text, columnsSpec) {
  const lines = text.split(/\r?\n/).filter((l) => l.trim());
  if (lines.length === 0) throw new Error("CSV 为空");
  // 简单 CSV 解析（支持引号包裹，不依赖 npm）
  const parseLine = (line) => {
    const out = [];
    let cur = "";
    let inQ = false;
    for (const ch of line) {
      if (ch === '"') inQ = !inQ;
      else if (ch === "," && !inQ) {
        out.push(cur);
        cur = "";
      } else cur += ch;
    }
    out.push(cur);
    return out;
  };
  const header = parseLine(lines[0]).map((h) => h.trim());
  const [nameCol, ratingCol] = columnsSpec
    ? columnsSpec.split(",").map((s) => s.trim())
    : ["player", "rating"];
  const nameIdx = header.findIndex((h) => h.toLowerCase() === nameCol.toLowerCase());
  const ratingIdx = header.findIndex((h) => h.toLowerCase() === ratingCol.toLowerCase());
  if (nameIdx < 0 || ratingIdx < 0) {
    throw new Error(`CSV 表头 ${header.join(",")} 中找不到列 ${nameCol}/${ratingCol}（用 --columns 指定）`);
  }
  const map = new Map();
  for (const line of lines.slice(1)) {
    const cols = parseLine(line);
    const name = normalizeName(cols[nameIdx]);
    const rating = cleanRating(cols[ratingIdx]);
    if (name && rating !== null) map.set(name, rating);
  }
  return map;
}

function parseJson(text) {
  const data = JSON.parse(text);
  const list = Array.isArray(data) ? data : data.players ?? data.data ?? data.rows ?? [];
  const map = new Map();
  for (const row of list) {
    const name = normalizeName(row.player ?? row.name ?? row.nickname);
    const rating = cleanRating(row.rating ?? row.rating2 ?? row.hltv_rating);
    if (name && rating !== null) map.set(name, rating);
  }
  return map;
}

// —— 数据源 2：HLTV 统计榜（best-effort；403 给出本地运行指引）——
/** 从 HLTV 统计榜 HTML 里正则抽取 (昵称, rating)。页面结构变化时可能失效。 */
function parseHltvLeaderboard(html) {
  const map = new Map();
  // HLTV 统计榜行结构：<td class="playerCol">…<a href="/player/…">nickname</a>…</td>…
  // 后接 <td class="ratingCol">1.23</td>（2.0 评级）
  const rowRe =
    /<td class="playerCol">[\s\S]*?<a href="\/player\/[^"]*"[^>]*>([^<]+)<\/a>[\s\S]*?<td class="ratingCol">([\d.]+)<\/td>/gi;
  let m;
  while ((m = rowRe.exec(html)) !== null) {
    const name = normalizeName(m[1]);
    const rating = cleanRating(m[2]);
    if (name && rating !== null) map.set(name, rating);
  }
  return map;
}

async function scrapeHltvLeaderboard(maxOffset = 200, step = 50) {
  const map = new Map();
  let offset = 0;
  let emptyPages = 0;
  while (offset <= maxOffset) {
    const url = HLTV_STATS_URL(offset);
    console.log(`  爬取 ${url}`);
    let res;
    try {
      res = await fetchWithRetry(url);
    } catch (e) {
      if (e.message.includes("403")) {
        console.error(`
╔══════════════════════════════════════════════════════════════════╗
║  HLTV 直连被 403 拦截（Cloudflare 反爬）。                         ║
║  请在本地浏览器/真实网络环境运行本脚本，或用更稳的数据源：          ║
║    node scripts/scrape_hltv.mjs --fetch-github <raw.json|csv 地址>  ║
║    node scripts/scrape_hltv.mjs --input <本地下载的数据集>          ║
║  （GitHub raw 实测可直连——推荐先找现成的 HLTV 预爬数据集）          ║
╚══════════════════════════════════════════════════════════════════╝`);
      }
      throw e;
    }
    const html = await res.text();
    const page = parseHltvLeaderboard(html);
    if (page.size === 0) {
      emptyPages += 1;
      if (emptyPages >= 2) break; // 连续空页 → 到尾/被换页脚本
    } else {
      emptyPages = 0;
      for (const [k, v] of page) map.set(k, v);
      console.log(`  本页 ${page.size} 名选手，累计 ${map.size}`);
    }
    offset += step;
    await sleep(REQUEST_DELAY_MS);
  }
  return map;
}

// —— 覆盖度报告（与 standings 阵容交叉核对）——
function coverageReport(ratingMap) {
  const standings = JSON.parse(readFileSync(STANDINGS_PATH, "utf8"));
  const rosterNames = [...new Set(standings.rankings.flatMap((t) => t.roster))];
  const covered = rosterNames.filter((n) => ratingMap.has(n));
  const missing = rosterNames.filter((n) => !ratingMap.has(n));
  return { total: rosterNames.length, covered: covered.length, missing };
}

// —— 主流程 ——
async function main() {
  const dryRun = has("--dry-run");
  const merge = has("--merge");
  const columnsSpec = opt("--columns");

  let ratingMap = new Map();
  let source = "manual-v1";

  // 合并模式：以现有 player_ratings.json 为底
  if (merge && existsSync(OUT_PATH)) {
    const existing = JSON.parse(readFileSync(OUT_PATH, "utf8"));
    for (const p of existing.players ?? []) {
      const name = normalizeName(p.player);
      const rating = cleanRating(p.rating);
      if (name && rating !== null) ratingMap.set(name, rating);
    }
    console.log(`合并底稿：${ratingMap.size} 名（现有 player_ratings.json）`);
  }

  // 数据源：优先级 GitHub > 本地文件 > HLTV 直爬
  const githubUrl = opt("--fetch-github");
  const inputFile = opt("--input");
  if (githubUrl) {
    console.log(`从 GitHub raw 拉取：${githubUrl}`);
    const res = await fetchWithRetry(githubUrl);
    const text = await res.text();
    const ct = res.headers.get("content-type") ?? "";
    const parsed = ct.includes("json") ? parseJson(text) : parseCsv(text, columnsSpec);
    for (const [k, v] of parsed) ratingMap.set(k, v);
    source = `github:${githubUrl}`;
  } else if (inputFile) {
    console.log(`读取本地数据集：${inputFile}`);
    const text = readFileSync(inputFile, "utf8");
    const parsed = inputFile.endsWith(".json") ? parseJson(text) : parseCsv(text, columnsSpec);
    for (const [k, v] of parsed) ratingMap.set(k, v);
    source = `file:${inputFile}`;
  } else if (has("--scrape-hltv")) {
    console.log("直爬 HLTV 统计榜（best-effort）…");
    const scraped = await scrapeHltvLeaderboard();
    for (const [k, v] of scraped) ratingMap.set(k, v);
    source = "hltv-stats-2024-2025";
  } else {
    console.error("请指定数据源：--fetch-github <url> | --input <file> | --scrape-hltv（或 --dry-run 只看覆盖度）");
    process.exit(1);
  }

  // 覆盖度报告
  const cov = coverageReport(ratingMap);
  console.log(`\n覆盖度：standings 阵容 ${cov.total} 名真实选手中，已有评级 ${cov.covered} 名（${((cov.covered / cov.total) * 100).toFixed(1)}%）`);
  if (cov.missing.length > 0) {
    console.log(`缺失（仍会随机膨胀，建议补全）：${cov.missing.slice(0, 40).join(", ")}${cov.missing.length > 40 ? ` …等 ${cov.missing.length} 名` : ""}`);
  }

  const out = {
    generated_at: new Date().toISOString(),
    source,
    player_count: ratingMap.size,
    players: [...ratingMap.entries()]
      .map(([player, rating]) => ({ player, rating }))
      .sort((a, b) => b.rating - a.rating || a.player.localeCompare(b.player)),
  };

  if (dryRun) {
    console.log(`\n[dry-run] 不写文件。输出预览（前 10）：`);
    out.players.slice(0, 10).forEach((p) => console.log(`  ${p.player} = ${p.rating}`));
    return;
  }

  const outPath = opt("--out") ? path.resolve(opt("--out")) : OUT_PATH;
  writeFileSync(outPath, JSON.stringify(out, null, 2) + "\n", "utf8");
  console.log(`\n已写入 ${outPath}（${out.player_count} 名选手）`);
  console.log("后端校验：重启 csc-server 即可生效（RatingBaseline::from_json_str 只读 players[]）。");
}

main().catch((e) => {
  console.error(`\n✗ 失败：${e.message}`);
  process.exit(1);
});
