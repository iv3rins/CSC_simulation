#!/usr/bin/env node
/**
 * vision-bridge.mjs — DeepSeek V4 Flash Vision Exp 视觉桥（替代 Ox 截图 MCP 的视觉核验层）
 *
 * 背景：主模型 deepseek-v4-flash-vip 无读图能力；Ox（chrome-devtools-mcp 截图工具）已弃用。
 * 本脚本直连 opencode-go 网关（https://opencode.ai/zen/go/v1）的 deepseek-v4-flash-vision-exp，
 * 对截图做视觉核验（布局/空白/重叠/溢出/配色），对齐 docs/QA-AUTOTEST-prompt.md §5 协议。
 *
 * 用法：
 *   node scripts/vision-bridge.mjs <imagePath>                     # 单图分析（默认核验提示词）
 *   node scripts/vision-bridge.mjs <imagePath> '<自定义提示词>'
 *   node scripts/vision-bridge.mjs --dir <目录> [--out <汇总json>] # 批量分析目录下所有 png
 *
 * 输出：单图打印模型 JSON；--dir 模式打印汇总表并可选写汇总文件。
 * 依赖：node >= 18（内置 fetch）；auth.json 中 opencode-go 通道的 key。
 */
import { readFileSync, readdirSync, existsSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const authRoot = process.env.XDG_DATA_HOME
  || join(process.env.HOME || process.env.USERPROFILE || ".", ".local", "share");
const AUTH_FILE = process.env.OPENCODE_AUTH_FILE
  || join(authRoot, "opencode", "auth.json");
const API = "https://opencode.ai/zen/go/v1/chat/completions";
const MODEL = "deepseek-v4-flash-vision-exp";
const UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

// 视觉核验提示词（对齐 QA-AUTOTEST-prompt.md §5 四项 + 结构化输出）
const DEFAULT_PROMPT = `你是 CSC（CS2 职业选手生涯模拟器）的视觉验收 QA。请核验这张网页截图，输出 JSON（不要 markdown 代码块）：

{
  "page": "该页面名称/功能判断",
  "layout_ok": true/false,
  "layout_note": "布局是否符合：主体内容居中、占据合理宽度（桌面 max 1240px）、未被压到一角、无元素整体缩挤在角落",
  "blank_ok": true/false,
  "blank_note": "是否有大面积空白（内容区应有数据却空白=缺陷）；若有，指出空白区域位置",
  "overlap_ok": true/false,
  "overlap_note": "是否有元素重叠、遮挡、文字溢出/越界（长队名/选手名是否越界）",
  "color_ok": true/false,
  "color_note": "配色是否符合 HLTV 视觉（浅灰画布、白卡片、HLTV 蓝 #2d7dd2、涨红跌绿）",
  "issues": ["问题1", "问题2"],
  "verdict": "PASS / WARN / FAIL",
  "summary": "一句话总结"
}`;

function getKey() {
  if (!existsSync(AUTH_FILE)) throw new Error("auth.json 不存在: " + AUTH_FILE);
  const auth = JSON.parse(readFileSync(AUTH_FILE, "utf8"));
  const key = auth["opencode-go"]?.key;
  if (!key) throw new Error("auth.json 中无 opencode-go key");
  return key;
}

async function analyze(imgPath, prompt, { maxTokens = 4000, retries = 2 } = {}) {
  const b64 = readFileSync(imgPath).toString("base64");
  const body = JSON.stringify({
    model: MODEL,
    messages: [{
      role: "user",
      content: [
        { type: "text", text: prompt },
        { type: "image_url", image_url: { url: `data:image/png;base64,${b64}` } },
      ],
    }],
    max_tokens: maxTokens,
  });
  const call = async () => {
    const res = await fetch(API, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: "Bearer " + getKey(),
        "User-Agent": UA,
      },
      body,
    });
    if (!res.ok) {
      const txt = await res.text().catch(() => "");
      throw new Error(`HTTP ${res.status}: ${txt.slice(0, 500)}`);
    }
    return res.json();
  };
  let data;
  let lastErr;
  for (let i = 0; i <= retries; i++) {
    try {
      data = await call();
      const c = data.choices?.[0]?.message?.content;
      if (c && c.trim()) break; // 拿到非空 content
      lastErr = new Error(`空 content（finish_reason=${data.choices?.[0]?.finish_reason}）`);
      if (i < retries) { await new Promise((r) => setTimeout(r, 800 * (i + 1))); }
    } catch (e) {
      lastErr = e;
      if (i < retries) await new Promise((r) => setTimeout(r, 800 * (i + 1)));
    }
  }
  if (!data) throw lastErr;
  const content = data.choices?.[0]?.message?.content ?? "";
  // 提取 JSON（模型可能带 ```json 围栏）
  const fenced = content.match(/```(?:json)?\s*([\s\S]*?)```/);
  const raw = fenced ? fenced[1] : content;
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start >= 0 && end > start) {
    try { return JSON.parse(raw.slice(start, end + 1)); } catch {}
  }
  return { verdict: "PARSE_FAIL", raw: content.slice(0, 500), summary: "模型输出非结构化 JSON，需人工查看 raw" };
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length === 0) {
    console.error("用法: node scripts/vision-bridge.mjs <img> ['提示词'] | --dir <目录> [--out <json>]");
    process.exit(1);
  }
  if (args[0] === "--dir") {
    const dir = resolve(args[1]);
    const outIdx = args.indexOf("--out");
    const outFile = outIdx >= 0 ? resolve(args[outIdx + 1]) : null;
    if (!existsSync(dir)) { console.error("目录不存在:", dir); process.exit(1); }
    const imgs = readdirSync(dir).filter((f) => /\.(png|jpe?g)$/i.test(f)).sort();
    console.error(`批量核验 ${imgs.length} 张截图于 ${dir} ...`);
    const results = [];
    for (const f of imgs) {
      const p = join(dir, f);
      try {
        const r = await analyze(p, DEFAULT_PROMPT);
        results.push({ file: f, ...r });
        console.log(`  [${r.verdict ?? "?"}] ${f}`);
      } catch (e) {
        results.push({ file: f, verdict: "ERROR", summary: e.message });
        console.error(`  [ERROR] ${f}: ${e.message}`);
      }
    }
    const pass = results.filter((r) => r.verdict === "PASS").length;
    const warn = results.filter((r) => r.verdict === "WARN").length;
    const fail = results.filter((r) => r.verdict === "FAIL").length;
    console.log(`\n汇总: PASS=${pass} WARN=${warn} FAIL=${fail} ERROR=${results.length - pass - warn - fail} / 共${results.length}`);
    if (outFile) { writeFileSync(outFile, JSON.stringify(results, null, 2)); console.error("已写入:", outFile); }
    return;
  }
  const img = resolve(args[0]);
  const prompt = args[1] || DEFAULT_PROMPT;
  if (!existsSync(img)) { console.error("文件不存在:", img); process.exit(1); }
  const r = await analyze(img, prompt).catch((e) => ({ verdict: "ERROR", summary: e.message }));
  console.log(JSON.stringify(r, null, 2));
}

main();
