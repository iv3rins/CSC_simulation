#!/usr/bin/env node
// 容量压测（Sprint 2 验收）：1/5/10 个长生涯档的并发保存 / 并发读档。
//
// 前置：先启动 release 服务端：
//   cd backend && cargo run --release -p csc-server -- ../assets 8080
// 用法：
//   node scripts/capacity_stress.mjs [games=10] [years=20] [base=http://127.0.0.1:8080]
// 输出：JSON 压测摘要（推进时长、保存/读档 P50/P95/max、gzip 体积与 CRC 校验）。

const GAMES = Number(process.argv[2] ?? 10);
const YEARS = Number(process.argv[3] ?? 20);
const BASE = process.argv[4] ?? "http://127.0.0.1:8080";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function req(path, init = {}, timeoutMs = 600_000) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const res = await fetch(`${BASE}${path}`, { ...init, signal: ctrl.signal });
    return res;
  } finally {
    clearTimeout(timer);
  }
}

async function json(res, label) {
  const text = await res.text();
  if (!res.ok) throw new Error(`${label} HTTP ${res.status}: ${text.slice(0, 300)}`);
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

const started = Date.now();
const ids = [];

// 1) 建局：不同 seed，Auto 策略（压测不牵涉 Human 决策通道）。
for (let i = 0; i < GAMES; i++) {
  const res = await req("/games", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ seed: 2000 + i, policy: "auto", player_name: `Soak${i + 1}` }),
  });
  const body = await json(res, "POST /games");
  ids.push(body.game_id);
}

// 2) 逐局推进到目标年限（服务端同步推进；LRU 会持续把空闲局 gzip 落盘）。
const advanceMs = [];
for (let i = 0; i < ids.length; i++) {
  const t = Date.now();
  const res = await req(`/games/${ids[i]}/advance`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ months: YEARS * 12 }),
  });
  await json(res, `POST /games/${ids[i]}/advance`);
  advanceMs.push(Date.now() - t);
  if (process.env.VERBOSE) console.error(`game ${ids[i]} advance ${YEARS}y ${advanceMs.at(-1)}ms`);
}

// 3) 并发保存：10 局长档同时取 gzip，测量墙钟 P95 与 CRC。
async function saveOne(id) {
  const t = Date.now();
  const res = await req(`/games/${id}/save/gzip`);
  if (!res.ok) throw new Error(`save ${id} HTTP ${res.status}: ${(await res.text()).slice(0, 200)}`);
  const bytes = new Uint8Array(await res.arrayBuffer());
  const jsonBytes = Number(res.headers.get("x-csc-json-bytes") ?? 0);
  const crc = res.headers.get("x-csc-save-crc32") ?? "";
  return { id, ms: Date.now() - t, gzip: bytes.length, jsonBytes, crc, bytes };
}

const saves = await Promise.all(ids.map(saveOne));

// 4) 并发读档：把刚拿到的 gzip 原样读回同一局，校验 CRC 后确认 restore 成功。
async function loadOne({ id, crc, bytes }) {
  const t = Date.now();
  const res = await req(`/games/${id}/load/gzip`, {
    method: "POST",
    headers: { "content-type": "application/gzip", "x-csc-expected-crc32": crc },
    body: bytes,
  });
  await json(res, `POST /games/${id}/load/gzip`);
  return { id, ms: Date.now() - t };
}

const loads = await Promise.all(saves.map(loadOne));

// 5) 并发恢复后轻量视图可用性抽查（读档后立即 GET /view）。
const views = await Promise.all(
  ids.map(async (id) => {
    const t = Date.now();
    const res = await req(`/games/${id}/view`);
    const body = await json(res, `GET /games/${id}/view`);
    return { id, ms: Date.now() - t, ok: res.ok, date: body?.sim_date ?? body?.date ?? "?" };
  })
);

let health = await json(await req("/health"), "GET /health");

// 6) 等待后台容量收缩：/health 只排发 gzip 落盘，不阻塞；轮询到活跃数 ≤ 上限。
let shrink_ms = null;
const maxActive = health.max_active ?? null;
if (maxActive !== null && health.games > maxActive) {
  const t0 = Date.now();
  for (let i = 0; i < 240; i++) {
    await sleep(500);
    health = await json(await req("/health"), "GET /health（收缩轮询）");
    if (health.games <= maxActive) {
      shrink_ms = Date.now() - t0;
      break;
    }
  }
}

const pct = (xs, p) => {
  const a = [...xs].sort((a, b) => a - b);
  return a[Math.min(a.length - 1, Math.floor(((a.length - 1) * p) / 100))];
};
const sum = (xs) => xs.reduce((a, b) => a + b, 0);

const out = {
  generated_at: new Date().toISOString(),
  config: { base: BASE, games: GAMES, years: YEARS },
  wall_s: Number(((Date.now() - started) / 1000).toFixed(1)),
  advance_ms: { total: sum(advanceMs), p50: pct(advanceMs, 50), p95: pct(advanceMs, 95), max: Math.max(...advanceMs) },
  save_ms: { total: sum(saves.map((s) => s.ms)), p50: pct(saves.map((s) => s.ms), 50), p95: pct(saves.map((s) => s.ms), 95), max: Math.max(...saves.map((s) => s.ms)) },
  load_ms: { total: sum(loads.map((l) => l.ms)), p50: pct(loads.map((l) => l.ms), 50), p95: pct(loads.map((l) => l.ms), 95), max: Math.max(...loads.map((l) => l.ms)) },
  bytes: {
    gzip_total: sum(saves.map((s) => s.gzip)),
    gzip_mean: Math.round(sum(saves.map((s) => s.gzip)) / saves.length),
    gzip_min: Math.min(...saves.map((s) => s.gzip)),
    gzip_max: Math.max(...saves.map((s) => s.gzip)),
    json_total: sum(saves.map((s) => s.jsonBytes)),
    json_mean: Math.round(sum(saves.map((s) => s.jsonBytes)) / saves.length),
    crc_ok: saves.every((s) => /^[0-9a-f]{8}$/.test(s.crc)),
  },
  views: { ok: views.every((v) => v.ok), p95_ms: pct(views.map((v) => v.ms), 95), dates: views.map((v) => v.date) },
  health,
  shrink_ms,
  errors: [],
};

console.log(JSON.stringify(out, null, 2));
