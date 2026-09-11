// gametest-drive.mjs — 游戏测试驱动：模拟真实玩家推进世界（REST 权威通道）。
// 默认决策语义来自 shared_defaults.mjs（与后端 AutoDecisionSource 三方对齐，见 C-E2）。
// 用法：node scripts/gametest-drive.mjs <gameid> [--until-match] [--max-days N]
import { setTimeout as sleep } from "node:timers/promises";
import { defaultDecision } from "../scripts/shared_defaults.mjs";

const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";

async function json(res, what) {
  const body = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`${what} HTTP ${res.status}: ${body?.error ?? ""}`);
  return body;
}

async function pending() {
  const { pending } = await json(await fetch(`${BASE}/games/${GAME}/decisions/pending`), "pending");
  return pending;
}

async function submit(points, batchId) {
  const decisions = points.map(defaultDecision);
  await json(
    await fetch(`${BASE}/games/${GAME}/decisions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ decisions, batch_id: batchId }),
    }),
    "submit",
  );
}

async function view() {
  return json(await fetch(`${BASE}/games/${GAME}/view`), "view");
}

async function advance(days) {
  const res = await fetch(`${BASE}/games/${GAME}/advance`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ days }),
  });
  const body = await res.json().catch(() => null);
  if (!res.ok) return { ok: false, error: body?.error ?? `HTTP ${res.status}` };
  return { ok: true, status: body?.status };
}

const maxDays = Number(process.argv.find((a, i) => process.argv[i - 1] === "--max-days") ?? "400");
const untilMatch = process.argv.includes("--until-match");

let advanced = 0;
let paused = 0;
for (;;) {
  // 每轮：清空待决策 → 推进 14 天 → 循环
  let p = await pending();
  while (p && p.points?.length) {
    await submit(p.points, p.batch_id);
    paused += p.points.length;
    p = await pending();
    await sleep(400);
  }
  const v = await view();
  const cal = await json(
    await fetch(`${BASE}/games/${GAME}/calendar?year=${new Date(`${v.date}T00:00:00`).getFullYear()}`),
    "calendar",
  );
  const playerEvents = cal.player_events ?? [];
  const hasPendingFixture = playerEvents.some(
    (e) => (e.status === "active" || e.status === "future" || e.status === "scheduled") && (e.fixtures ?? []).length > 0,
  );
  if (untilMatch && hasPendingFixture) {
    console.log(JSON.stringify({ done: "match-ready", date: v.date, player_events: playerEvents.length, fixtures: playerEvents.flatMap((e) => e.fixtures ?? []).length, advanced, paused }));
    process.exit(0);
  }
  if (advanced >= maxDays) {
    console.log(JSON.stringify({ done: "max-days", date: v.date, player_events: playerEvents.length, advanced, paused }));
    process.exit(0);
  }
  const lastDate = v.date;
  const r = await advance(14);
  if (!r.ok) {
    console.log("advance error:", r.error);
    process.exit(1);
  }
  advanced += 14;
  await sleep(1500);
  const v2 = await view();
  if (v2.date === lastDate && paused === 0) {
    // 可能 advance 被拒（正在推进）
    console.log(JSON.stringify({ done: "stalled", date: v2.date, advanced, paused }));
    process.exit(1);
  }
  if (v2.date !== v.date) paused = 0; // 推进成功，重置本轮回溯
  if (advanced % 56 === 0) console.log(`progress: ${v2.date} days=${advanced} paused=${paused}`);
}