use super::{err, game_of};
use crate::state::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use csc_career::archive::SeasonRecord;
use csc_core::query::{top20_of, top20_of_board};
use csc_events::{JournalChannel, ProjectionContext};
use serde::Deserialize;

// —— 查询 ——

pub(super) async fn get_state(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // 完整存档快照（兼容/调试/读档对比用）。**网页端高频刷新请走 `/view`**——
    // 20 年生涯的 GameState ≈68MB，轮询该端点没有可扩展性。
    let snapshot = entry.snapshot.lock().expect("snapshot 锁").clone();
    Json(snapshot).into_response()
}

/// 客户端轻量视图：主角 + roster + 主角赛事（series 懒加载）+ 概览计数。
/// 长生涯下保持 MB 级以下（完整快照的 1~3%）。
pub(super) async fn get_view(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let view = entry.view.lock().expect("view 锁").clone();
    Json(view).into_response()
}

/// 年度赛历：`plan` = 全年完整排期（与引擎动态排程同源），
/// `results` = 该年已完赛的**全部**赛事摘要（含 NPC 队胜负；不含逐图 KDA）。
/// 赛程页按需拉取，避免把 20 年全部赛果塞进高频 `/view`。
#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    pub year: Option<i32>,
}

pub(super) async fn get_calendar(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(q): Query<CalendarQuery>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let year = q.year.unwrap_or(snap.sim_year);
    let plan = csc_core::season_calendar_plan(year);
    let results = csc_core::calendar_results_for_year(&snap, year);
    // P0-A：玩家实际参与的赛程（不再用“全部同档赛事”误展示为“我的赛程”）。
    // 生成逻辑与主角识别见 csc_core::player_calendar_events（含单测）。
    let player_events = csc_core::player_calendar_events(&snap, year, &plan);

    Json(serde_json::json!({
        "year": year,
        "sim_date": format!("{}-{:02}-{:02}", snap.sim_year, snap.sim_month, snap.sim_day),
        "plan": plan,
        "results": results,
        "player_events": player_events,
    }))
    .into_response()
}

/// R2 赛事对象化：按赛事名返回聚合对象（参赛队伍+VRS+冠军+对阵布局+空状态原因）。
/// 供 LIVE 赛事页「赛事信息侧栏」与前端「下一场队伍赛事」承诺兑现——
/// 赛事不再只是日历 scheduled_record 集合，而是可查询的一等对象。
pub(super) async fn get_event_aggregate(
    State(state): State<AppState>,
    Path((id, event_name)): Path<(u64, String)>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    match csc_core::event_aggregate(&snap, &event_name) {
        Some(agg) => Json(agg).into_response(),
        None => err(StatusCode::NOT_FOUND, format!("赛事 {event_name} 不存在")),
    }
}

/// R2：标记某赛事对局已看完（watched 持久语义，幂等）。按 event_name + fixture_id
/// 精确定位到 `scheduled_records` 中的 fixture，置 `watched=true`，写入存档。
/// 响应 `{ "watched": true, "found": bool }`。
#[derive(Debug, Deserialize)]
pub(crate) struct MarkWatchedRequest {
    pub event_name: String,
    pub fixture_id: u32,
}

pub(super) async fn mark_fixture_watched(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(req): Json<MarkWatchedRequest>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let result = tokio::task::spawn_blocking(move || {
        entry.arc().mark_watched(req.event_name, req.fixture_id)
    })
    .await;
    match result {
        Ok(Ok(found)) => {
            Json(serde_json::json!({ "watched": true, "found": found })).into_response()
        }
        Ok(Err(e)) => err(StatusCode::CONFLICT, e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

/// 世界新闻：近期 T1+ 对阵（含 NPC 队）——journal 为降噪只含玩家比赛，
/// 此处直接从完整赛事结果派生最近 10 场顶级系列赛（不 clone 全量存档）。
#[derive(Debug, serde::Serialize)]
pub struct NewsItem {
    pub date: String,
    pub event_name: String,
    pub tier: csc_domain::tourney_tier::TourneyTier,
    pub winner: String,
    pub loser: String,
    pub score: String,
}

pub(super) async fn get_news(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let mut items: Vec<NewsItem> = Vec::new();
    // HLTV 近期对阵口径：每场赛事最多取最后 3 场（决赛/半决赛优先），
    // 这样新闻不会 10 条全是同一场赛事的同一天比赛，而是覆盖最近多场 T1。
    const NEWS_PER_EVENT: usize = 3;
    for ev in snap.events.iter().rev() {
        if !ev.event.tier.is_elite() {
            continue;
        }
        for series in ev.series.iter().rev().take(NEWS_PER_EVENT) {
            // 系列赛大比分（如 "2:0"/"3:2"）——HLTV 近期对阵的紧凑口径
            let winner_maps = series
                .maps
                .iter()
                .filter(|m| m.winner_sig == series.winner_sig)
                .count();
            let loser_maps = series.maps.len() - winner_maps;
            let score = format!("{winner_maps}:{loser_maps}");
            items.push(NewsItem {
                date: ev.event.date.clone(),
                event_name: ev.event.name.clone(),
                tier: ev.event.tier,
                winner: series
                    .winner_sig
                    .split('|')
                    .next()
                    .unwrap_or(&series.winner_sig)
                    .to_string(),
                loser: series
                    .loser_sig
                    .split('|')
                    .next()
                    .unwrap_or(&series.loser_sig)
                    .to_string(),
                score,
            });
        }
        if items.len() >= 10 {
            break;
        }
    }
    Json(serde_json::json!({ "items": items })).into_response()
}

/// 单场直播回放懒加载（series 不在 ClientState 中下发）。
/// 按 `event_name + date + match_index` 从完整快照中只 clone 一场 series。
#[derive(Debug, Deserialize)]
pub struct ReplayQuery {
    pub event: String,
    pub date: String,
    pub index: usize,
}

pub(super) async fn get_replay(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(q): Query<ReplayQuery>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let series = snap
        .events
        .iter()
        .find(|ev| ev.event.name == q.event && ev.event.date == q.date)
        .and_then(|ev| ev.series.get(q.index))
        .filter(|s| s.replayable)
        .cloned();
    match series {
        Some(series) => {
            // 回合级回放：纯函数确定性生成（种子派生自 series 本身），
            // 不存储、不消耗主 RNG——同一场比赛永远得到同一回放。
            let replay = csc_simulation::ReplayBuilder::build(&series);
            Json(serde_json::json!({ "series": series, "replay": replay })).into_response()
        }
        None => err(StatusCode::NOT_FOUND, "该场次不可回放或不存在"),
    }
}

pub(super) async fn get_summary(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // 概览计数已内联在 ClientState 里：锁内只取标量，不 clone 视图本体
    let view = entry.view.lock().expect("view 锁");
    let date = view.date.clone();
    Json(serde_json::json!({
        "game_id": id,
        "policy": entry.policy,
        "month": view.month,
        "date": date,
        "team_count": view.team_count,
        "player_count": view.player_count,
        "npc_count": view.npc_count,
        "journal_len": view.journal_len,
        "decisions_logged": view.decisions_logged,
        "events_len": view.event_count,
    }))
    .into_response()
}

/// journal 服务端分页上限（M11）：单响应最多返回 `JOURNAL_PAGE_LIMIT` 条事件，
/// 20 年档单响应不再可能达数十 MB；前端增量对账协议不变（`next_seq` 语义 =
/// 已同步游标，一次同步后若达到 limit 说明还有更多，前端可选循环拉取）。
pub const JOURNAL_PAGE_LIMIT: usize = 200;
/// limit 参数硬上界（防恶意超大请求拖垮渲染/序列化）。
const JOURNAL_PAGE_LIMIT_MAX: usize = 1000;

#[derive(Debug, Deserialize)]
pub struct JournalQuery {
    pub since: Option<i32>,
    /// 默认 career_feed；传 world_wire 获取完整世界线。
    pub channel: Option<JournalChannel>,
    /// 服务端分页上限（M11）：缺省 `JOURNAL_PAGE_LIMIT`（200），clamp(1, 1000)。
    /// 前端旧版（不传 limit）行为不变（默认 200，增量协议兼容）。
    pub limit: Option<usize>,
}

pub(super) async fn get_journal(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(q): Query<JournalQuery>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let since = q.since.unwrap_or(-1);
    let channel = q.channel.unwrap_or(JournalChannel::CareerFeed);
    let limit = q
        .limit
        .unwrap_or(JOURNAL_PAGE_LIMIT)
        .clamp(1, JOURNAL_PAGE_LIMIT_MAX);
    // M12 锁内只做窄读（不 clone 完整 GameState；journal 是全量快照中会随
    // 时间无限增长的部分）：主角标量反解 + 该 channel 可见的窗口（seq 过滤 →
    // channel 可见性过滤 → take(limit) → clone 需渲染窗口）。narrator 渲染与
    // projection 序列化是纯函数，移到锁外（AppState.text 启动期装载、运行期不可变）。
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let protagonist_name = snap
        .world
        .players
        .iter()
        .find(|p| p.is_player())
        .map(|p| p.name.clone());
    let protagonist_id = snap
        .world
        .players
        .iter()
        .find(|p| p.is_player())
        .map(|p| p.id);
    let protagonist_team_id = protagonist_id
        .and_then(|pid| snap.world.player(pid))
        .and_then(|pc| pc.team);
    let protagonist_team = protagonist_team_id
        .and_then(|tid| snap.world.team(tid))
        .map(|t| t.name.clone());
    let projection_context = ProjectionContext {
        protagonist_id,
        protagonist_team_id,
    };
    // **channel 可见性过滤必须在 take 之前**：若先 take 再过滤，窗口内全是
    // world_wire 事件时 career_feed 会「推进游标但 0 条事件」，中间 career
    // 事件被游标跳过而永久丢失（M11 游标语义的关键约束）。
    let window: Vec<csc_events::event::WorldEvent> = snap
        .journal
        .iter()
        .filter(|e| e.seq() > since)
        .filter(|e| csc_events::projection::project(e, projection_context).visible_in(channel))
        .take(limit)
        .cloned()
        .collect();
    // M11 next_seq 语义改造：取**截断窗口末端** + 1（而非全流末端）——否则前端
    // journalSeq 直接跳到全流末端，中间事件永久丢失（store/game.tsx 会把
    // next_seq - 1 当作已同步）。空窗口（无新事件）→ 全流末端兜底（与现协议一致）。
    let next_seq = window
        .last()
        .map(|e| e.seq() + 1)
        .or_else(|| snap.journal.last().map(|e| e.seq() + 1))
        .unwrap_or(0);
    drop(snap);

    // —— 锁外渲染（M12）：Narrator 纯函数；text 包不可变，安全持引用 ——
    let narrator =
        csc_events::Narrator::view(protagonist_name.as_deref(), protagonist_team.as_deref());
    let narrated: Vec<serde_json::Value> = window
        .iter()
        .filter_map(|e| {
            let narration = csc_events::Narrator::render(&narrator, e, &state.text);
            let mut v = csc_events::projection::projected_json(e, projection_context)?;
            if let Some(n) = narration {
                let obj = v.as_object_mut()?;
                obj.insert(
                    "narration".into(),
                    serde_json::json!({ "title": n.title, "body": n.body, "aside": n.aside }),
                );
            }
            Some(v)
        })
        .collect();
    Json(serde_json::json!({ "events": narrated, "next_seq": next_seq })).into_response()
}

pub(super) async fn get_archive(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // M12：锁内只 clone 档案 map（u32 键序列化为 JSON 对象键——serde_json 对
    // 整数键自动字符串化）；BTreeMap 重组是纯值操作，放锁外。
    let archive: std::collections::BTreeMap<u32, Vec<SeasonRecord>> = {
        let snap = entry.snapshot.lock().expect("snapshot 锁");
        snap.archive.iter().map(|(k, v)| (k.0, v.clone())).collect()
    };
    Json(serde_json::json!({ "archive": archive })).into_response()
}

/// 主角生涯结局/复盘（退役页数据源）：综合生涯汇总 + 结构化荣誉 + 印记产出
/// 「传奇等级（7 档阶梯）+ 动态 GOAT 评分 + 一句话定论 + 逐段复盘」。
/// 生涯尚短时 `tier="ROOKIE"`（未有定论）；**主角退役后仍可查询**（含 retired 标志）。
pub(super) async fn get_ending(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // M12 锁内窄 clone：只取主角生涯评估所需标量（pid/name/totals/career/retired +
    // 档案 map——仅主角赛季，**不 clone 完整 GameState**）；
    // `CareerEndingEvaluator::evaluate` 与序列化移锁外（纯函数，text 包不可变）。
    let (pid, name, totals, career, retired) = {
        let snap = entry.snapshot.lock().expect("snapshot 锁");
        let Some(pid) = snap
            .world
            .players
            .iter()
            .find(|p| p.is_player())
            .map(|p| p.id)
        else {
            return err(StatusCode::NOT_FOUND, "尚无主角");
        };
        let pc = snap.world.player(pid).expect("主角实体存在");
        let name = pc.name.clone();
        let retired = pc.retired;
        let career = pc.career.clone().expect("主角必有生涯数据");
        // 档案 map（仅主角赛季）clone + totals 锁内算好，锁外不再触碰快照。
        let mut archive = csc_career::archive::CareerArchive::default();
        archive.restore(snap.archive.clone());
        let Some(totals) = archive.totals_of(pid) else {
            return Json(
                serde_json::json!({ "pending": true, "note": "生涯尚未开启，没有可复盘的赛季" }),
            )
            .into_response();
        };
        (pid, name, totals, career, retired)
    };
    // —— 锁外：评估 + 序列化（CareerEndingEvaluator 纯函数）——
    let ending =
        csc_career::CareerEndingEvaluator::evaluate(pid, &name, &totals, &career, &state.text);
    Json(serde_json::json!({
        "player_id": pid,
        "player_name": name,
        "tier": serde_json::to_string(&ending.tier).unwrap_or_default(),
        "tier_label": ending.tier.label(),
        "goat_score": ending.goat_score,
        "retired": retired,
        "verdict": ending.verdict,
        "review": ending.review,
    }))
    .into_response()
}

/// 当前年度 HLTV TOP20 实时榜单：
/// 从快照的历年累计器 + 荣誉记录实时计算综合分 + 评语。
///
/// **跨年回退（2026 修复）**：新赛季初期实时榜的正常入围选手还不足
/// 20 人（2026 赛历 1 月即开赛，只有 5-6 人跨过早期样本门槛）——此时
/// 回退展示最近一届已收官榜单，避免玩家误以为「主角没进 T1，世界赛事
/// 就没有模拟」，也避免在头几个月展示残缺的年度榜。
pub(super) async fn get_top20(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    // M12 审计：`top20_of` 需要完整 `&World` 引用（Top20Evaluator 全量扫描
    // 选手/队伍查名与队伍归属），World 无法廉价 clone 移出锁；其计算量级
    // （~200 选手排序 + 评语格式化）远小于 journal 全流渲染与生涯评估——
    // 保持锁内借用计算（只读窄借用，不 clone 完整 GameState），
    // 锁外只做 JSON 序列化。真正的锁外化重活（journal/archive/ending）已落地。
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    let year = snap.sim_year;
    // 实时榜尚凑不满 20 名正常入围选手时 → 回退最近一届完整年度榜。
    // 2026 新赛历下 1 月就有 BLAST Bounty/IEM Kraków，实时榜会出现 5-6 人的
    // 早期样本；HLTV 语义下年度 TOP20 应等 20 名正常选手都跨过样本门槛后
    // 再切换为 CURRENT，此前继续展示上一届完整榜单。
    let live = top20_of(&snap.world, &snap.yearly_rating, year, &state.text);
    let qualified = live.iter().filter(|e| !e.wildcard).count();
    let historical = qualified < 20;
    if historical && let Some(last) = snap.top20_history.last() {
        let entries = top20_of_board(last, &state.text);
        return Json(serde_json::json!({
            "year": last.year,
            "source": "COMPLETED",
            "entries": entries,
        }))
        .into_response();
    }
    Json(serde_json::json!({ "year": year, "source": "CURRENT", "entries": live })).into_response()
}

/// 历届年度 TOP20 榜单快照（含 NPC 名次与入选依据：评分组成/样本量/荣誉次数）
/// ——历史榜单复盘：颁奖后年度累计器已清空，本端点从存档快照直接读出。
pub(super) async fn get_top20_history(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Response {
    let entry = match game_of(&state, id).await {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let snap = entry.snapshot.lock().expect("snapshot 锁");
    // top20_history 只有年度 20 人小榜；clone 榜单本身即可，不 clone 完整 GameState
    Json(serde_json::json!({ "boards": snap.top20_history.clone() })).into_response()
}
