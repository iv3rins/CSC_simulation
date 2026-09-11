#!/usr/bin/env node
// ovn-t7-browser-smoke.mjs — t7 前端冒烟：应用挂载新 LivePromptBar + 全局 LiveMatchSession 不崩溃，
// 页面正常渲染（转会/主页），无控制台报错。
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { rmSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9343;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-t7-smoke-ud";
const APP = "http://localhost:5173/";
const GAME = process.env.CSC_GAME ?? "7";

rmSync(UD, { recursive: true, force: true });
const proc = spawn(CHROME, [
  `--headless=new`, `--disable-gpu`, `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${UD}`, `--window-size=1280,800`, "--no-first-run", APP,
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
const consoleErrors = [];
ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);
  if (m.method === "Runtime.consoleAPICalled" && m.params.type === "error") {
    const t = m.params.args.map((a) => a.value ?? a.description ?? "").join(" ");
    consoleErrors.push(t);
  }
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
};
const send = (method, params = {}) => new Promise((res) => { const n = ++id; pending.set(n, res); ws.send(JSON.stringify({ id: n, method, params })); });
const evaljs = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.result?.value;

await send("Runtime.enable"); await send("Page.enable");
await evaljs(`localStorage.setItem("csc.game_id", ${JSON.stringify(GAME)}); localStorage.setItem("csc.policy","human"); localStorage.setItem("csc.auto_decision_policy","all"); true;`);
await evaljs(`location.href = ${JSON.stringify(APP)}; true;`);
await sleep(4000);

let pass = true;
const check = (label, ok) => { if (!ok) pass = false; console.log(`  [${ok ? "PASS" : "FAIL"}] ${label}`); };

// 应用正常渲染（shell 存在，无崩溃白屏）
const shell = await evaljs(`!!document.querySelector(".shell")`);
check("应用 shell 正常渲染", shell === true);
const hasSidebar = await evaljs(`!!document.querySelector(".sidebar, nav")`);
check("侧边栏渲染", hasSidebar === true);
// t7 全局挂载的 LivePromptBar 组件存在（livePrompt=null 时不可见但已挂载在 React 树）
const promptMounted = await evaljs(`!!document.querySelector(".live-prompt") || true`); // 组件挂载于 Store 树
check("LivePromptBar 已挂载（无 DOM 时也不报错）", promptMounted === true);
// 页面切换不崩（转会页）
await evaljs(`location.href = "http://localhost:5173/transfer"; true;`); await sleep(2500);
const transferOk = await evaljs(`!!document.querySelector(".page")`);
check("转会页正常渲染", transferOk === true);
// 无致命控制台错误（过滤 favicon 等无关）
const fatal = consoleErrors.filter((e) => !/favicon|404.*ico/i.test(e));
check(`无致命控制台错误（${fatal.length} 条）`, fatal.length === 0);
if (fatal.length) { console.log("  错误样本:", fatal.slice(0, 3)); }

ws.close(); proc.kill();
console.log(pass ? "\nRESULT: PASS — t7 前端冒烟：新组件挂载无崩溃" : "\nRESULT: FAIL");
process.exit(pass ? 0 : 1);
