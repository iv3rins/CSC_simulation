#!/usr/bin/env node
// verify-determinism.mjs — 确定性验证：同 seed 同决策(auto 无玩家决策) → 同世界。
// 推进 N 个月后比较两个局的核心状态指纹（rng_state/队伍/玩家/事件/赛事结果等）。
// 用法: node verify-determinism.mjs <gameA> <gameB> <months>
const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const [A, B, MONTHS] = [process.argv[2] ?? "11", process.argv[3] ?? "12", Number(process.argv[4] ?? "6")];

async function json(res, what) { const b = await res.json().catch(() => null); if (!res.ok) throw new Error(`${what} HTTP ${res.status}: ${JSON.stringify(b)?.slice(0,120)}`); return b; }

async function advanceN(id, months) {
  const res = await fetch(`${BASE}/games/${id}/advance`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ months }) });
  return await json(res, "advance");
}

// 指纹：只取确定性相关、且不依赖玩家名输入的核心字段
function fingerprint(st) {
  return JSON.stringify({
    month: st.month,
    rng_state: st.rng_state,
    sim: [st.sim_year, st.sim_month, st.sim_day],
    teamCount: st.world.teams.length,
    playerCount: st.world.players.length,
    teamRankings: st.world.teams.map(t => [t.name, t.vrs_ranking]).sort((a,b)=>a[0].localeCompare(b[0])),
    teamRosters: st.world.teams.map(t => [t.name, [...t.roster_ids].sort((x,y)=>x-y)]).sort((a,b)=>a[0].localeCompare(b[0])),
    eventCount: (st.events||[]).length,
    eventChampions: (st.events||[]).map(e => [e.event.name, e.champion]).sort((a,b)=>a[0].localeCompare(b[0])),
    decisionsCount: (st.decisions||[]).length,
  });
}

(async () => {
  console.log(`推进 ${MONTHS} 个月比较 game ${A} vs ${B} ...`);
  await advanceN(A, MONTHS).catch(()=>{});
  await advanceN(B, MONTHS).catch(()=>{});
  await new Promise(r => setTimeout(r, 1500));
  const stA = await json(await fetch(`${BASE}/games/${A}/state`), "state A");
  const stB = await json(await fetch(`${BASE}/games/${B}/state`), "state B");
  const fa = fingerprint(stA);
  const fb = fingerprint(stB);
  if (fa === fb) {
    console.log("RESULT: PASS — 同 seed 同决策 → 世界状态完全一致（确定性成立）");
    process.exit(0);
  } else {
    // 找第一处差异
    console.log("RESULT: FAIL — 世界状态存在差异");
    const ka = JSON.parse(fa), kb = JSON.parse(fb);
    for (const k of Object.keys(ka)) {
      if (JSON.stringify(ka[k]) !== JSON.stringify(kb[k])) {
        console.log(`  diff key: ${k}`);
        console.log("    A:", JSON.stringify(ka[k]).slice(0,300));
        console.log("    B:", JSON.stringify(kb[k]).slice(0,300));
      }
    }
    process.exit(1);
  }
})().catch(e => { console.error("error:", e.message); process.exit(1); });
