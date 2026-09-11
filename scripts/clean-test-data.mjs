#!/usr/bin/env node
// Remove generated build/runtime data before a clean E2E pass.
import { existsSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");

if (process.platform === "win32") {
  try {
    execFileSync("taskkill", ["/F", "/IM", "csc-server.exe"], { stdio: "ignore" });
  } catch {
    // No server process is the normal clean-start case.
  }
}

const targets = [
  "backend/target",
  "assets/games",
];

for (const relative of targets) {
  const path = resolve(root, relative);
  if (existsSync(path)) {
    try {
      rmSync(path, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    } catch (error) {
      throw new Error(`无法清理 ${relative}；请确认没有构建或服务进程仍在运行：${error.message}`);
    }
  }
  console.log(`[clean] ${relative}`);
}
