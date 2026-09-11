#!/usr/bin/env node
// MCP 交互式桥接（文件输入版）：从一个 JSON 文件读取步骤序列，在一个 MCP server 会话内依次执行。
// 用法: node mcp-bridge-session.mjs <stepsFile.json> [outputDir]
// stepsFile: [{"t":"take_snapshot","a":{},"saveAs":"shot.png","delay":600}, ...]
import { spawn } from 'node:child_process';
import { writeFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const stepsFile = process.argv[2];
const outDir = process.argv[3] || null;
const steps = JSON.parse(readFileSync(stepsFile, 'utf8'));
if (outDir && !existsSync(outDir)) mkdirSync(outDir, { recursive: true });
const MCP_BIN = process.env.CHROME_DEVTOOLS_MCP_BIN || 'chrome-devtools-mcp';

const mcp = spawn('node', [MCP_BIN, '--browserUrl', 'http://127.0.0.1:9222', '--no-usage-statistics', '--experimentalVision'], {
  stdio: ['pipe', 'pipe', 'pipe'],
});
let buf = '';
let nextId = 1;
let stepIdx = 0;
let shotSeq = 0;

const timer = setTimeout(() => {
  console.error(JSON.stringify({ error: 'TIMEOUT at step ' + stepIdx, partial: buf.slice(0, 1000) }));
  mcp.kill();
  process.exit(1);
}, 180000);

function sendNext() {
  if (stepIdx >= steps.length) {
    clearTimeout(timer);
    mcp.kill();
    process.exit(0);
  }
  const step = steps[stepIdx];
  const id = nextId++;
  mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method: 'tools/call', params: { name: step.t, arguments: step.a ?? {} } }) + '\n');
}

mcp.stdout.on('data', (d) => {
  buf += d.toString();
  let idx;
  while ((idx = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line || !line.startsWith('{')) continue;
    try {
      const msg = JSON.parse(line);
      if (msg.id === undefined) continue;
      const step = steps[stepIdx];
      const content = msg.result?.content ?? [];
      const parts = [];
      for (const c of content) {
        if (c.type === 'image') {
          const b64 = c.data ?? '';
          if (b64 && outDir) {
            shotSeq++;
            const name = step.saveAs || `shot-${String(stepIdx).padStart(2, '0')}-${shotSeq}.png`;
            const p = join(outDir, name);
            const bytes = Buffer.from(b64, 'base64');
            writeFileSync(p, bytes);
            parts.push(`[image saved: ${p} ${bytes.length}B]`);
          } else {
            parts.push(`[image ${(c.data ?? '').length}chars]`);
          }
        } else if (c.type === 'text') {
          parts.push(c.text);
        } else {
          parts.push(`[${c.type}]`);
        }
      }
      const isErr = msg.result?.isError === true || msg.error;
      console.log(`[${stepIdx}] ${step.t} ${isErr ? 'ERROR' : 'OK'}: ${(parts.join('\n') || JSON.stringify(msg.result ?? msg.error)).slice(0, 4000)}`);
      stepIdx++;
      setTimeout(sendNext, step.delay ?? 600);
    } catch {}
  }
});
mcp.stderr.on('data', () => {});
mcp.on('error', (e) => {
  clearTimeout(timer);
  console.error(JSON.stringify({ error: e.message }));
  process.exit(1);
});
sendNext();
