#!/usr/bin/env node
// verify-watch-copy.mjs — confirm watch/replay i18n Chinese copy renders.
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9334;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-watch-verify-ud";
const URL = "file:///D:/repo-by-iverins/cs-career-simulation/docs/screenshots/ovn-watch-copy-harness.html";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [`--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`, `--user-data-dir=${UD}`, `--window-size=720,560`, URL], { stdio: "ignore" });
await sleep(2500);

let target;
try {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  target = list.find((t) => t.type === "page" && t.url.includes("ovn-watch-copy")) || list.find((t) => t.type === "page");
  if (!target) throw new Error("no page");
} catch (e) { console.error("CDP fail:", e.message); proc.kill(); process.exit(2); }

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 0; const pending = new Map();
ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
await send("Runtime.enable");
await sleep(500);

const text = await send("Runtime.evaluate", { expression: "document.body.innerText", returnByValue: true });
const body = text.result?.result?.value ?? "";
const must = ["赛果 · 赛后复盘", "系列赛获胜", "你的赛后复盘", "击杀", "阵亡", "场均伤害 ADR", "为何取胜", "生涯复盘", "个人发挥", "团队实力", "化学反应", "关键时刻", "关键决策复盘", "3 次决策", "系列 Rating", "拿下本场系列赛", "系列 MVP", "StyleVerify · 26 杀 · 19 死 · ADR 92.4", "系列 Rating 榜 · 按 Rating 排序", "赛果", "回放"];
let pass = true;
for (const s of must) { const ok = body.includes(s); if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${s}`); }
// must NOT contain raw english enums
const rawEnums = ["WHY WE WON", "WHY WE LOST", "CAREER REVIEW", "POST-MATCH REVIEW", "LIVE DECISIONS", "SERIES RATING"];
for (const s of rawEnums) { const bad = body.includes(s); if (bad) pass = false; console.log(`  [${bad ? "FAIL" : "PASS"}] no '${s}'`); }

await send("Page.enable"); await sleep(300);
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot?.result?.data) { const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-watch-copy.png"; writeFileSync(out, Buffer.from(shot.result.data, "base64")); console.log("screenshot ->", out, statSync(out).size, "bytes"); }
ws.close(); proc.kill();
console.log(pass ? "RESULT: PASS — watch copy is productized Chinese" : "RESULT: FAIL");
process.exit(pass ? 0 : 1);
