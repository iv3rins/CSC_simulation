#!/usr/bin/env node
// verify-transfer-backend.mjs — 直接用完整批次 API 提交转会决策，验证后端转会执行。
// 模拟前端 submitTransfer 的整批提交：TRANSFER_WINDOW → 第一候选(接受)，其余点(如 LIFE_EVENT) → 默认。
// 非转会窗默认决策来自 shared_defaults.mjs（与后端 AutoDecisionSource 三方对齐，见 C-E2）。
// 用法: node verify-transfer-backend.mjs <game_id>
import { defaultDecision } from "../scripts/shared_defaults.mjs";
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";

async function json(res, what) { const b = await res.json().catch(() => null); if (!res.ok) throw new Error(`${what} HTTP ${res.status}: ${JSON.stringify(b)?.slice(0,200)}`); return b; }

const { pending } = await json(await fetch(`${BASE}/games/${GAME}/decisions/pending`), "pending");
if (!pending) { console.log("no pending"); process.exit(1); }
console.log("batch_id=", pending.batch_id, "date=", pending.date, "points=", pending.points.length);

let acceptSig = null;
const decisions = pending.points.map((p) => {
  const [kind, d] = Object.entries(p)[0];
  let opt;
  if (kind === "TRANSFER_WINDOW") {
    acceptSig = d.offers[0].candidates[0].team_signature;
    opt = acceptSig; // 接受第一候选
    console.log("  TRANSFER_WINDOW -> ACCEPT:", acceptSig);
  } else {
    opt = defaultDecision(p).option_id; // shared_defaults：与后端 source.rs 对齐
    console.log("  ", kind, "-> default:", opt);
  }
  return { point_id: d.id, option_id: opt };
});

console.log("--- submitting full batch ---");
const res = await json(await fetch(`${BASE}/games/${GAME}/decisions`, {
  method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ decisions, batch_id: pending.batch_id }),
}), "submit");
console.log("submit:", JSON.stringify(res));

// 等模拟线程应用
await new Promise((r) => setTimeout(r, 1500));

// 验证：主角队伍应变为目标队（不再留原队），且有 TRANSFER_DONE
const st = await json(await fetch(`${BASE}/games/${GAME}/state`), "state");
const player = st.world.players.find((x) => x.career);
const team = st.world.teams.find((t) => t.id === player.team);
console.log("player team after submit:", team?.name, "(id", player.team, ")");
const transferDone = (st.journal || []).filter((e) => e.TRANSFER_DONE);
if (transferDone.length) {
  const td = transferDone[transferDone.length - 1].TRANSFER_DONE;
  console.log("TRANSFER_DONE:", td.player_name, td.from_team, "->", td.to_team);
} else {
  console.log("NO TRANSFER_DONE event");
}
