#!/usr/bin/env node
// drive-to-live-day.mjs — advance a human game DAY-BY-DAY (draining decisions), checking
// live/today after each day, until a pending player fixture appears (active event + pending fixture).
// Unlike the 14-day driver, day-by-day catches the narrow pending window before month-end settle.
// 默认决策语义来自 shared_defaults.mjs（与后端 AutoDecisionSource 三方对齐，见 C-E2）；
// TRANSFER_WINDOW 保留挂起（返回 null）。
import { setTimeout as sleep } from "node:timers/promises";
import { defaultDecision } from "../scripts/shared_defaults.mjs";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";
const MAX = Number(process.argv[3] ?? "420");

async function json(res, what) { const b = await res.json().catch(() => null); if (!res.ok) throw new Error(`${what} ${res.status}: ${b?.error ?? ""}`); return b; }
async function drain() {
  for (let i = 0; i < 30; i++) {
    const { pending } = await json(await fetch(`${BASE}/games/${GAME}/decisions/pending`), "pending");
    if (!pending || !pending.points?.length) return;
    // if a transfer window is pending, don't consume it but do consume the rest
    const decisions = pending.points.filter((p) => Object.keys(p)[0] !== "TRANSFER_WINDOW").map(defaultDecision).filter(Boolean);
    if (decisions.length) { await json(await fetch(`${BASE}/games/${GAME}/decisions`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: pending.batch_id }) }), "submit"); }
    await sleep(120);
  }
}
async function liveToday() {
  try { const r = await fetch(`${BASE}/games/${GAME}/live/today`); const j = await r.json(); return j.matches ?? []; } catch { return []; }
}
for (let step = 0; step < MAX; step++) {
  await drain();
  const m = await liveToday();
  if (m.length > 0) {
    console.log(JSON.stringify({ found: true, matches: m }));
    process.exit(0);
  }
  try {
    const res = await fetch(`${BASE}/games/${GAME}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ days: 1 }) });
    if (res.status !== 200 && res.status !== 202 && res.status !== 409) { const b = await res.json().catch(() => null); console.error("advance", res.status, b?.error); process.exit(1); }
  } catch {}
  await sleep(60);
}
console.log(JSON.stringify({ found: false }));
process.exit(1);
