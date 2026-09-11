// ovn-matchday-trace.mjs — 慢速日推逼近比赛日，验证 live/today 窗口（每步确认日期只+1）。
import { setTimeout as sleep } from "node:timers/promises";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "19";
const MAX = Number(process.argv[3] ?? "60");

async function get(path) {
  const res = await fetch(`${BASE}/games/${GAME}${path}`);
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`GET ${path} HTTP ${res.status}`);
  return data;
}

async function flush() {
  for (let i = 0; i < 10; i++) {
    const { pending } = await get("/decisions/pending");
    if (!pending || !pending.points?.length) return;
    const decisions = pending.points.map((pt) => {
      const kind = Object.keys(pt)[0];
      const d = pt[kind];
      const opts = d.options ?? [];
      let option_id;
      if (kind === "LIFE_EVENT") option_id = d.kind === "MATCH_FIXER_CONTACT" ? "REFUSE" : (d.kind === "TEAMMATE_POWER_STRUGGLE" ? "MEDIATE" : (opts[0]?.id ?? "DECLINE"));
      else if (kind === "TRANSFER_WINDOW") option_id = d.offers?.[0]?.candidates?.[0] ? "STAY" : "STAY";
      else option_id = opts[0]?.id ?? opts[0]?.focus ?? "DEFAULT";
      return { point_id: d.id, option_id };
    });
    const res = await fetch(`${BASE}/games/${GAME}/decisions`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: pending.batch_id }) });
    if (!res.ok) throw new Error(`flush HTTP ${res.status}`);
    await sleep(200);
  }
}

let lastDate = null;
for (let d = 0; d < MAX; d++) {
  await flush();
  const lt = await get("/live/today");
  if (lt.matches?.length > 0) {
    console.log(`MATCH DAY at ${new Date().toISOString()} day#${d}: ${JSON.stringify(lt.matches)}`);
    // 窗口应跨请求持续
    await sleep(600);
    const again = await get("/live/today");
    console.log(`persists: ${again.matches?.length > 0}`);
    process.exit(0);
  }
  // advance 1 天
  for (let attempt = 0; attempt < 5; attempt++) {
    const res = await fetch(`${BASE}/games/${GAME}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ days: 1 }) });
    if (res.ok) break;
    if (res.status === 409) { await sleep(800); continue; }
    throw new Error(`advance HTTP ${res.status}: ${(await res.text()).slice(0, 120)}`);
  }
  await sleep(900);
  const v = await get("/view");
  const date = v.date;
  if (date !== lastDate) {
    lastDate = date;
    console.log(`day#${d} → ${date}`);
  } else {
    // 日期没变：可能仍在推进中，多等一拍
    await sleep(1000);
    const v2 = await get("/view");
    if (v2.date !== lastDate) { lastDate = v2.date; console.log(`day#${d} (+retry) → ${v2.date}`); }
    else { console.log(`STALL at ${date} (advancing or waiting?)`); process.exit(1); }
  }
}
console.log(`no match day in ${MAX} steps; last=${lastDate}`);
process.exit(1);