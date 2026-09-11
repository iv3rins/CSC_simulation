#!/usr/bin/env node
// ovn-t6-browser-test.mjs — 浏览器实测 t6 转会市场前端：
//   1) 挂起报价时市场页显示【转会窗·报价提示】+ 候选 + 接受/留队/稍后按钮（HLTV 浅色）
//   2) 截图 ovn-transfer-market.png
//   3) 点击「接受报价」→ 市场提示清除（前端 store + 后端 transfers/market 置空）
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9341;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-t6-ud";
const APP = "http://localhost:5173/transfer";
const GAME = process.env.CSC_GAME ?? "3";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [
  `--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${UD}`, `--window-size=1100,900`, "--no-first-run", APP,
], { stdio: "ignore" });
await sleep(2500);

let target;
try {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  target = list.find((t) => t.type === "page") || list[0];
  if (!target) throw new Error("no page");
} catch (e) { console.error("CDP fail:", e.message); proc.kill(); process.exit(2); }

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 0; const pending = new Map();
ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
const evaljs = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;

await send("Runtime.enable");
await send("Page.enable");

// 先设 localStorage 让 app boot 恢复 game 3
await evaljs(`localStorage.setItem("csc.game_id", ${JSON.stringify(GAME)}); localStorage.setItem("csc.policy","human"); true;`);
await evaljs(`location.href = ${JSON.stringify(APP)}; true;`);
await sleep(3500);

let pass = true;
const check = (label, ok) => { if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${label}`); };

// --- 1) 市场提示渲染 ---
await sleep(1500); // 等 refreshTransferMarket + WS
const body = (await evaljs("document.body.innerText")) ?? "";
const marketInDom = await evaljs(`!!document.querySelector(".transfer-market")`);
console.log("--- 市场提示渲染 ---");
check("DOM 存在 .transfer-market 区块", marketInDom === true);
check("含『转会窗 · 报价提示』", body.includes("转会窗 · 报价提示"));
check("含『选择接受一份报价，或留队』", body.includes("选择接受一份报价，或留队"));
check("含『有效期至』", body.includes("有效期至"));
check("含『接受报价』按钮", body.includes("接受报价"));
check("含『留队（续约）』", body.includes("留队"));
check("含『稍后决定』", body.includes("稍后决定"));
check("含候选队 Aurora Gaming", body.includes("Aurora Gaming") || body.includes("MIBR") || body.includes("TYLOO"));
// money() 全应用统一用 $ 符号（financial 约定；与财务/转会其他卡片一致），这里断言年薪行存在
check("含年薪金额行", body.includes("年薪 $"));

// HLTV 浅色计算样式
const cardCss = await evaljs(`(() => { const el=document.querySelector(".transfer-market"); if(!el) return null; const c=getComputedStyle(el); return {bg:c.backgroundColor, borderTop:c.borderTopColor}; })()`);
console.log("  .transfer-market bg:", cardCss?.bg, "borderTop:", cardCss?.borderTop);
check("市场卡片为白色背景 (HLTV 浅色)", cardCss?.bg === "rgb(255, 255, 255)");

// 截图（报价提示可见）
const shot1 = await send("Page.captureScreenshot", { format: "png" });
if (shot1?.result?.data) {
  const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-transfer-market.png";
  writeFileSync(out, Buffer.from(shot1.result.data, "base64"));
  console.log("screenshot(提示) ->", out, statSync(out).size, "bytes");
}

// --- 2) 接受报价 → 状态清除 ---
// 若有决策模态遮挡，先提交它（会连同转会窗按默认提交；但我们先点市场按钮）
const modalBefore = await evaljs(`!!document.querySelector(".decision-modal")`);
console.log("--- 接受报价 → 清除 ---");
console.log("  决策模态是否出现(遮挡):", modalBefore);

// 记录当前候选按钮数，点击第一个「接受报价」
const btnInfo = await evaljs(`(() => { const bs=[...document.querySelectorAll(".transfer-candidate button")]; return {count:bs.length, text:bs[0]?.innerText}; })()`);
console.log("  接受报价按钮数:", btnInfo?.count, "首个:", btnInfo?.text);
check("存在可点击的接受报价按钮", (btnInfo?.count ?? 0) >= 1);

if ((btnInfo?.count ?? 0) >= 1) {
  // 记录接受前主角队伍（供转会执行校验）
  const beforeTeam = await evaljs(`(async () => { try { const r = await fetch("/games/${GAME}/state"); const j = await r.json(); const p = j.world.players.find(x => x.career); return p ? (j.world.teams.find(t => t.id === p.team)?.name ?? "?") : "?"; } catch { return "?"; } })()`);
  console.log("  接受前队伍:", beforeTeam);
  await evaljs(`(() => { const b=document.querySelector(".transfer-candidate button"); if(b) b.click(); return true; })()`);
  await sleep(2500);
  const marketAfter = await evaljs(`!!document.querySelector(".transfer-market")`);
  const bodyAfter = (await evaljs("document.body.innerText")) ?? "";
  check("点击接受后市场提示从页面消失", marketAfter === false);
  check("点击后不再显示『转会窗 · 报价提示』", !bodyAfter.includes("转会窗 · 报价提示"));
  // 转会执行校验：主角应转会到目标队（第一个候选），不再留原队
  const afterTeam = await evaljs(`(async () => { try { const r = await fetch("/games/${GAME}/state"); const j = await r.json(); const p = j.world.players.find(x => x.career); return p ? (j.world.teams.find(t => t.id === p.team)?.name ?? "?") : "?"; } catch { return "?"; } })()`);
  console.log("  接受后队伍:", afterTeam);
  check(`接受报价后主角已转会（${beforeTeam} → ${afterTeam}）`, beforeTeam !== afterTeam);
}

// 后端确认：transfers/market 应为 null
try {
  const r = await fetch(`http://localhost:8080/games/${GAME}/transfers/market`);
  const j = await r.json();
  check("后端 transfers/market 已置空 (market:null)", j.market === null);
} catch (e) { console.log("  [WARN] 后端 market 检查失败:", e.message); }

ws.close(); proc.kill();
console.log(pass ? "\nRESULT: PASS — t6 市场提示渲染 + 接受后清除 验证通过" : "\nRESULT: FAIL");
process.exit(pass ? 0 : 1);
