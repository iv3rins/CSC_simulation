//! Scene commands share the game's single owner. World advancement remains gated
//! until the nested tournament continuation is implemented and verified.
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use csc_core::engine::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::persist::{build_persisted_envelope, write_persisted_package};
use super::types::Policy;

const MAX_RECEIPTS: usize = 64;
static GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareerSession {
    pub generation: String,
    pub receipts: Vec<CareerReceipt>,
}

impl Default for CareerSession {
    fn default() -> Self {
        // Transport identity only; never read by the simulation or its RNG.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            generation: format!(
                "{now:x}-{}",
                GENERATION_COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            receipts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareerChoice {
    pub generation: String,
    pub request_id: String,
    pub scene_instance_id: String,
    pub choice_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareerReceipt {
    #[serde(flatten)]
    pub request: CareerChoice,
    pub status: String,
    pub outcome: Value,
}

#[derive(Debug)]
pub enum CareerCommand {
    Continue,
    Choose(CareerChoice),
}

pub(super) fn load_state(
    engine: &mut Engine,
    session: &mut CareerSession,
    state: csc_core::state::GameState,
    path: Option<&Path>,
    policy: Policy,
) -> Result<(), String> {
    let path = path.ok_or("未配置持久化目录，不能确认读档")?;
    let mut candidate = engine.fork_snapshot().map_err(|e| e.to_string())?;
    candidate.restore_result(state).map_err(|e| e.to_string())?;
    let next_session = CareerSession::default();
    let mut envelope = build_persisted_envelope(policy, true, candidate.snapshot(), 3)?;
    envelope.career_session = next_session.clone();
    write_persisted_package(path, &envelope)?;
    *engine = candidate;
    *session = next_session;
    Ok(())
}

impl CareerSession {
    fn previous(&self, request: &CareerChoice) -> Result<Option<Value>, String> {
        if request.generation != self.generation {
            return Err("运行实例已改变，请重新读取当前生涯".into());
        }
        if request.request_id.is_empty() || request.request_id.len() > 128 {
            return Err("request_id 必须为 1–128 字节".into());
        }
        if let Some(receipt) = self
            .receipts
            .iter()
            .find(|r| r.request.request_id == request.request_id)
        {
            if receipt.request != *request {
                return Err("request_id 已用于不同的场景或选项".into());
            }
            return serde_json::to_value(receipt)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if self.receipts.len() >= MAX_RECEIPTS {
            return Err(
                "当前版本的 64 条回执容量已满；为防止旧请求重复执行，已暂停新增选择".into(),
            );
        }
        Ok(None)
    }
}

/// Caller is the game command loop. Publish state only after persistence succeeds.
pub(super) fn execute(
    engine: &mut Engine,
    session: &mut CareerSession,
    command: CareerCommand,
    path: Option<&Path>,
    policy: Policy,
) -> Result<Value, String> {
    if let CareerCommand::Choose(request) = &command
        && let Some(receipt) = session.previous(request)?
    {
        return Ok(receipt);
    }
    let path = path.ok_or("未配置持久化目录，不能确认生涯选择已保存")?;
    let mut candidate = engine.fork_snapshot().map_err(|e| e.to_string())?;
    let mut next_session = session.clone();
    let result = match command {
        CareerCommand::Continue => {
            if candidate.narrative().progress.active_scene.is_none() {
                let facts = candidate.narrative_facts();
                let scene = candidate.narrative().select_scene(&facts).ok_or(
                    "当前没有满足条件的新场景；正式比赛的可恢复推进尚未完成，暂不能推进世界",
                )?;
                candidate.begin_scene(&scene, &facts);
            }
            json!({"status":"awaiting_choice"})
        }
        CareerCommand::Choose(request) => {
            let active = candidate
                .narrative()
                .progress
                .active_scene
                .as_ref()
                .ok_or("当前没有待选择场景")?;
            if active.instance_id != request.scene_instance_id {
                return Err("场景已过期，请重新读取当前生涯".into());
            }
            let outcome = candidate
                .apply_story_choice(&request.choice_id)
                .map_err(|e| e.to_string())?;
            let receipt = CareerReceipt {
                request,
                status: "committed".into(),
                outcome: json!({"immediate":outcome.immediate,"tracking":outcome.tracking}),
            };
            let result = serde_json::to_value(&receipt).map_err(|e| e.to_string())?;
            next_session.receipts.push(receipt);
            result
        }
    };
    let mut envelope = build_persisted_envelope(policy, true, candidate.snapshot(), 3)?;
    envelope.career_session = next_session.clone();
    write_persisted_package(path, &envelope)?;
    *engine = candidate;
    *session = next_session;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_checks_complete_identity_before_pending_state() {
        let mut session = CareerSession::default();
        let request = CareerChoice {
            generation: session.generation.clone(),
            request_id: "r1".into(),
            scene_instance_id: "scene#1".into(),
            choice_id: "A".into(),
        };
        session.receipts.push(CareerReceipt {
            request: request.clone(),
            status: "committed".into(),
            outcome: json!({"immediate":["recorded"]}),
        });
        let original = session.previous(&request).unwrap().unwrap();
        let restored: CareerSession =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(restored.previous(&request).unwrap().unwrap(), original);
        let mut changed = request.clone();
        changed.scene_instance_id = "scene#2".into();
        assert!(session.previous(&changed).is_err());
        changed = request;
        changed.generation = "old".into();
        assert!(session.previous(&changed).is_err());
    }
}
