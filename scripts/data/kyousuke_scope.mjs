#!/usr/bin/env node
/**
 * Scout: find all Falcons matches in the last-3-month window via csapi.de,
 * so we can then pull scoreboards and extract kyousuke's per-map stats.
 * Window anchor: 2026-06-08 .. 2026-09-08 (matches date-descending).
 */
import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const API_BASE = "https://api.csapi.de";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const WINDOW_START = "2026-06-08"; // inclusive
const WINDOW_END = "2026-09-08";   // inclusive (today per env)
const TEAM = "Falcons";

const url = (o) => `${API_BASE}/matches/?offset=${o}`;

async function fetchPage(offset) {
  const res = await fetch(url(offset), { signal: AbortSignal.timeout(20_000) });
  if (!res.ok) throw new Error(`HTTP ${res.status} @ offset ${offset}`);
  return res.json();
}

const found = [];
let offset = 0;
let scanned = 0;
let hitOlderThanWindow = false;
while (!hitOlderThanWindow) {
  let page;
  try {
    page = await fetchPage(offset);
  } catch (e) {
    console.error(`stop @ offset ${offset}: ${e.message}`);
    break;
  }
  if (!Array.isArray(page) || page.length === 0) {
    console.error(`empty @ offset ${offset}, stop`);
    break;
  }
  for (const m of page) {
    if (!m.date) continue;
    if (m.date < WINDOW_START) { hitOlderThanWindow = true; break; } // dates descending
    scanned++;
    const inWindow = m.date <= WINDOW_END;
    if (inWindow && ((m.team1?.name === TEAM) || (m.team2?.name === TEAM))) {
      found.push({ id: m.id, date: m.date, event: m.event, best_of: m.best_of,
        vs: (m.team1.name === TEAM ? m.team2.name : m.team1.name),
        score1: m.team1.score, score2: m.team2.score });
    }
  }
  offset += page.length;
  await sleep(140); // be gentle
}

found.sort((a, b) => (a.date < b.date ? -1 : a.date > b.date ? 1 : 0));
const out = path.join(__dirname, "kyousuke_scope.json");
writeFileSync(out, JSON.stringify({ window: [WINDOW_START, WINDOW_END], team: TEAM,
  matchesScannedInWindow: scanned, matchesFound: found.length, matches: found }, null, 2));
console.log(`window ${WINDOW_START}..${WINDOW_END} team=${TEAM}`);
console.log(`Falcons matches in window: ${found.length}`);
for (const f of found) console.log(`  ${f.date}  id=${f.id}  bo${f.best_of}  vs ${f.vs}  ${f.score1}-${f.score2}  [${f.event}]`);
