// flush-pending.mjs — 提交当前 pending 批次（健壮处理任意决策类型），直到无批次。
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";
const MAX = Number(process.argv[3] ?? "30");
import { setTimeout as sleep } from "node:timers/promises";

async function pending() {
  const r = await fetch(`${BASE}/games/${GAME}/decisions/pending`);
  if (!r.ok) throw new Error(`pending HTTP ${r.status}`);
  return (await r.json()).pending;
}

function choose(pt) {
  const kind = Object.keys(pt)[0];
  const d = pt[kind];
  const opts = d.options ?? [];
  let option_id;
  if (kind === "LIFE_EVENT") {
    if (d.kind === "MATCH_FIXER_CONTACT") option_id = "REFUSE";
    else if (d.kind === "TEAMMATE_POWER_STRUGGLE") option_id = "MEDIATE";
    else option_id = opts[0]?.id ?? "DECLINE";
  } else if (kind === "TRANSFER_WINDOW") {
    const best = d.offers?.[0]?.candidates?.[0];
    const current = d.offers?.[0]?.current_ranking;
    const shouldJump = best && (current == null || best.ranking < current);
    option_id = shouldJump ? best.team_signature : "STAY";
  } else {
    option_id = opts[0]?.id ?? opts[0]?.focus ?? "DEFAULT";
  }
  return { point_id: d.id, option_id };
}

for (let i = 0; i < MAX; i++) {
  const p = await pending();
  if (!p || !p.points?.length) {
    console.log(`done: no pending batch (iter ${i})`);
    process.exit(0);
  }
  const decisions = p.points.map(choose);
  const r = await fetch(`${BASE}/games/${GAME}/decisions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ decisions, batch_id: p.batch_id }),
  });
  if (!r.ok) {
    console.log(`submit err: HTTP ${r.status} ${(await r.text()).slice(0, 200)}`);
    process.exit(1);
  }
  console.log(`submitted batch ${p.batch_id} @${p.date} pts=${decisions.length}`);
  await sleep(700);
}
console.log(`max iters reached (${MAX})`);