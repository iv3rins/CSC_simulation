//! 测试共享 fixture：玩家属性拉满 + VRS 引擎构造（**唯一事实源**）。
//!
//! 历史上 `npc_market.rs` / `transfer/mod.rs` / `transfer/tests.rs` 各自复制过
//! 逐字符相同的 `boost_power` / `vrs_with_teams`（risk_diagnose Jaccard=1.00）。
//! 一旦单边修改，不同测试会在不同强度的选手上做断言，破坏可复现性契约。
//! 提取本模块后，三处测试共享同一实现。

use csc_entities::world::World;
use csc_util::id::PlayerId;
use csc_vrs::engine::VrsEngine;

/// 把玩家属性拉满到 T1 门槛以上（power ≥ 85）。
pub fn boost_power(world: &mut World, player_id: PlayerId) {
    let pc = world.player_mut(player_id).unwrap();
    pc.base.reaction = 95;
    pc.base.stability = 95;
    pc.skill.aim = 95;
    pc.skill.leader = 95;
    pc.skill.communication = 95;
    pc.skill.clutch = 95;
    pc.pro.mentality = 95;
    pc.pro.confidence = 95;
    pc.pro.team_spirit = 95;
    pc.weapon.ak = 95;
    pc.weapon.awp = 95;
    pc.weapon.pistol = 95;
    pc.weapon.smoke = 95;
    pc.weapon.utility = 95;
}

/// 用 `(name, ranking, points, roster)` 列表构造一个确定性 VrsEngine。
pub fn vrs_with_teams(teams: &[(&str, i32, i32, Vec<&str>)]) -> VrsEngine {
    let mut db = csc_vrs::database::VrsDatabase::from_json_files(&[]).expect("空资产必合法");
    for (name, ranking, points, roster) in teams {
        let roster_str: Vec<String> = roster.iter().map(|s| s.to_string()).collect();
        let sig = csc_vrs::database::VrsDatabase::signature_of(name, &roster_str);
        db.upsert_state(
            sig,
            csc_vrs::database::VrsTeamState {
                team_name: name.to_string(),
                points: *points,
                ranking: *ranking,
                roster: roster_str.clone(),
                last_settled_roster: roster_str,
                seed_points: *points,
            },
        );
    }
    VrsEngine::from_database(db)
}
