#!/usr/bin/env node
/**
 * 扩充 standings 资产：把 40 支真实开局队伍扩充为 128 支（可配置）。
 *
 * 数据源：
 *  - assets/standings_global_2026_01_05.json   真实 TOP40 + 真实阵容
 *  - assets/teams-logo/teams.json              HLTV 队伍目录（名称/rank/队标）
 *  - assets/player_ratings.json                真实选手 Rating 先验（686 名）
 *
 * 策略：
 *  1. 保留原 TOP40 不动（排名/积分/阵容完全一致，确定性不变）；
 *  2. 从 HLTV 队伍目录按 rank 顺序挑未出现的真实队名，直到目标规模；
 *  3. 新队积分沿 TOP40 尾部指数衰减（floor 250），排名连续 41..N；
 *  4. 阵容从 player_ratings 中未被占用的真实昵称按 rating 降序切片填充
 *     （第 41 名拿最强自由选手，越往后越弱；不足 5 人时用确定性占位名）。
 *
 * 用法：
 *   node scripts/expand_standings.mjs [--teams 128] [--out assets/standings_global_2026_01_05.json]
 * 原 40 队文件会备份到 assets/backup/standings_global_2026_01_05.40teams.json。
 */

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : fallback;
};
const targetTeams = Number(opt("--teams", "128"));
const outPath = resolve(root, opt("--out", join("assets", "standings_global_2026_01_05.json")));
const inPath = resolve(root, "assets/standings_global_2026_01_05.json");
const teamsPath = resolve(root, "assets/teams-logo/teams.json");
const ratingsPath = resolve(root, "assets/player_ratings.json");
const backupPath = resolve(root, "assets/backup/standings_global_2026_01_05.40teams.json");

if (!Number.isInteger(targetTeams) || targetTeams < 40 || targetTeams > 500) {
  console.error("--teams 必须是 40..500 的整数");
  process.exit(1);
}

const norm = (s) => String(s).toLowerCase().replace(/&/g, " and ").replace(/[^a-z0-9]/g, "");

// 始终从 40 队基线扩编（脚本幂等：重复运行不会在 128 队结果上继续叠加）。
const baseSource = existsSync(backupPath) ? backupPath : inPath;
const original = JSON.parse(readFileSync(baseSource, "utf8"));
mkdirSync(dirname(backupPath), { recursive: true });
if (!existsSync(backupPath) && baseSource !== backupPath) {
  writeFileSync(backupPath, JSON.stringify(original, null, 2));
  console.log(`已备份原 40 队文件 → ${backupPath}`);
}

const base = original.rankings;
if (base.length > targetTeams) {
  console.error(`目标 ${targetTeams} 小于当前 ${base.length} 队，拒绝缩编`);
  process.exit(1);
}
if (base.length === targetTeams) {
  console.log(`已是 ${targetTeams} 队，无需扩充`);
  process.exit(0);
}

const teamCatalog = JSON.parse(readFileSync(teamsPath, "utf8"));
// 队名「品牌核」：Team Falcons / Falcons / FaZe Clan / FaZe 视为同一组织，
// 避免把 TOP40 的赞助商全称版当成第二支新队重复加进世界。
const coreName = (s) =>
  norm(s)
    .replace(/^(team|org|the)(?=[a-z0-9])/, "")
    .replace(/(esports|gaming|clan|team|org|cs)$/, "")
    .replace(/^the/, "");
const usedTeamCore = new Set(base.map((t) => coreName(t.teamName)));
const candidates = teamCatalog
  .filter((e) => !usedTeamCore.has(coreName(e.name)))
  .sort((a, b) => a.rank - b.rank);

if (candidates.length + base.length < targetTeams) {
  console.error(`队伍目录只有 ${candidates.length} 支可用候选，无法达到 ${targetTeams} 队`);
  process.exit(1);
}

// 真实昵称池：排除 TOP40 已占用名字；rating 降序。
const ratings = JSON.parse(readFileSync(ratingsPath, "utf8")).players ?? [];
const usedPlayer = new Set(base.flatMap((t) => t.roster).map((n) => n.toLowerCase()));
const freePlayers = ratings
  .map((p) => p.player)
  .filter((name) => !usedPlayer.has(String(name).toLowerCase()))
  .sort((a, b) => {
    const ra = ratings.find((p) => p.player === a)?.rating ?? 0;
    const rb = ratings.find((p) => p.player === b)?.rating ?? 0;
    return rb - ra;
  });

const fallbackName = (teamName, slot, used) => {
  const stem = norm(teamName).slice(0, 12) || "player";
  for (let i = 1; i < 1000; i++) {
    const name = `${stem}${slot + 1}_${i}`;
    if (!used.has(name.toLowerCase())) return name;
  }
  throw new Error("fallback 昵称耗尽");
};

const expanded = base.map((t) => ({ ...t }));
const usedNames = new Set(base.flatMap((t) => t.roster).map((n) => n.toLowerCase()));
let playerCursor = 0;
const lastPoints = base.at(-1)?.points ?? 386;
const lastRank = base.at(-1)?.ranking ?? 40;

for (let i = 0; expanded.length < targetTeams && i < candidates.length; i++) {
  const team = candidates[i];
  const ranking = lastRank + 1 + i;
  const decay = 0.96 ** (ranking - lastRank);
  const points = Math.max(250, Math.round(lastPoints * decay));
  const roster = [];
  for (let slot = 0; slot < 5; slot++) {
    let name = null;
    while (playerCursor < freePlayers.length) {
      const candidate = freePlayers[playerCursor++];
      if (!usedNames.has(candidate.toLowerCase())) {
        name = candidate;
        break;
      }
    }
    if (!name) name = fallbackName(team.name, slot, usedNames);
    usedNames.add(name.toLowerCase());
    roster.push(name);
  }
  expanded.push({
    ranking,
    points,
    teamName: team.name,
    roster,
  });
}

if (expanded.length !== targetTeams) {
  console.error(`扩充后 ${expanded.length} 队，未达到 ${targetTeams} 队`);
  process.exit(1);
}

const out = { ...original, rankings: expanded };
writeFileSync(outPath, JSON.stringify(out, null, 2));
console.log(`standings 已扩充：${base.length} → ${expanded.length} 队`);
console.log(`  新增 ${expanded.length - base.length} 队（首：${expanded[40]?.teamName}，末：${expanded.at(-1)?.teamName}）`);
console.log(`  真实自由选手池 ${freePlayers.length} 人，已使用 ${playerCursor} 人`);
