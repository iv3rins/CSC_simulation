# CSC 展示层设计契约（原 Frontend Design Contract）

> **仓库状态（2026-09）**：`frontend/` 已从仓库移除，前后端彻底分离。
> 本文档是展示层重建时的产品/交互/视觉设计依据，与 docs/analysis/（协议契约）配套使用。

## 1. Product Positioning

CSC is a first-person Counter-Strike professional career management game. The player is **one athlete**, not a club owner and not a data analyst. Every screen must answer at least one of these questions:

- What is happening to my career now?
- What decision can I make at this moment?
- What match, contract, relationship, or ranking consequence will follow?

The core experience is a career that **advances on its own** and pauses at key moments for the player. The player does not click to push time forward every cycle; time flows, and the player steps in when the world requires a decision, a match decision, a transfer choice, or a career call.

HLTV is the information-density and visual reference. Use its compact tables, rating semantics, team logos, map and event terminology, light canvas, white cards, thin borders, and blue accent. Add a game loop through objectives, resource states, actionable choices, progression, match feedback, and career consequences.

## 2. Real CS Roster Model (no bench, no six-man rotation)

Real Counter-Strike rosters are **five players, no active substitutes on the roster**. There is no bench, no 6th man, no rotation of a sixth player between maps. The player's roster status can only be one of these:

1. **Active starter (首发)** — the permanent five. The player is locked into the starting five for every match of the season.
2. **On loan / stand-in for 1–2 events (短期救火/临时顶替)** — when a team has an injured or benched player, they may sign a **short-term stand-in** for a specific event (a single tournament, typically one to two events). When the event ends, the stand-in contract ends, and the player either returns to their original team or becomes a free agent. This is **not** being "on the bench" of the strong team.
3. **Free agent (自由人/待业)** — no contract, searching for a team.
4. **Retired (已退役)** — career ended.

Explicitly forbidden in the design: a permanent six-man roster, map-by-map rotation of a sixth player, or "sitting on the bench of a big team waiting for a starting slot." If a big team wants the player long-term, they must **sign them as one of the starting five** (a real transfer), not place them on a reserve list.

### Why entering a big team must be a real move, not a bench slot

In CS there is no substitute position to "wait in." Opportunities to enter a top team come as one of these concrete moves:

- **Direct signing as starter (正式转会为首发)** — a team opens a starting slot and buys out or signs the player. The player immediately plays every match.
- **Short-term stand-in (1–2 events)** — the player fills in for an injured/absent starter for one event, gains exposure and a taste of the level, then returns to their origin or becomes a free agent. The resulting reputation bump may convert into a real starting offer later.
- **Free agency signing (自由市场签约)** — contract expired or bought out, then signed as starter.
- **Trial / tryout (试训)** — a chance to prove yourself; if the team likes you, they offer a starting contract.

The player must never be modeled as "a reserve on a top team." The top team either wants them as part of the starting five or they do not want them at all.

### Stand-in presentation rules

- A stand-in is shown as a clearly labeled **short-term engagement**: `临时顶替 · {EventName} · 1 场赛事`.
- It does **not** change the player's long-term team contract. On the match page and home page, the player's team remains their real club; the stand-in engagement is a separate, time-bounded label.
- When the event ends, the player returns to their origin team or becomes a free agent, with a reputation/visibility consequence derived from the event performance.
- Never render the player as "belonging to" the strong team beyond the event window.

## 3. Time Model and Pause Points

Time **advances automatically**. The player does not drive every step. The app exposes explicit simulation controls:

- `播放 / 暂停` (play / pause)
- `加速` (speed up — jump ahead while auto-processing)
- `推进到下一个重要事件` (advance to the next important event)

The system **auto-pauses** when the player must act:

- A match is starting (enter the LIVE match center).
- A pre-match roster / map veto / game plan needs a decision.
- A transfer, buyout, tryout, or contract offer arrives.
- An injury reaches match-affecting severity.
- A teammate, coach, or club relationship reaches a major turning point.
- A contract is nearing expiry.
- The club faces demotion, disbandment, or funding crisis.
- Qualification to a major event is earned or lost.
- Annual ranking / honours / TOP20 settlement occurs.
- Retirement conditions are met or the career endpoint triggers.

Routine training, recovery, low-tier match results, and general news must **not** pause the player.

## 4. Information Architecture

The application shell has three persistent levels:

- Sidebar: player identity and primary career destinations.
- Top bar: simulation date, play/pause/advance controls, and global status.
- Main content: the selected career workspace.

Primary destinations stay stable and are grouped by task (see existing grouping: career center / competition / club / world / profile). Do not create overlapping navigation aliases.

Recommended primary destinations, kept to a small set:

- **主页 (Home / Career Hub)** — current status and the next event.
- **比赛 (Matches)** — LIVE center, schedule, and history/replays.
- **职业 (Career)** — training, contract, transfers, relationships, objectives.
- **世界 (World)** — rankings, TOP20, NPC roster changes, news.
- **生涯 (Profile / Archive)** — data, honours, season summaries, career ending.

The Career Hub order is fixed by player priority:

1. Player identity, current team, rating, money, and contract (with a clear stand-in label if active).
2. Next event or immediate career task (with the play/pause control prominent).
3. Morale, fitness, injury, and team cohesion.
4. Actions available at this moment.
5. Season objective and progress.
6. Recent match feedback and career events.

## 5. Component Contract

### `CareerHubPage`

Composition only. It may select current player/team/matches and arrange sections. Extract a section when it exceeds roughly 80 lines or is reused.

### `Meter`

Receives a label and a normalized 0-100 value. Clamp visual width. Colors communicate category, not quality by themselves. Always show the numeric value.

### Match Presentation

Use Counter-Strike terms: BO1/BO3, map, T side, CT side, K-D, ADR, KAST, Swing, Rating 2.0. Rating below 1.00 is red; rating at or above 1.00 is green. Positive Swing is green and negative Swing is red. Do not infer a winner from K-D alone.

A detailed match table must keep this column order:

`Player | K-D | Swing | ADR | KAST | Rating 2.0`

Team blocks are separate and players are sorted by Rating descending. Nicknames keep the form `Given 'Nickname' Family` when full names are available.

### Live vs Result vs Replay

These three states are separate and must not be visually merged:

- **LIVE** — the match is in progress. The page shows the real current state: teams, current map, current score, and the player's live stats (K-D, first bloods, clutches, economy). The world clock is paused. The player may watch, accelerate, skip non-critical rounds, and make limited in-match decisions (map veto, game plan, tactical adjustments at half-time / losing streaks / economic turning points).
- **RESULT** — the match has ended. Show the result, map scores, per-player data, key rounds, and the official Rating with its口径 (map / series / season).
- **REPLAY** — a completed match watched back. Deterministic replay only; it cannot change the result. It is not LIVE.

### Player Presentation

The player header must show nickname/name as the dominant label. Team, role, age, ranking, and status are supporting metadata. Career stats and underlying skill attributes are separate concepts and must not be visually merged.

Core stat labels use canonical forms: `RATING 2.0`, `DPR`, `KAST`, `ADR`, `KPR`, `ROUND SWING`. Qualitative labels such as `POOR` and `OKAY` must accompany, not replace, values.

### Actions

An action tile contains an icon, command name, and current consequence/status. Actions route to an actual workflow. Do not add a tile that only opens explanatory text. Routine actions are not shown every cycle; they appear when relevant (a transfer offer, a training window, a stand-in opportunity).

## 6. HLTV Visual System

The display layer follows an HLTV 2026 light theme as the final visual override layer (design tokens captured from the removed `frontend/src/styles/global.css`, see visual notes below).

- Reference: HLTV.org 2026 — light grey canvas `#e9edf1`, white cards `#ffffff`, thin borders `#d9e0e8`.
- Panels: white `#ffffff` with inner raised surfaces `#f7f9fb`.
- Primary accent: HLTV blue `#2d7dd2`. Positive green `#2e9e5b`; negative/injury red `#d64545`; information blue `#2d7dd2`; warning amber `#e8930c`.
- Borders provide structure; shadows are reserved for modals and are subtle (max `0 24px 80px rgba(16,38,64,.35)`).
- Panels use 4px radius; compact rows use 3-4px.
- Use one accent edge (3px top border or left border) per section. Do not fill entire panels with accent color.
- No decorative gradients, glow orbs, glass effects, oversized hero typography, or floating cards. The live-match overlay may keep a dark immersive backdrop, but its panels stay white.
- English uppercase is limited to canonical esports labels and compact eyebrows. Main commands remain Chinese.
- Numerical data use `var(--mono)` and tabular figures (`font-variant-numeric: tabular-nums`).
- Letter spacing is zero except compact uppercase eyebrows, capped at `0.1em`.

### HLTV table conventions

- Dense, compact tables with thin row borders `#d9e0e8` and hover background `#edf2f7`.
- Numeric columns right-aligned; header row uppercase compact labels.
- Rating 2.0 column emphasized; green at/above 1.00, red below.
- Ranking tables show rank number, team/player, and a highlighted delta or badge.
- Empty states state the next valid action, such as advancing time, arranging training, or waiting for the next offer.

## 7. Layout and Responsive Rules

Desktop content max width is 1240px. The hub uses dense two-column grids with 12px gaps. Fixed-format elements need explicit grid tracks so dynamic labels cannot move neighboring controls.

At 1080px, secondary panels stack below the primary panel. At 760px, the hero becomes one column, actions become a 2x2 grid, and low-priority match columns may be hidden. Team names truncate; commands and numeric results do not.

Never place cards inside cards. A layout wrapper like `hub-side` is only a layout wrapper. A panel can contain rows, tables, meters, or action tiles, but not another panel.

## 8. Interaction and Feedback

- Hover states use border/background changes and at most 1px translation.
- Layout dimensions must remain stable on hover and loading.
- Disabled simulation controls visibly remain in place.
- Pending decisions and injuries require persistent badges until resolved.
- Match outcomes use both text (`W`/`L`, score, value) and color.
- Empty states state the next valid action.
- Keyboard focus must remain visible for links, buttons, and form controls.
- The simulation is automatic: the top bar shows whether time is `播放中` or `已暂停`, and the reason it paused (e.g., "比赛即将开始", "收到转会报价"). The player can always pause manually and resume.

## 9. Non-Visual LLM Rules

When updating a component without image understanding, follow this document and existing class conventions exactly. Do not redesign from prose alone.

1. Read the protocol contract types in `docs/analysis/11-前端-api与store.md` and the lightweight view DTOs (`GET /games/{id}/view` → `ClientState`) before adding data.
2. Never hard-code player names, teams, ratings, match results, dates, or transfer events in a production component.
3. Derive display values through selectors or small pure functions. Do not duplicate simulation formulas in JSX.
4. Keep the simulation engine and API contract unchanged unless the task explicitly requires backend work.
5. Every new view needs loading/empty, normal, and exceptional states such as injury, free agent, no match, or no history.
6. Use existing `Portrait`, `TeamLogo`, `EventItem`, `AttributeBar`, and icons before creating alternatives.
7. Keep user actions as links or buttons. A decorative panel must not look clickable.
8. Do not use emoji as interface icons. Extend `components/icons.tsx` or use the existing icon set.
9. Do not add marketing copy, tutorial paragraphs, fake chat, fake viewers, fake loot, battle passes, currencies, or RPG systems unsupported by the simulation.
10. Validate the display layer against the backend contract (deterministic golden runs + protocol checks) after component changes.

## 10. Change Checklist

Before finishing a component update:

- Confirm all displayed values come from current state or a documented formatter.
- Confirm free-agent, injury, no-match, and no-history states do not crash.
- Confirm a short-term stand-in engagement is never rendered as a permanent roster change or a "bench" slot.
- Confirm no nested cards or explanatory feature copy was introduced.
- Confirm long player/team/event names truncate or wrap without overlap.
- Confirm 1280px and 390px layouts remain usable.
- Confirm positive/negative values include text or signs in addition to color.
- Confirm live, result, and replay states are visually distinguishable and correctly labeled.
- Confirm the top bar shows the auto-advance play/pause state and the pause reason.
- Run typecheck and production build.
