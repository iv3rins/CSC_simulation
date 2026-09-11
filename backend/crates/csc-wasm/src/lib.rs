//! # csc-wasm —— WASM 绑定层（M10）
//!
//! 同一模拟核心（`csc-core` 及以下零 IO、零异步）编译到 `wasm32-unknown-unknown`，
//! 提供**本地单机模式的 v1 形态**：全自动推演 + 存档/读档/事件流。
//!
//! **v1 边界（诚实声明）**：`advance_month` 是同步整体（月内决策批次在函数内
//! 完成），WASM 同步边界下**无法在月中断点挂起等真人决策**——所以 v1 只提供
//! auto 推演（`csc_advance`）。真人交互（决策面板/图间干预）请走 `csc-server`
//! 的 WS 通道（D3：channel 决策源桥接）；WASM 交互模式需要"步骤化状态机"
//! （`EngineStep`，mapping D3 演进备选），列为后续工作。
//!
//! C ABI + JSON（零 wasm-bindgen 依赖；前端以 TextEncoder/JSON.parse 桥接）：
//!
//! ```text
//! csc_create(seed, standings_json, len, baseline_json, len) -> *mut WasmSession
//! csc_advance(session, months)                           -> 0/1（全自动推进）
//! csc_snapshot_json(session)                             -> GameState JSON（存档/渲染）
//! csc_restore(session, json, len)                        -> 0/1（读档）
//! csc_journal_json(session, since)                       -> 事件流增量 JSON
//! csc_free(session)
//! csc_string_free(ptr)                                    // 释放 csc_*_json 返回串
//! ```

use std::ffi::c_char;

use csc_core::engine::Engine;
use csc_core::state::GameState;
use csc_decision::source::AutoDecisionSource;
use csc_time::clock::SimClock;

/// WASM 会话：引擎独占。
pub struct WasmSession {
    pub engine: Engine,
}

// —— C ABI ——

fn read_json(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn c_string(s: &str) -> *mut c_char {
    std::ffi::CString::new(s)
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// 创建会话：standings JSON 形如 `{ "standings_2026_01_05.json": "<内容>", ... }`
/// （至少一期）；baseline 为 roles_baseline.json 内容（可空）。
/// 成功返回非空指针；失败返回 null（坏资产/无 standings）。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_create(
    seed: u64,
    standings_ptr: *const u8,
    standings_len: usize,
    baseline_ptr: *const u8,
    baseline_len: usize,
) -> *mut WasmSession {
    let standings_json = read_json(standings_ptr, standings_len);
    let baseline_json = read_json(baseline_ptr, baseline_len);
    let files: Vec<(String, String)> =
        match serde_json::from_str::<std::collections::HashMap<String, String>>(&standings_json) {
            Ok(map) => map.into_iter().collect(),
            Err(_) => return std::ptr::null_mut(),
        };
    let baseline = (!baseline_json.is_empty()).then_some(baseline_json);
    let Ok(engine) = Engine::load_from_standings(
        &files,
        &csc_core::CalibrationAssets {
            baseline: baseline.as_deref(),
            ..Default::default()
        },
        seed,
        SimClock::of(2026, 1, 1),
    ) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(WasmSession { engine }))
}

/// 释放会话。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_free(ptr: *mut WasmSession) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

/// 全自动推进 `months` 个月。返回 0 = 成功；1 = 失败（空指针）。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_advance(ptr: *mut WasmSession, months: u32) -> u32 {
    let Some(session) = (unsafe { ptr.as_mut() }) else {
        return 1;
    };
    let mut auto = AutoDecisionSource;
    // Auto 决策源不会产生协议错误；失败视为内部错误 → 返回 1
    if session.engine.run_season(months as i32, &mut auto).is_err() {
        return 1;
    }
    0
}

/// 世界快照 JSON（存档/渲染）。用后 `csc_string_free`。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_snapshot_json(ptr: *mut WasmSession) -> *mut c_char {
    let Some(session) = (unsafe { ptr.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let state = session.engine.snapshot();
    c_string(&serde_json::to_string(&state).unwrap_or_else(|_| "{}".into()))
}

/// 读档（JSON = GameState；低版本档自动迁移到 CURRENT_VERSION，未知版本拒绝）。返回 0 = 成功；1 = 失败。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_restore(
    ptr: *mut WasmSession,
    json_ptr: *const u8,
    json_len: usize,
) -> u32 {
    let Some(session) = (unsafe { ptr.as_mut() }) else {
        return 1;
    };
    let json = read_json(json_ptr, json_len);
    match GameState::migrate(&json) {
        Ok(state) => {
            session.engine.restore(state);
            0
        }
        Err(_) => 1,
    }
}

/// 事件流增量 JSON（seq 之后）：`{"events":[...],"next_seq":N}`。用后 `csc_string_free`。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_journal_json(ptr: *mut WasmSession, since: i32) -> *mut c_char {
    let Some(session) = (unsafe { ptr.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let events: Vec<_> = session.engine.journal().since(since);
    let next_seq = events.last().map(|e| e.seq() + 1).unwrap_or(since + 1);
    c_string(
        &serde_json::to_string(&serde_json::json!({ "events": events, "next_seq": next_seq }))
            .unwrap_or_else(|_| "{}".into()),
    )
}

/// 释放 `csc_*_json` 返回的字符串。
/// # Safety
/// 指针必须来自同一次会话的 `csc_create`（或为空）；JSON 指针/长度必须
/// 指向调用期间有效的内存。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn csc_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(std::ffi::CString::from_raw(ptr)) };
    }
}

// —— 宿主侧测试（native 下跑；wasm 目标仅编译验证）——

#[cfg(test)]
mod tests {
    use super::*;
    use csc_domain::tier::Tier;
    use csc_util::id::TeamId;
    use csc_util::rng::Xoshiro256StarStar;

    fn fixture_files() -> Vec<(String, String)> {
        let mut rankings = String::new();
        for ti in 0..40 {
            if ti > 0 {
                rankings.push(',');
            }
            let roster: Vec<String> = (0..5).map(|i| format!("T{ti}P{i}")).collect();
            rankings.push_str(&format!(
                r#"{{"ranking":{},"points":{},"teamName":"Team{}","roster":[{}]}}"#,
                ti + 1,
                2000 - ti * 50,
                ti,
                roster
                    .iter()
                    .map(|n| format!(r#""{n}""#))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        vec![(
            "standings_global_2026_01_05.json".to_string(),
            format!(r#"{{"rankings":[{rankings}]}}"#),
        )]
    }

    #[test]
    fn c_abi_lifecycle() {
        let files = fixture_files();
        let standings_json = serde_json::to_string(&std::collections::HashMap::from([(
            files[0].0.clone(),
            files[0].1.clone(),
        )]))
        .unwrap();
        let ptr = unsafe {
            csc_create(
                42,
                standings_json.as_ptr(),
                standings_json.len(),
                std::ptr::null(),
                0,
            )
        };
        assert!(!ptr.is_null(), "创建失败");

        // 推进 1 个月
        assert_eq!(unsafe { csc_advance(ptr, 1) }, 0);

        // 快照 JSON 含月数
        let json_ptr = unsafe { csc_snapshot_json(ptr) };
        assert!(!json_ptr.is_null());
        let json = unsafe {
            std::ffi::CStr::from_ptr(json_ptr)
                .to_string_lossy()
                .into_owned()
        };
        assert!(json.contains("\"month\":1"), "快照应含月数：{json}");
        unsafe { csc_string_free(json_ptr) };

        // 事件流增量
        let j_ptr = unsafe { csc_journal_json(ptr, -1) };
        let j = unsafe {
            std::ffi::CStr::from_ptr(j_ptr)
                .to_string_lossy()
                .into_owned()
        };
        assert!(j.contains("events"), "事件流 JSON：{j}");
        unsafe { csc_string_free(j_ptr) };

        // 读档往返
        let state_json = {
            let p = unsafe { csc_snapshot_json(ptr) };
            let s = unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
            unsafe { csc_string_free(p) };
            s
        };
        assert_eq!(
            unsafe { csc_restore(ptr, state_json.as_ptr(), state_json.len()) },
            0
        );

        // 坏档拒绝
        assert_eq!(unsafe { csc_restore(ptr, b"{}".as_ptr(), 2) }, 1);

        // 空指针防御
        assert_eq!(unsafe { csc_advance(std::ptr::null_mut(), 1) }, 1);
        assert!(unsafe { csc_snapshot_json(std::ptr::null_mut()) }.is_null());

        unsafe { csc_free(ptr) };
    }

    #[test]
    fn create_rejects_bad_assets() {
        let bad = std::collections::HashMap::from([("standings_2026_01_05.json".to_string(), r#"{"rankings":[{"ranking":1,"points":-5,"teamName":"T","roster":["a","b","c","d","e"]}]}"#.to_string())]);
        let json = serde_json::to_string(&bad).unwrap();
        let ptr = unsafe { csc_create(42, json.as_ptr(), json.len(), std::ptr::null(), 0) };
        assert!(ptr.is_null(), "坏资产必须拒绝");
    }

    #[test]
    fn native_session_with_protagonist_advances() {
        // 宿主侧直接使用 WasmSession 结构（无 C ABI 绕行）
        let mut engine = Engine::load_from_standings(
            &fixture_files(),
            &csc_core::CalibrationAssets::default(),
            42,
            SimClock::of(2026, 1, 1),
        )
        .expect("fixture");
        let bottom = TeamId(engine.world().all_teams().len() as u32 - 1);
        engine
            .create_protagonist(
                "P",
                Tier::Tier4,
                bottom,
                &mut Xoshiro256StarStar::seed(7),
                None,
            )
            .expect("主角");
        let mut session = WasmSession { engine };
        let mut auto = AutoDecisionSource;
        session
            .engine
            .run_season(2, &mut auto)
            .expect("auto 推进必成功");
        assert_eq!(session.engine.snapshot().month, 2);
        assert!(session.engine.player().is_some());
    }

    /// P1-3：读档必须走版本迁移（WASM 曾漏调 migrate_state——低版本档会带旧版本号进引擎）。
    /// 构造 v1 档（把快照 version 字段改 1）经 csc_restore → 成功 + 引擎可用 +
    /// 再快照 version 已升回 CURRENT_VERSION（migrate 归一）。
    #[test]
    fn restore_upgrades_low_version_save() {
        let files = fixture_files();
        let standings_json = serde_json::to_string(&std::collections::HashMap::from([(
            files[0].0.clone(),
            files[0].1.clone(),
        )]))
        .unwrap();
        let ptr = unsafe {
            csc_create(
                42,
                standings_json.as_ptr(),
                standings_json.len(),
                std::ptr::null(),
                0,
            )
        };
        assert!(!ptr.is_null(), "创建失败");
        assert_eq!(unsafe { csc_advance(ptr, 1) }, 0);

        // 取当前档 JSON，把 version 改 1（模拟 v1 旧档——字段形状与 v9 兼容）
        let snap = {
            let p = unsafe { csc_snapshot_json(ptr) };
            let s = unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
            unsafe { csc_string_free(p) };
            s
        };
        let v1_json = snap.replacen(
            &format!("\"version\":{}", GameState::CURRENT_VERSION),
            "\"version\":1",
            1,
        );
        assert_ne!(v1_json, snap, "version 字段必须被改写");

        // 低版本档 restore：P1-3 前会失败/带旧版本进引擎；现在应成功且迁移归一
        assert_eq!(
            unsafe { csc_restore(ptr, v1_json.as_ptr(), v1_json.len()) },
            0,
            "低版本档读档必须成功（migrate 自动升 CURRENT_VERSION）"
        );

        // 读档后引擎可用：推进 + 快照 version 已归一
        assert_eq!(unsafe { csc_advance(ptr, 1) }, 0);
        let after = {
            let p = unsafe { csc_snapshot_json(ptr) };
            let s = unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
            unsafe { csc_string_free(p) };
            s
        };
        assert!(
            after.contains(&format!("\"version\":{}", GameState::CURRENT_VERSION)),
            "restore 后快照版本必须已升回 CURRENT_VERSION：{after}"
        );

        unsafe { csc_free(ptr) };
    }
}
