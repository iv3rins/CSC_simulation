// 脚本共享工具（结构拆分收敛）：仓库根定位 / standings 读取与 roster 展平 /
// JSON 读取带显式报错。此前各脚本各自复制「path.resolve(dirname(fileURLToPath(
// import.meta.url)), '..')」与「两层 for 展平 roster」片段（3+ 处），
// standings 结构或目录布局变更时需同步改所有脚本——现收敛为单点。
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

/** 仓库根目录（本文件位于 <root>/scripts/lib/ 下，向上两级）。 */
export function repoRoot() {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
}

/** 读取并解析 JSON；失败时带文件路径显式退出（不静默返回空）。 */
export function readJson(relPath) {
  const abs = path.join(repoRoot(), relPath);
  try {
    return JSON.parse(readFileSync(abs, "utf8"));
  } catch (e) {
    console.error(`读取失败：${abs}\n${e.message}`);
    process.exit(2);
  }
}

/** 读取最新一期 standings（rankings[] 含 roster[]）。 */
export function readStandings(relPath = "assets/standings_global_2026_01_05.json") {
  const standings = readJson(relPath);
  if (!standings || !Array.isArray(standings.rankings)) {
    console.error(`standings 结构非法（缺 rankings[]）：${relPath}`);
    process.exit(2);
  }
  return standings;
}

/** 展平全部 roster 名字（保留重复，供去重/覆盖统计）。 */
export function flattenRoster(standings) {
  const names = [];
  for (const k of Object.keys(standings.rankings)) {
    for (const n of standings.rankings[k].roster ?? []) names.push(n);
  }
  return names;
}
