#!/usr/bin/env node
// drive-to-live.mjs — advance a human game (draining non-transfer decisions) until
// GET /live/today returns a non-empty matches (active tournament + pending player fixture).
// 默认决策语义来自 shared_defaults.mjs（与后端 AutoDecisionSource 三方对齐，见 C-E2）；
// TRANSFER_WINDOW 保留挂起（返回 null），由调用方决定。
import { setTimeout as sleep } from "node:timers/promises";
import { defaultDecision } from "../scripts/shared_defaults.mjs";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";
const MAX = Number(process.argv[3] ?? "300");

async function json(res, what) { const b = await res.json().catch(() => null); if (!res.ok) throw new Error(`${what} ${res.status}: ${b?.error ?? ""}`); return b; }
async function drain() {
  for (let i = 0; i < 30; i++) {
    const { pending } = await json(await fetch(`${BASE}/games/${GAME}/decisions/pending`), "pending");
    if (!pending || !pending.points?.length) return;
    // TRANSFER_WINDOW 保留挂起（不提交转会窗决策）
    const decisions = pending.points.filter((p) => Object.keys(p)[0] !== "TRANSFER_WINDOW").map(defaultDecision).filter(Boolean);
    if (decisions.length) { await json(await fetch(`${BASE}/games/${GAME}/decisions`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: pending.batch_id }) }), "submit"); }
    await sleep(150);
  }
}
async function liveCount() {
  try { const r = await fetch(`${BASE}/games/${GAME}/live/today`); const j = await r.json(); return j.matches?.length ?? 0; } catch { return 0; }
}
for (let step = 0; step < MAX; step++) {
  await drain();
  if ((await liveCount()) > 0) {
    const r = await fetch(`${BASE}/games/${GAME}/live/today`); const j = await r.json();
    console.log(JSON.stringify({ found: true, matches: j.matches }));
    process.exit(0);
  }
  try {
    const res = await fetch(`${BASE}/games/${GAME}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ days: 14 }) });
    if (res.status !== 200 && res.status !== 202 && res.status !== 409) { const b = await res.json().catch(() => null); console.error("advance", res.status, b?.error); process.exit(1); }
  } catch {}
  await sleep(200);
}
console.log(JSON.stringify({ found: false }));
process.exit(1);
