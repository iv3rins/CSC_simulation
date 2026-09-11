// ovn-live-gate-e2e.mjs — 验证 LIVE 闸门：逐日推进，比赛日 live/today 非空。
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "18";
const MAX_DAYS = Number(process.argv[3] ?? "90");
import { setTimeout as sleep } from "node:timers/promises";

async function get(path) {
  const res = await fetch(`${BASE}/games/${GAME}${path}`);
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`GET ${path} HTTP ${res.status}: ${data?.error ?? ""}`);
  return data;
}
async function post(path, body, what) {
  const res = await fetch(`${BASE}/games/${GAME}${path}`, {
    method: "POST",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`${what} HTTP ${res.status}: ${data?.error ?? ""}`);
  return data;
}

async function flushPending() {
  for (let i = 0; i < 12; i++) {
    const { pending } = await get("/decisions/pending");
    if (!pending || !pending.points?.length) return;
    const decisions = pending.points.map((pt) => {
      const kind = Object.keys(pt)[0];
      const d = pt[kind];
      const opts = d.options ?? [];
      let option_id;
      if (kind === "LIFE_EVENT") option_id = d.kind === "MATCH_FIXER_CONTACT" ? "REFUSE" : (d.kind === "TEAMMATE_POWER_STRUGGLE" ? "MEDIATE" : (opts[0]?.id ?? "DECLINE"));
      else if (kind === "TRANSFER_WINDOW") { const best = d.offers?.[0]?.candidates?.[0]; option_id = best && best.ranking < (d.offers[0]?.current_ranking ?? 9999) ? best.team_signature : "STAY"; }
      else option_id = opts[0]?.id ?? opts[0]?.focus ?? "DEFAULT";
      return { point_id: d.id, option_id };
    });
    await post("/decisions", { decisions, batch_id: pending.batch_id }, "flush");
    await sleep(300);
  }
}

let hit = null;
for (let d = 0; d < MAX_DAYS; d++) {
  await flushPending();
  const today = await get("/live/today");
  if (today.matches?.length > 0) {
    hit = { day: d, date_found: true, matches: today.matches };
    console.log(`MATCH DAY FOUND at day ${d}: ${JSON.stringify(hit.matches)}`);
    break;
  }
  await post("/advance", { days: 1 }, "advance-day");
  await sleep(400);
  if (d % 14 === 0) {
    const v = await get("/view");
    console.log(`day ${d}: date=${v.date}`);
  }
}
if (!hit) {
  const v = await get("/view");
  console.log(`NO match day found in ${MAX_DAYS} days; final date=${v.date}`);
  process.exit(1);
}
// 验证 pending 窗口跨请求持续（再查两次）
await sleep(500);
const again = await get("/live/today");
console.log(`live/today persists: ${again.matches?.length > 0} (${JSON.stringify(again.matches?.map(m => m.event_name))})`);
process.exit(0);