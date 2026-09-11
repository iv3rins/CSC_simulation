#!/usr/bin/env node
/**
 * Pull scoreboards for each Falcons match found in kyousuke_scope.json and
 * extract kyousuke's per-map stat lines, then aggregate over the window.
 */
import { writeFileSync, readFileSync, existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const API_BASE = "https://api.csapi.de";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const SCOPE = JSON.parse(readFileSync(path.join(__dirname, "kyousuke_scope.json"), "utf8"));

const kyName = (s) => String(s).toLowerCase().includes("kyousuke");

async function fetchStats(matchId) {
  const res = await fetch(`${API_BASE}/matches/${matchId}/stats`, { signal: AbortSignal.timeout(25_000) });
  if (!res.ok) throw new Error(`HTTP ${res.status} for match ${matchId}`);
  return res.json();
}

const perMap = [];   // kyousuke line per map
const matchesPlayed = []; // matches where kyousuke appears

for (const f of SCOPE.matches) {
  let maps;
  try { maps = await fetchStats(f.id); }
  catch (e) { console.error(`  match ${f.id} (${f.date}) fetch failed: ${e.message}`); continue; }
  if (!Array.isArray(maps)) { console.error(`  match ${f.id}: unexpected shape`); continue; }
  let inMatch = false;
  for (const m of maps) {
    const mapName = m?.name ?? "All";
    // locate the Falcons roster among team1/team2
    let team = null;
    for (const t of [m?.team1, m?.team2]) {
      if (t?.players && t.players.some((p) => kyName(p.name))) { team = t; break; }
    }
    if (!team) continue;
    const row = team.players.find((p) => kyName(p.name));
    if (!row) continue;
    inMatch = true;
    perMap.push({
      matchId: f.id, date: f.date, event: f.event,
      vs: f.vs, map: mapName,
      k: row.k ?? null, d: row.d ?? null,
      adr: row.adr ?? null, kast: row.kast ?? null, rating: row.rating ?? null,
    });
  }
  if (inMatch) {
    matchesPlayed.push({ id: f.id, date: f.date, event: f.event, vs: f.vs, best_of: f.best_of });
    console.log(`  ${f.date} id=${f.id} vs ${f.vs}: kyousuke played`);
  } else {
    console.log(`  ${f.date} id=${f.id} vs ${f.vs}: kyousuke NOT in scoreboard (sat out?)`);
  }
  await sleep(160);
}

// window aggregate from the "All" (map-level) rows only
const allRows = perMap.filter((r) => r.map === "All");
const n = (v) => (typeof v === "number" ? v : parseFloat(v));
const agg = (rows, fn) => {
  const vals = rows.map(fn).filter((v) => v != null && !Number.isNaN(v));
  return vals.length ? vals.reduce((a, b) => a + b, 0) : null;
};
const avg = (rows, fn) => {
  const vals = rows.map(fn).filter((v) => v != null && !Number.isNaN(v));
  return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null;
};

const summary = {
  window: SCOPE.window, team: SCOPE.team,
  falconsMatchesInWindow: SCOPE.matches.length,
  matchesKyousukePlayed: matchesPlayed.length,
  mapsRecorded: perMap.length,
  matches: matchesPlayed,
  perMapAll: allRows,
  aggregateAllRows: {
    maps: allRows.length,
    totalK: agg(allRows, (r) => r.k),
    totalD: agg(allRows, (r) => r.d),
    avgADR: avg(allRows, (r) => r.adr),
    avgKAST: avg(allRows, (r) => r.kast),
    avgRating: avg(allRows, (r) => r.rating),
  },
};

const outPath = path.join(__dirname, "kyousuke_recent.json");
writeFileSync(outPath, JSON.stringify(summary, null, 2));
console.log("\n=== SUMMARY ===");
console.log(`Falcons matches in window: ${SCOPE.matches.length}; kyousuke played ${matchesPlayed.length}`);
console.log(`Maps recorded: ${perMap.length}; "All" aggregate rows: ${allRows.length}`);
const a = summary.aggregateAllRows;
console.log(`Aggregate (All rows): K ${a.totalK} / D ${a.totalD}  ADR ${a.avgADR?.toFixed(1)}  KAST ${a.avgKAST?.toFixed(1)}%  Rating ${a.avgRating?.toFixed(3)}`);
console.log(`saved -> ${outPath}`);
