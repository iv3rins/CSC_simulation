//! 世界仓库（arena）—— 实体系统的持有与原子操作（Kotlin `EntityEngine` 的存储层转写）。
//!
//! 职责边界：本模块只负责**持有与一致性**（创建/查询/归属原子操作）；
//! 引擎编排职责（VRS 排名同步、队伍生成门面等）在 M8 `csc-core` 补全
//! （依赖 `csc-domain::VrsEntry` 与 `csc-vrs` 的 VrsEngine，见 lib.rs 说明）。

use csc_domain::tier::Tier;
use csc_util::id::{PlayerId, TeamId};
use csc_util::rng::Xoshiro256StarStar;

use crate::character::PlayerCharacter;
use crate::generator::RandomPlayerGenerator;
use crate::role::Role;
use crate::team::Team;

/// World 原子操作错误——ID 无效（越界）或归属引用不变量被破坏。
///
/// 原子操作**不再隐式 panic**：失败以 `Result` 显式上抛，调用方决定
/// 传播（`?`）或标注内部不变量（`.expect`）。这是服务器协议接入前的
/// 错误面收敛——存档/读档、转会、人口更新等路径的非法 ID 可被拦截而非崩溃。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldError {
    /// 选手 ID 越界（无此 arena 槽位）
    PlayerNotFound(PlayerId),
    /// 队伍 ID 越界（无此 arena 槽位）
    TeamNotFound(TeamId),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorldError::PlayerNotFound(id) => write!(f, "选手不存在: PlayerId({})", id.0),
            WorldError::TeamNotFound(id) => write!(f, "队伍不存在: TeamId({})", id.0),
        }
    }
}

impl std::error::Error for WorldError {}

/// 世界仓库：全部实体的 arena（**id 即 Vec 索引**，存档 = arena 快照）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct World {
    /// 选手 arena（Player 与 NPC 统一存储，`career: Option` 区分）
    pub players: Vec<PlayerCharacter>,
    /// 队伍 arena
    pub teams: Vec<Team>,
    /// 自由市场（转会中离开队伍的选手；`team = None` 即自由身）
    pub free_agents: Vec<PlayerId>,
    /// 下一个选手 ID（= players.len()，序列化保一致）
    next_player_id: u32,
    /// 下一个队伍 ID（= teams.len()）
    next_team_id: u32,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// 空世界（id 从 0 开始，即 arena 索引）。
    pub fn new() -> Self {
        Self {
            players: Vec::new(),
            teams: Vec::new(),
            free_agents: Vec::new(),
            next_player_id: 0,
            next_team_id: 0,
        }
    }

    // —— 创建 ——

    /// 创建并登记一名玩家（主角）：初始自由身（team=None、0 合同年）。
    ///
    /// @param age 指定初始年龄（主角固定 16 岁起步）；None = 随机 18~30。
    /// @param role 指定角色定位（None = 随机）。
    pub fn create_player(
        &mut self,
        tier: Tier,
        name: Option<&str>,
        rng: &mut Xoshiro256StarStar,
        age: Option<i32>,
        role: Option<Role>,
    ) -> PlayerId {
        let mut p = RandomPlayerGenerator::generate_player(tier, name, rng, age, role);
        p.id = PlayerId(self.next_player_id);
        self.next_player_id += 1;
        self.players.push(p);
        PlayerId(self.players.len() as u32 - 1)
    }

    /// 创建并登记一名 NPC（可指定归属队伍）。
    ///
    /// 归属队伍无效时返回 [`WorldError::TeamNotFound`]（先校验、后登记，避免 ID 跳号）。
    pub fn create_npc(
        &mut self,
        tier: Tier,
        team: Option<TeamId>,
        role: Option<Role>,
        name: Option<&str>,
        rng: &mut Xoshiro256StarStar,
        age: Option<i32>,
    ) -> Result<PlayerId, WorldError> {
        // 先校验归属队伍，失败则不动任何状态（不递增 next_player_id → 不破坏「id == len」不变量）
        if let Some(tid) = team
            && self.teams.get(tid.0 as usize).is_none()
        {
            return Err(WorldError::TeamNotFound(tid));
        }
        let mut p = RandomPlayerGenerator::generate_npc(tier, team, role, name, rng, age);
        let id = PlayerId(self.next_player_id);
        p.id = id;
        self.next_player_id += 1;
        if let Some(tid) = team {
            let idx = self.teams.get_mut(tid.0 as usize).expect("已校验队伍存在");
            idx.roster_ids.push(id);
        }
        self.players.push(p);
        Ok(id)
    }

    /// 注册一个**已生成**的角色（初始装载：VrsMapper 生成 → 本方法登记 arena + 归属）。
    /// 与 `create_npc` 的区别：不重新生成属性，保留调用方构造的精确值（rng 消费在生成端）。
    ///
    /// 归属队伍无效时返回 [`WorldError::TeamNotFound`]（先校验、后登记）。
    pub fn push_character(
        &mut self,
        mut pc: PlayerCharacter,
        team: Option<TeamId>,
    ) -> Result<PlayerId, WorldError> {
        if let Some(tid) = team
            && self.teams.get(tid.0 as usize).is_none()
        {
            return Err(WorldError::TeamNotFound(tid));
        }
        let id = PlayerId(self.next_player_id);
        pc.id = id;
        self.next_player_id += 1;
        pc.team = team;
        if let Some(tid) = team {
            let idx = self.teams.get_mut(tid.0 as usize).expect("已校验队伍存在");
            idx.roster_ids.push(id);
        }
        self.players.push(pc);
        Ok(id)
    }

    /// 创建并登记一支队伍（空阵容）。
    pub fn create_team(
        &mut self,
        name: impl Into<String>,
        vrs_ranking: i32,
        vrs_value: i32,
    ) -> TeamId {
        let id = TeamId(self.next_team_id);
        self.next_team_id += 1;
        self.teams.push(Team::new(id, name, vrs_ranking, vrs_value));
        id
    }

    // —— 查询 ——

    /// 按 ID 查选手（ID 即索引；越界返回 None）。
    pub fn player(&self, id: PlayerId) -> Option<&PlayerCharacter> {
        self.players.get(id.0 as usize)
    }

    /// 按 ID 查选手（可变）。
    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut PlayerCharacter> {
        self.players.get_mut(id.0 as usize)
    }

    /// 按 ID 查队伍。
    pub fn team(&self, id: TeamId) -> Option<&Team> {
        self.teams.get(id.0 as usize)
    }

    /// 按 ID 查队伍（可变）。
    pub fn team_mut(&mut self, id: TeamId) -> Option<&mut Team> {
        self.teams.get_mut(id.0 as usize)
    }

    /// 按昵称查选手（线性；原型规模小；引用一律用 ID）。退役者不可查。
    pub fn player_by_name(&self, name: &str) -> Option<PlayerId> {
        self.players
            .iter()
            .find(|p| p.name == name && !p.retired)
            .map(|p| p.id)
    }

    /// 按队名查队伍（线性）。
    pub fn team_by_name(&self, name: &str) -> Option<TeamId> {
        self.teams.iter().find(|t| t.name == name).map(|t| t.id)
    }

    /// 全部选手。
    pub fn all_players(&self) -> &[PlayerCharacter] {
        &self.players
    }

    /// 全部玩家（主角；= Kotlin `EntityEngine.allPlayers()`）。退役者不返回。
    pub fn all_players_only(&self) -> Vec<&PlayerCharacter> {
        self.players
            .iter()
            .filter(|p| p.is_player() && !p.retired)
            .collect()
    }

    /// 全部 NPC（= Kotlin `EntityEngine.allNpcs()`）。退役者不返回。
    pub fn all_npcs(&self) -> Vec<&PlayerCharacter> {
        self.players
            .iter()
            .filter(|p| p.is_npc() && !p.retired)
            .collect()
    }

    /// 全部队伍。
    pub fn all_teams(&self) -> &[Team] {
        &self.teams
    }

    /// 自由市场中的全部选手。
    pub fn all_free_agents(&self) -> Vec<PlayerId> {
        self.free_agents.clone()
    }

    /// 队伍的阵容选手引用（= Kotlin `Team.roster` 的 arena 形态）。
    pub fn roster(&self, team_id: TeamId) -> Vec<&PlayerCharacter> {
        let Some(team) = self.teams.get(team_id.0 as usize) else {
            return Vec::new();
        };
        team.roster_ids
            .iter()
            .filter_map(|pid| self.player(*pid))
            .collect()
    }

    /// 队伍阵容签名（= Kotlin `Signatures.teamSignature`：`队名 | 选手名排序 join(",")`）。
    pub fn signature_of(&self, team_id: TeamId) -> String {
        let Some(team) = self.teams.get(team_id.0 as usize) else {
            return String::new();
        };
        let mut names: Vec<String> = team
            .roster_ids
            .iter()
            .filter_map(|pid| self.player(*pid))
            .map(|p| p.name.clone())
            .collect();
        names.sort();
        team.signature(&names)
    }

    // —— 归属原子操作（维护 roster_ids ⟷ player.team 双端不变量）——

    /// 把选手签入队伍（双端同步：roster_ids push + player.team = Some）。
    /// 幂等：已在阵容中则仅刷新 team 字段。
    ///
    /// 任一 ID 无效时返回 [`WorldError`]（先校验、后写入，避免「player.team 已改、
    /// roster_ids 未同步」的半更新）。
    pub fn assign_player_to_team(
        &mut self,
        player: PlayerId,
        team_id: TeamId,
    ) -> Result<(), WorldError> {
        if self.players.get(player.0 as usize).is_none() {
            return Err(WorldError::PlayerNotFound(player));
        }
        if self.teams.get(team_id.0 as usize).is_none() {
            return Err(WorldError::TeamNotFound(team_id));
        }
        let p = self
            .players
            .get_mut(player.0 as usize)
            .expect("已校验选手存在");
        p.team = Some(team_id);
        let t = self
            .teams
            .get_mut(team_id.0 as usize)
            .expect("已校验队伍存在");
        if !t.roster_ids.contains(&player) {
            t.roster_ids.push(player);
        }
        Ok(())
    }

    /// 把选手移出队伍（双端同步：roster_ids 移除 + player.team = None）。
    ///
    /// 选手或其所属队伍 ID 无效时返回 [`WorldError`]。
    pub fn release_player(&mut self, player: PlayerId) -> Result<(), WorldError> {
        let p = self
            .players
            .get_mut(player.0 as usize)
            .ok_or(WorldError::PlayerNotFound(player))?;
        if let Some(tid) = p.team.take() {
            let t = self
                .teams
                .get_mut(tid.0 as usize)
                .ok_or(WorldError::TeamNotFound(tid))?;
            t.roster_ids.retain(|id| *id != player);
        }
        Ok(())
    }

    /// 放入自由市场（= Kotlin `EntityEngine.addFreeAgent`；同时解除队伍归属）。
    pub fn add_free_agent(&mut self, player: PlayerId) -> Result<(), WorldError> {
        self.release_player(player)?;
        if !self.free_agents.contains(&player) {
            self.free_agents.push(player);
        }
        Ok(())
    }

    /// 从自由市场移除（自由人退役等）。幂等：不在市场中则无操作，不失败。
    pub fn remove_free_agent(&mut self, player: PlayerId) {
        self.free_agents.retain(|id| *id != player);
    }

    /// 标记 NPC 退役（退役等世界人口更新；= Kotlin `EntityEngine.removeNpc`）。
    ///
    /// **ID 稳定性设计**：arena **不收缩**——退役 = 置 `retired` 标记 +
    /// 解除归属（roster_ids/自由市场清理）。保持「id = Vec 索引」存档不变量：
    /// 其余选手 ID 永远稳定；退役者作为档案记录保留在 arena。
    /// 查询层（`all_players`/`all_npcs`/`player_by_name` 等）过滤退役者。
    pub fn remove_npc(&mut self, player: PlayerId) -> Result<(), WorldError> {
        self.release_player(player)?;
        self.remove_free_agent(player);
        if let Some(pc) = self.players.get_mut(player.0 as usize) {
            pc.retired = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng() -> Xoshiro256StarStar {
        Xoshiro256StarStar::seed(42)
    }

    #[test]
    fn id_equals_arena_index() {
        let mut w = World::new();
        let p0 = w.create_player(Tier::Tier1, Some("A"), &mut rng(), None, None);
        let p1 = w.create_player(Tier::Tier1, Some("B"), &mut rng(), None, None);
        let t0 = w.create_team("Vitality", 1, 2000);
        assert_eq!(p0.0, 0);
        assert_eq!(p1.0, 1);
        assert_eq!(t0.0, 0);
        assert_eq!(w.player(p0).unwrap().name, "A");
        assert_eq!(w.team(t0).unwrap().name, "Vitality");
    }

    #[test]
    fn create_npc_registers_roster() {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let n = w
            .create_npc(
                Tier::Tier1,
                Some(t),
                Some(Role::Awp),
                Some("ZywOo"),
                &mut rng(),
                None,
            )
            .unwrap();
        assert_eq!(w.player(n).unwrap().team, Some(t));
        assert_eq!(w.team(t).unwrap().roster_ids, vec![n]);
        assert_eq!(w.roster(t).len(), 1);
    }

    #[test]
    fn assign_and_release_keep_invariant() {
        let mut w = World::new();
        let p = w.create_player(Tier::Tier1, Some("P"), &mut rng(), None, None);
        let t = w.create_team("Vitality", 1, 2000);
        w.assign_player_to_team(p, t).unwrap();
        assert_eq!(w.player(p).unwrap().team, Some(t));
        assert_eq!(w.team(t).unwrap().roster_ids, vec![p]);

        // 幂等：重复签约不重复入列
        w.assign_player_to_team(p, t).unwrap();
        assert_eq!(w.team(t).unwrap().roster_ids, vec![p]);

        w.release_player(p).unwrap();
        assert_eq!(w.player(p).unwrap().team, None);
        assert!(w.team(t).unwrap().roster_ids.is_empty());
    }

    #[test]
    fn signature_matches_kotlin_format() {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        w.create_npc(
            Tier::Tier0,
            Some(t),
            Some(Role::Awp),
            Some("ZywOo"),
            &mut rng(),
            None,
        )
        .unwrap();
        w.create_npc(
            Tier::Tier0,
            Some(t),
            Some(Role::Igl),
            Some("apEX"),
            &mut rng(),
            None,
        )
        .unwrap();
        assert_eq!(
            w.signature_of(t),
            "Vitality|ZywOo,apEX",
            "Kotlin 格式：无空格"
        );
    }

    #[test]
    fn free_agent_flow() {
        let mut w = World::new();
        let p = w.create_player(Tier::Tier1, Some("P"), &mut rng(), None, None);
        let t = w.create_team("Vitality", 1, 2000);
        w.assign_player_to_team(p, t).unwrap();
        w.add_free_agent(p).unwrap();
        assert_eq!(w.player(p).unwrap().team, None);
        assert_eq!(w.all_free_agents(), vec![p]);
        w.remove_free_agent(p);
        assert!(w.all_free_agents().is_empty());
    }

    #[test]
    fn serde_roundtrip_world() {
        let mut w = World::new();
        let t = w.create_team("Vitality", 1, 2000);
        let p = w.create_player(Tier::Tier1, Some("P"), &mut rng(), None, None);
        w.assign_player_to_team(p, t).unwrap();
        let json = serde_json::to_string(&w).unwrap();
        let back: World = serde_json::from_str(&json).unwrap();
        assert_eq!(w, back);
        // 恢复后 ID 分配不冲突（next_* 从快照恢复）
        let mut restored = back;
        let p2 = restored.create_player(Tier::Tier1, Some("Q"), &mut rng(), None, None);
        assert_eq!(p2.0, w.players.len() as u32);
    }
}
