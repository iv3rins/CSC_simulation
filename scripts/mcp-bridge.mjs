#!/usr/bin/env node
// MCP 桥接客户端 v3：支持把 image 类型结果保存为文件（data 字段是 base64）。
// 用法: node mcp-bridge.mjs <toolName> '<jsonArgs>' [outputFile]
// 屏蔽 chrome-devtools-mcp 的 stderr 噪音（版本提示等）。
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const tool = process.argv[2];
const args = process.argv[3] ? JSON.parse(process.argv[3]) : {};
const outFile = process.argv[4] || null;
const MCP_BIN = process.env.CHROME_DEVTOOLS_MCP_BIN || 'chrome-devtools-mcp';

const mcp = spawn('node', [MCP_BIN, '--browserUrl', 'http://127.0.0.1:9222', '--no-usage-statistics', '--experimentalVision'], {
  stdio: ['pipe', 'pipe', 'pipe'],
});
let buf = '';
const timer = setTimeout(() => {
  console.error(JSON.stringify({ error: 'TIMEOUT', partial: buf.slice(0, 800) }));
  mcp.kill();
  process.exit(1);
}, 90000);

mcp.stdout.on('data', (d) => {
  buf += d.toString();
  let idx;
  while ((idx = buf.indexOf('\n')) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line || !line.startsWith('{')) continue;
    try {
      const msg = JSON.parse(line);
      if (msg.id === 1) {
        clearTimeout(timer);
        const content = msg.result?.content ?? [];
        const parts = [];
        for (const c of content) {
          if (c.type === 'image') {
            const b64 = c.data ?? '';
            if (b64 && outFile) {
              const bytes = Buffer.from(b64, 'base64');
              writeFileSync(outFile, bytes);
              parts.push(`[image saved: ${outFile} (${bytes.length} bytes)]`);
            } else if (b64) {
              parts.push(`[image data:${b64.length}chars]`);
            } else {
              parts.push(`[image no data, keys: ${Object.keys(c).join(',')}]`);
            }
          } else if (c.type === 'text') {
            parts.push(c.text);
          } else {
            parts.push(`[${c.type}]`);
          }
        }
        console.log(parts.join('\n') || JSON.stringify(msg.result ?? msg.error));
        mcp.kill();
        process.exit(msg.error ? 1 : 0);
      }
    } catch {}
  }
});
mcp.stderr.on('data', () => { /* 屏蔽版本提示等噪音 */ });
mcp.on('error', (e) => {
  clearTimeout(timer);
  console.error(JSON.stringify({ error: e.message }));
  process.exit(1);
});
mcp.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: tool, arguments: args } }) + '\n');
