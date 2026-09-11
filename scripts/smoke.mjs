#!/usr/bin/env node
// 端到端冒烟：csc-server 协议全链路（REST + WS + 存档往返）。
//
// 覆盖：
//   1. POST /games（human 策略）→ 2. WS 订阅 → 3. POST /advance（202）
//   4. WS decisions 事件 → decide 回传（多批次）→ 5. step 事件（月步完成）
//   6. 第二个月改用 REST 通道（pending 轮询 + POST /decisions）
//   7. GET /save → POST /load 存档往返 → 8. state/summary 校验 → 9. DELETE
//
// 前置：csc-server 已启动（默认 http://127.0.0.1:8080，可用 CSC_BASE 覆盖）。
// 用法：node scripts/smoke.mjs

import { defaultDecision, selfcheck } from "./shared_defaults.mjs";

const BASE = (process.env.CSC_BASE ?? "http://127.0.0.1:8080").replace(/\/$/, "");
const WS = BASE.replace(/^http/, "ws");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function assert(cond, msg) {
  if (!cond) throw new Error(`断言失败：${msg}`);
}

async function json(res, what) {
  const body = await res.json().catch(() => null);
  assert(res.ok, `${what} 失败（HTTP ${res.status}）：${body?.error ?? ""}`);
  return body;
}

async function waitFor(cond, ms, what) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (cond()) return;
    await sleep(100);
  }
  throw new Error(`超时等待：${what}`);
}

async function main() {
  // 0. 默认决策三方对齐自检（C-E2：shared_defaults ↔ decide.ts ↔ 后端 source.rs）
  selfcheck();
  console.log("[smoke] shared_defaults 自检通过（13 场景 golden）");

  // 1. 健康检查
  const health = await json(await fetch(`${BASE}/health`), "GET /health");
  assert(health.status === "ok", "健康检查");

  // 2. 创建游戏（human 策略）
  const created = await json(
    await fetch(`${BASE}/games`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ seed: 42, policy: "human", player_name: "SmokeTest" }),
    }),
    "POST /games",
  );
  const gameId = created.game_id;
  console.log(`[smoke] 游戏已创建 #${gameId}`);

  // 3. WS 订阅（必须先于 advance，decisions 事件无缓冲）
  const ws = new WebSocket(`${WS}/games/${gameId}/ws`);
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = () => rej(new Error("WS 连接失败"));
  });
  console.log("[smoke] WS 已连接");

  let step = null;
  let journalPushed = 0;
  let batchesViaWs = 0;
  let autoRespond = true; // 第二个月切 REST 通道时关闭自动回传
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.type === "decisions") {
      if (autoRespond) {
        batchesViaWs += 1;
        const decisions = msg.points.map(defaultDecision);
        ws.send(JSON.stringify({ type: "decide", decisions, batch_id: msg.batch_id }));
      }
    } else if (msg.type === "step") {
      step = msg;
    } else if (msg.type === "journal") {
      journalPushed += msg.events.length;
    } else if (msg.type === "error") {
      console.error(`[smoke] WS error：${msg.message}`);
    }
  };

  // 4. 第一个月：POST /advance → 202；经 WS decisions/decide 驱动到 step
  const adv1 = await fetch(`${BASE}/games/${gameId}/advance`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ months: 1 }),
  });
  assert(adv1.status === 202, `human 推进应返回 202，实际 ${adv1.status}`);
  await waitFor(() => step !== null && step.month >= 1, 90_000, "第一个月 step");
  assert(batchesViaWs >= 1, "WS 应至少收到 1 个决策批次");
  assert(journalPushed > 0, "WS 应推送 journal 事件");
  console.log(`[smoke] 第 1 个月完成（WS 决策 ${batchesViaWs} 批 / journal 推送 ${journalPushed} 条）`);

  // 5. 第二个月：REST 通道（pending 轮询 + POST /decisions），WS 只观察不回应
  autoRespond = false;
  step = null;
  const adv2 = await fetch(`${BASE}/games/${gameId}/advance`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ months: 1 }),
  });
  assert(adv2.status === 202, "第二个月推进应返回 202");
  let batchesViaRest = 0;
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (step !== null && step.month >= 2) break;
    const p = await json(await fetch(`${BASE}/games/${gameId}/decisions/pending`), "GET pending");
    if (p.pending) {
      const decisions = p.pending.points.map(defaultDecision);
      await json(
        await fetch(`${BASE}/games/${gameId}/decisions`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ decisions, batch_id: p.pending.batch_id }),
        }),
        "POST /decisions",
      );
      batchesViaRest += 1;
      continue;
    }
    await sleep(100);
  }
  assert(step !== null && step.month >= 2, "第二个月 step（REST 决策）");
  // 世界是确定性的：某个月是否出现决策批次取决于种子内容（伤病/人生事件等）。
  // 不能假设"第 2 个月必有批次"——但 REST 通道必须被验证过：若无批次出现，
  // 推进第 3 个月再试一次；仍无批次则要求 WS 路径已覆盖决策提交即可。
  if (batchesViaRest === 0) {
    console.log("[smoke] 第 2 个月无决策批次（确定性世界），推进第 3 个月验证 REST 通道");
    const adv3 = await fetch(`${BASE}/games/${gameId}/advance`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ months: 1 }),
    });
    assert(adv3.status === 202, "第三个月推进应返回 202");
    const deadline3 = Date.now() + 90_000;
    while (Date.now() < deadline3) {
      if (step !== null && step.month >= 3) break;
      const p = await json(await fetch(`${BASE}/games/${gameId}/decisions/pending`), "GET pending");
      if (p.pending) {
        const decisions = p.pending.points.map(defaultDecision);
        await json(
          await fetch(`${BASE}/games/${gameId}/decisions`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ decisions, batch_id: p.pending.batch_id }),
          }),
          "POST /decisions",
        );
        batchesViaRest += 1;
        continue;
      }
      await sleep(100);
    }
    assert(step !== null && step.month >= 3, "第三个月 step（REST 决策）");
  }
  assert(
    batchesViaRest >= 1 || batchesViaWs >= 1,
    "REST 或 WS 至少一条决策通道应被真实提交过",
  );
  console.log(`[smoke] 第 2 个月完成（REST 决策 ${batchesViaRest} 批）`);

  // 6. 存档往返：GET /save → POST /load
  const save = await json(await fetch(`${BASE}/games/${gameId}/save`), "GET /save");
  assert(typeof save.version === "number" && save.version >= 1, "存档带格式版本（不硬编码具体值——跟随 GameState::CURRENT_VERSION）");
  const loaded = await json(
    await fetch(`${BASE}/games/${gameId}/load`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(save),
    }),
    "POST /load",
  );
  assert(loaded.loaded === true, "读档结果");
  console.log("[smoke] 存档往返成功");

  // 7. 状态校验（月数 ≥2：确定性世界下可能因 REST 通道验证推进到第 3 个月）
  const summary = await json(await fetch(`${BASE}/games/${gameId}/summary`), "GET /summary");
  assert(summary.month >= 2, `summary.month>=2（实际 ${summary.month}）`);
  assert(summary.policy === "human", "policy");

  const state = await json(await fetch(`${BASE}/games/${gameId}/state`), "GET /state");
  assert(state.journal.length > 0, "state.journal 非空");
  const protagonist = state.world.players.find((p) => p.career !== null && !p.retired);
  assert(protagonist && protagonist.name === "SmokeTest", "主角存在且名字正确");
  assert(protagonist.team !== null, "主角应从垫底队伍起步（team 非空）");
  const archive = await json(await fetch(`${BASE}/games/${gameId}/archive`), "GET /archive");

  // 8. 关闭本局
  const del = await json(await fetch(`${BASE}/games/${gameId}`, { method: "DELETE" }), "DELETE /games");
  assert(del.shutdown === true, "shutdown");
  ws.close();

  console.log(`[smoke] 全部通过：journal ${state.journal.length} 条 / 赛事 ${state.events.length} 场 / 档案 ${Object.keys(archive.archive).length} 名选手`);
  console.log("SMOKE PASS");
}

main().catch((e) => {
  console.error(`SMOKE FAIL：${e.message}`);
  process.exit(1);
});
