// ovn-matchday-window.mjs — 推进到指定日期附近后逐日捕捉比赛日窗口（live/today 非空）。
import { setTimeout as sleep } from "node:timers/promises";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "19";
const TARGET = process.argv[3] ?? "2026-02-14"; // 提前到比赛日前的目标日期
const MAX = Number(process.argv[4] ?? "40");

async function get(path) {
  const res = await fetch(`${BASE}/games/${GAME}${path}`);
  return res.ok ? (await res.json()) : null;
}
async function flush() {
  for (let i = 0; i < 15; i++) {
    const p = await get("/decisions/pending");
    if (!p?.pending?.points?.length) return;
    const decisions = p.pending.points.map((pt) => {
      const kind = Object.keys(pt)[0];
      const d = pt[kind];
      const opts = d.options ?? [];
      let option_id;
      if (kind === "LIFE_EVENT") option_id = d.kind === "MATCH_FIXER_CONTACT" ? "REFUSE" : (d.kind === "TEAMMATE_POWER_STRUGGLE" ? "MEDIATE" : (opts[0]?.id ?? "DECLINE"));
      else option_id = opts[0]?.id ?? opts[0]?.focus ?? "DEFAULT";
      return { point_id: d.id, option_id };
    });
    const res = await fetch(`${BASE}/games/${GAME}/decisions`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: p.pending.batch_id }) });
    if (!res.ok) break;
    await sleep(150);
  }
}
async function advanceOne() {
  for (let i = 0; i < 8; i++) {
    const res = await fetch(`${BASE}/games/${GAME}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ days: 1 }) });
    if (res.ok) return;
    await sleep(700);
  }
  console.log("advance failed persistently");
  process.exit(1);
}

let date = null;
for (let d = 0; d < MAX; d++) {
  await flush();
  const lt = await get("/live/today");
  if (lt?.matches?.length > 0) {
    console.log(`🎯 MATCH DAY at ${date ?? "?"} → ${JSON.stringify(lt.matches)}`);
    const again = await get("/live/today");
    console.log(`persists: ${again?.matches?.length > 0}`);
    process.exit(0);
  }
  await advanceOne();
  await sleep(700);
  const v = await get("/view");
  if (v && v.date !== date) {
    date = v.date;
    console.log(`→ ${date}`);
    if (date >= TARGET) { console.log(`reached ${TARGET}, now in match-day window zone`); }
  } else {
    await sleep(800);
    const v2 = await get("/view");
    if (v2 && v2.date === date) { console.log(`STALL @ ${date}`); process.exit(1); }
    date = v2.date;
    console.log(`→ ${date} (retry)`);
  }
}
console.log(`no match day in ${MAX} steps; last=${date}`);
process.exit(1);