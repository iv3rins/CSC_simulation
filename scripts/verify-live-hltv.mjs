#!/usr/bin/env node
// verify-live-hltv.mjs — programmatic HLTV-light verification of the LIVE overlay CSS.
// Opens the harness HTML, reads computed styles of key .live-session_* nodes via CDP,
// and asserts they match the HLTV 2026 light palette.
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { writeFileSync, rmSync, statSync } from "node:fs";

const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
const PORT = 9333;
const UD = "D:\\repo-by-iverins\\cs-career-simulation\\.chrome\\ovn-verify-ud";
const URL = "file:///D:/repo-by-iverins/cs-career-simulation/docs/screenshots/ovn-live-hltv-harness.html";

function rmrf(p) { try { rmSync(p, { recursive: true, force: true }); } catch {} }
rmrf(UD);

const proc = spawn(CHROME, [
  `--headless=new`, `--disable-gpu`, `--hide-scrollbars`,
  `--remote-debugging-port=${PORT}`, `--user-data-dir=${UD}`,
  `--window-size=860,780`, URL,
], { stdio: "ignore" });

const sleepMs = (ms) => sleep(ms);
await sleepMs(2500);

let target;
try {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  target = list.find((t) => t.type === "page" && t.url.includes("ovn-live-hltv-harness")) || list.find((t) => t.type === "page");
  if (!target) throw new Error("no page target");
} catch (e) {
  console.error("CDP list failed:", e.message);
  proc.kill();
  process.exit(2);
}

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let msgId = 0;
const pending = new Map();
ws.onmessage = (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
};
function send(method, params = {}) {
  return new Promise((res) => { const id = ++msgId; pending.set(id, res); ws.send(JSON.stringify({ id, method, params })); });
}
await send("Runtime.enable");
await sleepMs(1200); // let fonts/layout settle

const cssSel = (sel) => `(() => { const el = document.querySelector(${JSON.stringify(sel)}); if (!el) return null; const c = getComputedStyle(el); return { bg: c.backgroundColor, color: c.color, border: c.borderTopColor, borderW: c.borderTopWidth, radius: c.borderTopLeftRadius, font: c.fontFamily }; })()`;

const checks = {
  ".live-session": { bg: "rgb(255, 255, 255)", color: "rgb(26, 39, 51)" },
  ".live-session__head": { bg: "rgb(244, 247, 250)" },
  ".live-session__badge": { color: "rgb(214, 69, 69)" },
  ".live-session__scoreboard b": { bg: "rgb(247, 249, 251)", color: "rgb(26, 39, 51)" },
  ".live-session__decision": {},
  ".live-session__round--a": {},
  ".live-session__history--a": { color: "rgb(46, 158, 91)" },
};

const results = {};
let pass = true;
for (const [sel, expect] of Object.entries(checks)) {
  const r = await send("Runtime.evaluate", { expression: cssSel(sel), returnByValue: true });
  const val = r.result?.result?.value;
  results[sel] = val;
  if (!val) { console.log(`  [MISS] ${sel} -> no element`); pass = false; continue; }
  const okBg = !expect.bg || val.bg === expect.bg;
  const okColor = !expect.color || val.color === expect.color;
  const ok = okBg && okColor;
  if (!ok) pass = false;
  console.log(`  [${ok ? "PASS" : "FAIL"}] ${sel}`);
  console.log(`         bg=${val.bg} (want ${expect.bg ?? "-"})  color=${val.color} (want ${expect.color ?? "-"})`);
}

// Screenshot via Page.captureScreenshot (base64)
await send("Page.enable");
await sleepMs(500);
const shot = await send("Page.captureScreenshot", { format: "png" });
const data = shot?.result?.data;
if (data) {
  const out = "D:\\repo-by-iverins\\cs-career-simulation\\docs\\screenshots\\ovn-live-hltv.png";
  writeFileSync(out, Buffer.from(data, "base64"));
  console.log("screenshot ->", out, statSync(out).size, "bytes");
} else {
  console.log("screenshot: no data", JSON.stringify(shot).slice(0, 200));
}
ws.close();
proc.kill();
console.log(pass ? "RESULT: PASS — LIVE overlay is HLTV light" : "RESULT: FAIL");
process.exit(pass ? 0 : 1);
