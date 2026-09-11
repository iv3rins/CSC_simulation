use super::{err, game_of};
use crate::game::{Policy, SaveGuard};
use crate::state::AppState;
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use csc_core::engine::Engine;
use csc_core::state::GameState;
use csc_domain::tier::Tier;
use csc_time::clock::SimClock;
use csc_util::rng::Xoshiro256StarStar;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Deserialize;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

// —— 游戏生命周期 ——

/// 创建新游戏请求体。
#[derive(Debug, Deserialize)]
pub struct CreateGameRequest {
    #[serde(default)]
    pub story: bool,
    /// 世界种子（缺省 42）
    pub seed: Option<u64>,
    /// 决策策略（缺省 auto）
    pub policy: Option<Policy>,
    /// 主角昵称（缺省 "Player"）
    pub player_name: Option<String>,
    /// 主角角色定位（"IGL"|"AWP"|"RIFLER"|"ENTRY"|"SUPPORT"|"LURKER"；缺省 = 随机）
    pub role: Option<csc_entities::role::Role>,
}

pub(super) async fn create_game(
    State(state): State<AppState>,
    Json(req): Json<CreateGameRequest>,
) -> Response {
    let seed = req.seed.unwrap_or(Engine::DEFAULT_SEED);
    let policy = req.policy.unwrap_or(if req.story {
        Policy::Human
    } else {
        Policy::Auto
    });
    if req.story && policy != Policy::Human {
        return err(
            StatusCode::BAD_REQUEST,
            "故事模式需要 human 策略，不可自动代答",
        );
    }
    let story = req.story;
    let name = req.player_name.unwrap_or_else(|| "Player".to_string());
    let role = req.role;

    let assets = state.assets.clone();
    // 世界装载 + 主角创建是重活（真实资产 128 队），放阻塞线程池
    let result = tokio::task::spawn_blocking(move || -> Result<u64, String> {
        let mut engine = Engine::load_from_standings(
            &assets.standings,
            &csc_core::CalibrationAssets {
                baseline: assets.baseline.as_deref(),
                ratings: assets.ratings.as_deref(),
                rating_profile: assets.rating_profile.as_deref(),
                text: assets.text.as_deref(),
            },
            seed,
            SimClock::of(2026, 1, 1),
        )
        .map_err(|e| e.to_string())?;
        // 主角从垫底队伍起步（青训新秀叙事）：队伍按 VRS 排名升序生成，最后一支最弱
        let bottom_team =
            csc_util::id::TeamId(engine.world().all_teams().len().saturating_sub(1) as u32);
        // 主角卡独立于引擎主 RNG（同一世界可换卡）
        engine
            .create_protagonist(
                &name,
                Tier::Tier4,
                bottom_team,
                &mut Xoshiro256StarStar::seed(seed ^ 0x5EED_C0DE),
                role,
            )
            .map_err(|e| e.to_string())?;
        // R2 ① 修复：开局即物化当前月赛事排程 + fixtures + 锁（消除「卡赛事」——
        // 主页承诺 1 月有比赛但引擎此前不排程、比赛日静默滑过）。month_idx 与
        // `season_calendar_plan` 的月序平移一致（2026-01 → -1）。
        let (oy, om, _) = engine.clock().now();
        let opening_month_idx = (oy - 2026) * 12 + (om as i32 - 2);
        engine.plan_opening_month(opening_month_idx);
        if story {
            let content = assets
                .narrative
                .clone()
                .ok_or("缺少剧情资产，不能创建故事模式")?;
            engine
                .set_narrative_content(content)
                .map_err(|e| e.to_string())?;
            let facts = engine.narrative_facts();
            let scene = engine
                .narrative()
                .select_scene(&facts)
                .ok_or("剧情资产没有可用序章")?;
            engine.begin_scene(&scene, &facts);
        }
        Ok(state.games.create_with_story(engine, policy, story))
    })
    .await;

    match result {
        Ok(Ok(game_id)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "game_id": game_id })),
        )
            .into_response(),
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, format!("创建失败：{e}")),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "任务线程异常"),
    }
}

pub(super) async fn shutdown_game(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    match state.games.shutdown(id) {
        Some(()) => Json(serde_json::json!({ "shutdown": true })).into_response(),
        None => err(StatusCode::NOT_FOUND, format!("游戏 {id} 不存在")),
    }
}

// —— 存档 ——

/// 解压后 JSON 体积上限（防 gzip 解压炸弹；按 20 年 128 队档约 100–200MB 预留）。
const MAX_SAVE_JSON_BYTES: usize = 768 * 1024 * 1024;

/// 保存/读档操作超时（20 年档序列化+压缩实测 10s 内；留 30 倍余量防抖动与磁盘慢）。
const SAVE_OP_TIMEOUT: Duration = Duration::from_secs(300);

/// 统计 serde 实际写入字节数的适配器（获得 JSON 原始大小，不重复序列化）。
struct CountingWriter<W> {
    inner: W,
    written: usize,
}

impl<W: std::io::Write> std::io::Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// gzip 压缩产物（阻塞线程产物 + 表现层元数据）。
struct CompressedSave {
    bytes: Vec<u8>,
    version: u32,
    date: String,
    json_len: usize,
    crc32: u32,
}

/// 在阻塞线程池执行「取快照 + JSON 序列化 + gzip 压缩」。
/// 长生涯存档 CPU/内存开销大，不能占用 tokio worker。
fn compress_save(entry: Arc<crate::game::GameEntry>) -> Result<CompressedSave, String> {
    let snap = entry.state()?;
    let version = snap.version;
    let date = format!(
        "{}-{:02}-{:02}",
        snap.sim_year, snap.sim_month, snap.sim_day
    );
    // 直接流式写入 gzip 编码器，并用 CountingWriter 记录 JSON 原始大小，
    // 避免先构造完整 JSON Vec / 二次序列化（长生涯峰值内存减半）。
    let mut writer = CountingWriter {
        inner: GzEncoder::new(Vec::new(), Compression::default()),
        written: 0,
    };
    serde_json::to_writer(&mut writer, &snap).map_err(|e| format!("存档序列化失败：{e}"))?;
    let json_len = writer.written;
    let bytes = writer
        .inner
        .finish()
        .map_err(|e| format!("存档压缩失败：{e}"))?;
    let crc32 = crc32fast::hash(&bytes);
    Ok(CompressedSave {
        bytes,
        version,
        date,
        json_len,
        crc32,
    })
}

/// 存档/读档操作的统一脚手架：`game_of` → `SaveGuard` → 阻塞线程池执行 →
/// 超时/线程异常/操作错误四分支映射。四个端点（save / save_gzip / load /
/// load_gzip）共用同一模板（结构拆分收敛，此前每端点复制粘贴约 20 行）。
pub(super) async fn run_save_op<T>(
    state: &AppState,
    id: u64,
    conflict_msg: &'static str,
    op_name: &'static str,
    op_err_status: StatusCode,
    op: impl FnOnce(Arc<crate::game::GameEntry>) -> Result<T, String> + Send + 'static,
) -> Result<T, Response>
where
    T: Send + 'static,
{
    let entry = match game_of(state, id).await {
        Ok(e) => e,
        Err(r) => return Err(*r),
    };
    let Some(_save_guard) = SaveGuard::acquire(&entry.arc()) else {
        return Err(err(StatusCode::CONFLICT, conflict_msg));
    };
    let entry = entry.arc();
    match tokio::time::timeout(
        SAVE_OP_TIMEOUT,
        tokio::task::spawn_blocking(move || op(entry)),
    )
    .await
    {
        Ok(Ok(Ok(v))) => Ok(v),
        Ok(Ok(Err(e))) => Err(err(op_err_status, e)),
        Ok(Err(_)) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "存档任务线程异常")),
        Err(_) => Err(err(
            StatusCode::GATEWAY_TIMEOUT,
            format!("{op_name}超时（>300s）；请检查服务器负载后重试"),
        )),
    }
}

pub(super) async fn save_game(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let json = match run_save_op(
        &state,
        id,
        "本局已有存档任务进行中，请稍后再试",
        "存档",
        StatusCode::INTERNAL_SERVER_ERROR,
        |entry| {
            let snap = entry.state()?;
            serde_json::to_vec(&snap).map_err(|e| format!("存档序列化失败：{e}"))
        },
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        json,
    )
        .into_response()
}

/// 压缩存档下载：完整 GameState JSON → gzip（实测约 1/10）。
/// 返回裸 gzip 字节（application/gzip，不带 Content-Encoding，浏览器才能落盘）。
pub(super) async fn save_game_gzip(State(state): State<AppState>, Path(id): Path<u64>) -> Response {
    let save = match run_save_op(
        &state,
        id,
        "本局已有存档任务进行中，请稍后再试",
        "压缩存档",
        StatusCode::INTERNAL_SERVER_ERROR,
        compress_save,
    )
    .await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::HeaderName::from_static("x-csc-save-version"),
        HeaderValue::from_str(&save.version.to_string()).unwrap_or(HeaderValue::from_static("?")),
    );
    headers.insert(
        header::HeaderName::from_static("x-csc-save-date"),
        HeaderValue::from_str(&save.date).unwrap_or(HeaderValue::from_static("?")),
    );
    headers.insert(
        header::HeaderName::from_static("x-csc-json-bytes"),
        HeaderValue::from_str(&save.json_len.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    headers.insert(
        header::HeaderName::from_static("x-csc-gzip-bytes"),
        HeaderValue::from_str(&save.bytes.len().to_string())
            .unwrap_or(HeaderValue::from_static("0")),
    );
    headers.insert(
        header::HeaderName::from_static("x-csc-save-crc32"),
        HeaderValue::from_str(&format!("{:08x}", save.crc32))
            .unwrap_or(HeaderValue::from_static("0")),
    );
    (headers, save.bytes).into_response()
}

pub(super) async fn load_game(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(snap): Json<GameState>,
) -> Response {
    match run_save_op(
        &state,
        id,
        "本局正在保存或读档，请稍后再试",
        "读档",
        StatusCode::UNPROCESSABLE_ENTITY,
        move |entry| {
            let migrated = snap.migrate_state();
            entry.load(migrated)
        },
    )
    .await
    {
        Ok(()) => Json(serde_json::json!({ "loaded": true })).into_response(),
        Err(r) => r,
    }
}

/// 压缩存档读档：application/gzip → JSON → GameState → migrate → engine.restore。
/// 解压/解析放阻塞线程池，并限制解压后体积。
pub(super) async fn load_game_gzip(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    // 客户端可从文件名/保存时元数据携带校验和；服务端按需验证。
    if let Some(expected) = headers
        .get("x-csc-expected-crc32")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| u32::from_str_radix(v, 16).ok())
        && crc32fast::hash(&bytes) != expected
    {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "存档校验失败：CRC32 不匹配（文件可能损坏）",
        );
    }
    let snap = match run_save_op(
        &state,
        id,
        "本局正在保存或读档，请稍后再试",
        "压缩读档",
        StatusCode::UNPROCESSABLE_ENTITY,
        move |_entry| -> Result<GameState, String> {
            let mut json = Vec::new();
            GzDecoder::new(&bytes[..])
                .take((MAX_SAVE_JSON_BYTES + 1) as u64)
                .read_to_end(&mut json)
                .map_err(|e| format!("gzip 解压失败：{e}"))?;
            if json.len() > MAX_SAVE_JSON_BYTES {
                return Err(format!(
                    "解压后存档超过体积上限 {} MiB（疑似异常载荷）",
                    MAX_SAVE_JSON_BYTES / 1024 / 1024
                ));
            }
            serde_json::from_slice::<GameState>(&json)
                .map(|snap| snap.migrate_state())
                .map_err(|e| format!("压缩存档不是合法 GameState JSON：{e}"))
        },
    )
    .await
    {
        Ok(s) => s,
        Err(r) => return r,
    };
    match entry_load_via(&state, id, snap).await {
        Ok(()) => Json(serde_json::json!({ "loaded": true })).into_response(),
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// 在阻塞线程池执行 `entry.load`（load_game_gzip 的第二步：解压已完成，落引擎）。
pub(super) async fn entry_load_via(
    state: &AppState,
    id: u64,
    snap: GameState,
) -> Result<(), String> {
    let entry = match game_of(state, id).await {
        Ok(e) => e,
        Err(_) => return Err("本局不存在".to_string()),
    };
    let Some(_save_guard) = SaveGuard::acquire(&entry.arc()) else {
        return Err("本局正在保存或读档，请稍后再试".to_string());
    };
    let entry = entry.arc();
    match tokio::time::timeout(
        SAVE_OP_TIMEOUT,
        tokio::task::spawn_blocking(move || entry.load(snap)),
    )
    .await
    {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(e))) => Err(e),
        Ok(Err(_)) => Err("读档任务线程异常".to_string()),
        Err(_) => Err("读档超时（>300s）；请检查服务器负载后重试".to_string()),
    }
}
