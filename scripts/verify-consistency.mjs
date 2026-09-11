#!/usr/bin/env node
// verify-consistency.mjs — confirm home-page consistency copy renders in Chinese.
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9335;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-consistency-ud";
const URL = "file:///D:/repo-by-iverins/cs-career-simulation/docs/screenshots/ovn-consistency-harness.html";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [`--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`, `--user-data-dir=${UD}`, `--window-size=700,620`, URL], { stdio: "ignore" });
await sleep(2500);

let target;
try {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  target = list.find((t) => t.type === "page" && t.url.includes("ovn-consistency")) || list.find((t) => t.type === "page");
  if (!target) throw new Error("no page");
} catch (e) { console.error("CDP fail:", e.message); proc.kill(); process.exit(2); }

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let id = 0; const pending = new Map();
ws.onmessage = (ev) => { const m = JSON.parse(ev.data); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
await send("Runtime.enable");
await sleep(400);

const text = await send("Runtime.evaluate", { expression: "document.body.innerText", returnByValue: true });
const body = text.result?.result?.value ?? "";
const must = ["职业身份", "职业地位", "表现", "声望", "领导力", "影响力", "职业生涯亮点", "3 段记忆", "加入队伍", "首战 Major", "Major 冠军", "LIVE · 即将进行的对局", "3 天倒计时", "本月已完赛 12 场世界赛事", "我的完整赛程", "世界动态", "我的比赛", "下一场顶级赛事", "天", "最新战报", "回放", "三档可看 · 已完赛场次", "13 场 · 本队 2 场可回放", "观看回放", "进入赛事"];
let pass = true;
for (const s of must) { const ok = body.includes(s); if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${s}`); }
const rawEnums = ["PLAYER IDENTITY", "PERFORMANCE", "REPUTATION", "LEADERSHIP", "INFLUENCE", "CAREER HIGHLIGHTS", "NEXT TOP-TIER EVENT", "NEXT TEAM EVENT", "DAY", "DAYS", "BREAKING NEWS", "SEASON CALENDAR", "NEXT EVENT"];
for (const s of rawEnums) { const bad = body.includes(s); if (bad) pass = false; console.log(`  [${bad ? "FAIL" : "PASS"}] no '${s}'`); }

await send("Page.enable"); await sleep(300);
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot?.result?.data) { const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-consistency.png"; writeFileSync(out, Buffer.from(shot.result.data, "base64")); console.log("screenshot ->", out, statSync(out).size, "bytes"); }
ws.close(); proc.kill();
console.log(pass ? "RESULT: PASS — home consistency copy is Chinese" : "RESULT: FAIL");
process.exit(pass ? 0 : 1);
