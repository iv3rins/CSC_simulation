// live-cycle.mjs — 驱动一个 LIVE 会话：advance 直到 map 结束 / 决策点；支持 --auto-decide。
import { setTimeout as sleep } from "node:timers/promises";

const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const GAME = process.argv[2] ?? "0";
const MATCH = process.argv[3] ?? "LIVE-TEST-001";
const AUTO = process.argv.includes("--auto-decide");
const MAX = Number(process.argv.find((a, i) => process.argv[i - 1] === "--max") ?? "40");

async function post(path, body, what) {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`${what} HTTP ${res.status}: ${JSON.stringify(data)?.slice(0, 200)}`);
  return data;
}
async function get(path, what) {
  const res = await fetch(`${BASE}${path}`);
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(`${what} HTTP ${res.status}`);
  return data;
}

const decisionsTaken = [];
for (let i = 0; i < MAX; i++) {
  const av = await post(`/games/${GAME}/live/${encodeURIComponent(MATCH)}/advance`, null, "advance");
  const rn = av.state.round_number - 1;
  const line = `R${rn} score ${av.state.score_a}:${av.state.score_b} hist=${av.state.round_history.length} dec_req=${av.decision_required} fin=${av.output.map_finished}`;
  console.log(line);
  if (av.output.map_finished) {
    console.log(`MAP_FINISHED ${av.state.score_a}:${av.state.score_b} total_rounds=${av.state.round_history.length}`);
    const review = await get(`/games/${GAME}/live/${encodeURIComponent(MATCH)}/review`, "review");
    console.log("REVIEW:", JSON.stringify(review).slice(0, 1200));
    process.exit(0);
  }
  if (av.decision_required && av.decision_request) {
    console.log(`DECISION_REQUIRED round=${av.state.round_number} reason=${av.decision_request.reason} candidates=${(av.decision_request.candidates ?? []).join(",")}`);
    // advance 应 409
    try {
      await post(`/games/${GAME}/live/${encodeURIComponent(MATCH)}/advance`, null, "advance-while-pending");
      console.log("BUG: advance succeeded while decision pending");
    } catch (e) {
      console.log(`advance-while-pending correctly rejected: ${e.message.slice(0, 80)}`);
    }
    if (!AUTO) {
      process.exit(0); // 人工模式停在决策点
    }
    const pick = av.decision_request.candidates[0];
    const d = await post(`/games/${GAME}/live/${encodeURIComponent(MATCH)}/decide`, { decision: pick }, "decide");
    decisionsTaken.push(pick);
    console.log(`decided ${pick}`);
    await sleep(200);
  }
}
console.log(`max iters (${MAX}); decisions=${decisionsTaken.join(",")}`);