#!/usr/bin/env node
// verify-live-prompt.mjs — t7 LIVE 20s 提示条静态核对（HLTV 浅色 + 中文 + 倒计时 + 按钮）。
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9342;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-live-prompt-ud";
const URL = "file:///D:/repo-by-iverins/cs-career-simulation/docs/screenshots/ovn-live-prompt-harness.html";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [
  `--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${UD}`, `--window-size=980,300`, URL,
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
const evaljs = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true })).result?.result?.value;

await send("Runtime.enable"); await send("Page.enable"); await sleep(400);

let pass = true;
const check = (label, ok) => { if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${label}`); };

const text = (await evaljs("document.body.innerText")) ?? "";
console.log("--- t7 LIVE 提示条内容 ---");
check("含『比赛日 LIVE』", text.includes("比赛日 LIVE"));
check("含赛事名", text.includes("CCT Contenders"));
check("含『小组赛 · BO1』", text.includes("小组赛") && text.includes("BO1"));
check("含倒计时秒数（20s 内）", /20|1[0-9]/.test(text) && text.includes("s"));
check("含『进入实时观战』", text.includes("进入实时观战"));
check("含『跳过』", text.includes("跳过"));

const css = (sel) => evaljs(`(() => { const el=document.querySelector(${JSON.stringify(sel)}); if(!el) return null; const c=getComputedStyle(el); return {bg:c.backgroundColor,color:c.color,borderLeft:c.borderLeftColor,radius:c.borderTopLeftRadius,font:c.fontFamily}; })()`);
const bar = await css(".live-prompt");
const badge = await css(".live-prompt__badge");
const count = await css(".live-prompt__countdown b");
console.log("--- HLTV 浅色计算样式 ---");
console.log("  .live-prompt:", JSON.stringify(bar));
console.log("  .live-prompt__badge:", JSON.stringify(badge));
console.log("  .live-prompt__countdown b:", JSON.stringify(count));
check("提示条白色背景", bar?.bg === "rgb(255, 255, 255)");
check("左侧琥珀描边 (warn)", bar?.borderLeft === "rgb(232, 147, 12)");
check("LIVE badge 红色 (loss)", badge?.color === "rgb(214, 69, 69)");
check("倒计时琥珀色 (warn)", count?.color === "rgb(232, 147, 12)");

await send("Page.captureScreenshot", { format: "png" });
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot?.result?.data) {
  const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-live-prompt.png";
  writeFileSync(out, Buffer.from(shot.result.data, "base64"));
  console.log("screenshot ->", out, statSync(out).size, "bytes");
}
ws.close(); proc.kill();
console.log(pass ? "\nRESULT: PASS — t7 LIVE 提示条渲染 HLTV 浅色 + 中文 + 倒计时 + 按钮" : "\nRESULT: FAIL");
process.exit(pass ? 0 : 1);
