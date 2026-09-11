//! 全状态树 NaN/∞ 扫描（ZenGM checkNaNs 同款卫生防线；DEF-001 事件驱动）。
//!
//! 实现：**check-only Serializer**（`serde::Serializer` 值语义包装 + `Rc<RefCell>`
//! 共享状态）——遍历 `GameState` 的序列化图，在 `serialize_f64`/`serialize_f32`
//! 检查有限性（NaN 与 ±∞ 命中，记录 JSON 路径），其余类型跳过。
//!
//! 为什么不用 `serde_json::to_value` 事后扫 `Value`：
//! - serde_json 对 NaN/±∞ 不产生 `Value::Number`（`serialize_f64` 写 `null`、
//!   `Number::from_f64` 返回 None）——非有限浮点的痕迹只剩 `Value::Null`；
//! - 而合法状态树的 `Option=None`（`PlayerCharacter.career/injury/team` 等）
//!   也编码为 `null`——两者无法区分，按 Null 判定必误报（会 panic 掉所有
//!   含 NPC 的推进测试）；
//! - 本实现让 f64 在**序列化入口**即被检查，`Option=None` 走 `serialize_none`
//!   （跳过），零误报。这是唯一精确且不误报的路径。
//!
//! 仅 debug 构建挂推进步钩子（性能），release 手动调用。

use std::cell::RefCell;
use std::rc::Rc;

use serde::ser::{self, Serialize};

use crate::state::GameState;

/// 扫描全状态树的非有限浮点。返回命中路径列表（JSON 路径风格，
/// 如 `world.players[3].career.assists`），空 = 干净。
pub fn scan_state_for_nan(state: &GameState) -> Result<(), Vec<String>> {
    let shared = Rc::new(RefCell::new(SharedChecker::default()));
    let ser = CheckSerializer {
        shared: shared.clone(),
    };
    state
        .serialize(ser)
        .map_err(|e| vec![format!("状态序列化失败：{e}")])?;
    let checker = shared.borrow();
    if checker.hits.is_empty() {
        Ok(())
    } else {
        Err(checker.hits.clone())
    }
}

/// 共享扫描状态（路径栈 + 命中列表）。
#[derive(Default)]
struct SharedChecker {
    path: Vec<String>,
    hits: Vec<String>,
}

impl SharedChecker {
    fn check_f64(&mut self, value: f64) {
        if !value.is_finite() {
            self.hits.push(self.path.join("."));
        }
    }
    fn check_f32(&mut self, value: f32) {
        if !value.is_finite() {
            self.hits.push(self.path.join("."));
        }
    }
}

/// check-only 序列化器（值语义；内部共享状态供容器借用）。
#[derive(Clone)]
struct CheckSerializer {
    shared: Rc<RefCell<SharedChecker>>,
}

impl ser::Serializer for CheckSerializer {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = CheckSeq;
    type SerializeTuple = CheckSeq;
    type SerializeTupleStruct = CheckSeq;
    type SerializeTupleVariant = CheckSeq;
    type SerializeMap = CheckMap;
    type SerializeStruct = CheckStruct;
    type SerializeStructVariant = CheckStruct;

    fn serialize_bool(self, _v: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i8(self, _v: i8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i16(self, _v: i16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i32(self, _v: i32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_i64(self, _v: i64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u8(self, _v: u8) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u16(self, _v: u16) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u32(self, _v: u32) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_u64(self, _v: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Result<(), Self::Error> {
        self.shared.borrow_mut().check_f32(v);
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Self::Error> {
        self.shared.borrow_mut().check_f64(v);
        Ok(())
    }
    fn serialize_char(self, _v: char) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_str(self, _v: &str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<(), Self::Error> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _variant: &'static str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _idx: u32,
        _variant: &'static str,
        v: &T,
    ) -> Result<(), Self::Error> {
        v.serialize(self)
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(CheckSeq {
            shared: self.shared,
            count: 0,
        })
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(CheckSeq {
            shared: self.shared,
            count: 0,
        })
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(CheckSeq {
            shared: self.shared,
            count: 0,
        })
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(CheckSeq {
            shared: self.shared,
            count: 0,
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(CheckMap {
            shared: self.shared,
        })
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(CheckStruct {
            shared: self.shared,
        })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _idx: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(CheckStruct {
            shared: self.shared,
        })
    }
}

/// 序列容器（seq/tuple/tuple_struct/tuple_variant 共用）：按调用序 push `[i]`。
struct CheckSeq {
    shared: Rc<RefCell<SharedChecker>>,
    count: usize,
}

impl CheckSeq {
    fn element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), serde_json::Error> {
        let idx = self.count;
        self.count += 1;
        self.shared.borrow_mut().path.push(format!("[{idx}]"));
        let r = value.serialize(CheckSerializer {
            shared: self.shared.clone(),
        });
        self.shared.borrow_mut().path.pop();
        r
    }
}

impl ser::SerializeSeq for CheckSeq {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.element(v)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for CheckSeq {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.element(v)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for CheckSeq {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.element(v)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for CheckSeq {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Self::Error> {
        self.element(v)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Map 容器：键名进路径（键序列化为字符串后 push）。
struct CheckMap {
    shared: Rc<RefCell<SharedChecker>>,
}

impl ser::SerializeMap for CheckMap {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        // 键通常是字符串；为记录路径，把键序列化到临时缓冲。
        let mut buf = Vec::new();
        let mut kser = serde_json::Serializer::new(&mut buf);
        key.serialize(&mut kser)?;
        let key_str = String::from_utf8(buf).unwrap_or_else(|_| "<key>".to_string());
        self.shared.borrow_mut().path.push(key_str);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        let r = value.serialize(CheckSerializer {
            shared: self.shared.clone(),
        });
        self.shared.borrow_mut().path.pop();
        r
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Struct 容器：字段名进路径。
struct CheckStruct {
    shared: Rc<RefCell<SharedChecker>>,
}

impl ser::SerializeStruct for CheckStruct {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.shared.borrow_mut().path.push(key.to_string());
        let r = value.serialize(CheckSerializer {
            shared: self.shared.clone(),
        });
        self.shared.borrow_mut().path.pop();
        r
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for CheckStruct {
    type Ok = ();
    type Error = serde_json::Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.shared.borrow_mut().path.push(key.to_string());
        let r = value.serialize(CheckSerializer {
            shared: self.shared.clone(),
        });
        self.shared.borrow_mut().path.pop();
        r
    }

    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// debug 构建断言无 NaN/∞（挂推进步出口）。
#[cfg(debug_assertions)]
pub fn assert_no_nan(state: &GameState) {
    if let Err(hits) = scan_state_for_nan(state) {
        panic!("状态树含非有限浮点：{hits:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 干净状态（engine 跑 1 月）→ 零命中（合法 Option=None 不误报）。
    #[test]
    fn clean_state_has_no_hits() {
        // 满编 40 队世界 + 主角（复用 soak.rs fixture 模板：load_from_standings +
        // create_protagonist + plan_opening_month(-1)），推进 1 个月后快照扫描。
        let mut eng = crate::engine::Engine::load_from_standings(
            &[(
                "standings_global_2026_01_05.json".to_string(),
                standings_fixture(40),
            )],
            &crate::CalibrationAssets::default(),
            42,
            csc_time::clock::SimClock::of(2026, 1, 1),
        )
        .expect("fixture 资产合法");
        let bottom = csc_util::id::TeamId(eng.world().all_teams().len().saturating_sub(1) as u32);
        eng.create_protagonist(
            "MyPlayer",
            csc_domain::tier::Tier::Tier4,
            bottom,
            &mut csc_util::rng::Xoshiro256StarStar::seed(42 ^ 0x5EED),
            None,
        )
        .expect("主角创建");
        eng.plan_opening_month(-1);
        eng.advance_month(&mut csc_decision::source::AutoDecisionSource)
            .expect("推进必成功");
        // 合法状态含大量 Option=None（NPC 的 career/injury/team）→ 不得误报。
        assert_eq!(scan_state_for_nan(&eng.snapshot()), Ok(()));
    }

    /// 注入 NaN → 命中路径可读（含 world 前缀）。
    #[test]
    fn injected_nan_is_reported_with_path() {
        let mut state = minimal_state();
        let pid = state
            .world
            .create_npc(
                csc_domain::tier::Tier::Tier1,
                None,
                None,
                Some("NaNPlayer"),
                &mut csc_util::rng::Xoshiro256StarStar::seed(7),
                None,
            )
            .expect("无归属队伍必成功");
        state.world.players[pid.0 as usize].fatigue = f64::NAN;
        let err = scan_state_for_nan(&state).expect_err("必须报告命中");
        assert!(
            err.iter().any(|p| p.starts_with("world.")),
            "路径应包含 world 前缀，实际：{err:?}"
        );
    }

    /// 注入 ±∞ 同样命中（与 NaN 同类卫生问题）。
    #[test]
    fn injected_infinity_is_reported() {
        let mut state = minimal_state();
        let pid = state
            .world
            .create_npc(
                csc_domain::tier::Tier::Tier1,
                None,
                None,
                Some("InfPlayer"),
                &mut csc_util::rng::Xoshiro256StarStar::seed(8),
                None,
            )
            .expect("无归属队伍必成功");
        state.world.players[pid.0 as usize].fatigue = f64::INFINITY;
        let err = scan_state_for_nan(&state).expect_err("必须报告命中");
        assert!(!err.is_empty());
    }

    /// 合法 Option=None 不误报：NPC 无 career/injury/team → 零命中。
    #[test]
    fn legal_none_options_are_not_hits() {
        let state = minimal_state();
        assert_eq!(scan_state_for_nan(&state), Ok(()));
    }

    /// 最小合法状态（与 state.rs 测试模块 minimal_state 同构；随测试模块私有化
    /// 无法跨模块复用，故就地复制——避免手写 JSON 与实际 serde 形状漂移）。
    fn minimal_state() -> GameState {
        use std::collections::HashMap;
        GameState {
            version: crate::state::GameState::CURRENT_VERSION,
            month: 0,
            last_contract_year: 2026,
            locks: vec![],
            sim_year: 2026,
            sim_month: 1,
            sim_day: 1,
            rng_state: [0, 0, 0, 0],
            decisions: vec![],
            journal: vec![],
            archive: HashMap::new(),
            world: csc_entities::world::World::new(),
            vrs: csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法"),
            events: vec![],
            yearly_rating: csc_tournaments::yearly_rating::YearlyRatingTracker::default(),
            top20_history: vec![],
            scheduled_records: vec![],
            sim_version: crate::state::WORLD_SIM_VERSION,
            narrative: Default::default(),
            execution: crate::state::ExecutionState::Idle,
        }
    }

    /// 生成 N 队 × 5 人的 standings JSON（照 soak.rs::standings_fixture 模板）。
    fn standings_fixture(team_count: usize) -> String {
        let mut rankings = String::new();
        for ti in 0..team_count {
            if ti > 0 {
                rankings.push(',');
            }
            let roster: Vec<String> = (0..5).map(|i| format!("T{ti}P{i}")).collect();
            let points = (2000_i64 - ti as i64 * 50).max(250);
            rankings.push_str(&format!(
                r#"{{"ranking":{},"points":{},"teamName":"Team{}","roster":[{}]}}"#,
                ti + 1,
                points,
                ti,
                roster
                    .iter()
                    .map(|n| format!(r#""{n}""#))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        format!(r#"{{"rankings":[{rankings}]}}"#)
    }
}
