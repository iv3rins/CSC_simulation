#!/usr/bin/env node
// verify-transfer-market.mjs — confirm transfer-market prompt renders (light HLTV, Chinese).
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9336;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-transfer-ud";
const URL = "file:///D:/repo-by-iverins/cs-career-simulation/docs/screenshots/ovn-transfer-market-harness.html";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [`--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`, `--user-data-dir=${UD}`, `--window-size=760,720`, URL], { stdio: "ignore" });
await sleep(2500);

let target;
try {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  target = list.find((t) => t.type === "page" && t.url.includes("ovn-transfer-market")) || list.find((t) => t.type === "page");
  if (!target) throw new Error("no page");
} catch (e) { console.error("CDP fail:", e.message); proc.kill(); process.exit(2); }

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 0; const pending = new Map();
ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
await send("Runtime.enable");
await sleep(400);

// 验证浅色 HLTV 计算样式
const css = async (sel) => (await send("Runtime.evaluate", { expression: `(() => { const el = document.querySelector(${JSON.stringify(sel)}); if(!el) return null; const c=getComputedStyle(el); return {bg:c.backgroundColor, border:c.borderTopColor, color:c.color}; })()`, returnByValue: true })).result?.result?.value;
const text = await send("Runtime.evaluate", { expression: "document.body.innerText", returnByValue: true });
const body = text.result?.result?.value ?? "";
const must = ["转会窗 · 报价提示", "选择接受一份报价，或留队", "有效期至 2027-01-31", "接受后即与目标队签约", "Patins da Ferrari", "全价报价", "GenOne", "VRS #104", "入队实力 +0.1", "年薪 ¥74,000", "接受报价", "留队（续约）", "稍后决定"];
let pass = true;
for (const s of must) { const ok = body.includes(s); if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${s}`); }

const card = await css(".transfer-market");
const cand = await css(".transfer-candidate");
console.log("  transfer-market bg:", card?.bg, "border:", card?.border);
console.log("  transfer-candidate bg:", cand?.bg);
if (card?.bg !== "rgb(255, 255, 255)") { pass = false; console.log("  [FAIL] market card should be white bg"); } else console.log("  [PASS] market card white bg");
if (cand?.bg !== "rgb(255, 255, 255)") { pass = false; console.log("  [FAIL] candidate should be white"); } else console.log("  [PASS] candidate white bg");

await send("Page.enable"); await sleep(300);
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot?.result?.data) { const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-transfer-market.png"; writeFileSync(out, Buffer.from(shot.result.data, "base64")); console.log("screenshot ->", out, statSync(out).size, "bytes"); }
ws.close(); proc.kill();
console.log(pass ? "RESULT: PASS — transfer market prompt renders (light HLTV, Chinese)" : "RESULT: FAIL");
process.exit(pass ? 0 : 1);
