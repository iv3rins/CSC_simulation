#!/usr/bin/env node
// drive-to-transfer-window.mjs — poll a human game, draining non-transfer decisions,
// until a TRANSFER_WINDOW appears in pending. Tolerates the game's own auto-advance.
// 默认决策语义来自 shared_defaults.mjs（与后端 AutoDecisionSource 三方对齐，见 C-E2）；
// TRANSFER_WINDOW 保留挂起（返回 null）。
import { setTimeout as sleep } from "node:timers/promises";
import { defaultDecision } from "../scripts/shared_defaults.mjs";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";
const MAX = Number(process.argv[3] ?? "400");

async function json(res, what) { const b = await res.json().catch(() => null); if (!res.ok) throw new Error(`${what} ${res.status}: ${b?.error ?? ""}`); return b; }
async function drain() {
  for (let i = 0; i < 20; i++) {
    const { pending } = await json(await fetch(`${BASE}/games/${GAME}/decisions/pending`), "pending");
    if (!pending || !pending.points?.length) return null;
    const kinds = pending.points.map((p) => Object.keys(p)[0]);
    if (kinds.includes("TRANSFER_WINDOW")) return pending;
    // 非转会窗批次：全部按默认提交（batch_id 已携带）
    const decisions = pending.points.map(defaultDecision).filter(Boolean);
    if (decisions.length) { await json(await fetch(`${BASE}/games/${GAME}/decisions`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: pending.batch_id }) }), "submit"); }
    await sleep(250);
  }
  return null;
}
for (let step = 0; step < MAX; step++) {
  const tw = await drain();
  if (tw) { console.log(JSON.stringify({ found: true, batch: tw })); process.exit(0); }
  // try to nudge advance (tolerate 409 = game already advancing)
  try {
    const res = await fetch(`${BASE}/games/${GAME}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ days: 14 }) });
    if (res.status !== 200 && res.status !== 202 && res.status !== 409) { const b = await res.json().catch(() => null); console.error("advance", res.status, b?.error); process.exit(1); }
  } catch {}
  await sleep(300);
}
console.log(JSON.stringify({ found: false }));
process.exit(1);
