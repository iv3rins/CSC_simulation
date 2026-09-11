//! 可被对战模拟的选手实体（Kotlin `PlayerCharacter.kt` + `Player.kt` + `NPC.kt` 的组合转写）。

use csc_util::id::PlayerId;
use csc_util::id::TeamId;
use serde::{Deserialize, Serialize};

use crate::attributes::{BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes};
use crate::career::CareerInfo;
use crate::injury::Injury;
use crate::role::Role;

/// 可被对战模拟的选手抽象（**组合而非继承**：NPC 与玩家共用同一结构）。
///
/// Kotlin 用继承（`PlayerCharacter` ← `Player`/`NPC`）；Rust 用单一结构 +
/// `career: Option<CareerInfo>` 区分（`Some` = 玩家，`None` = NPC）——
/// 模拟路径统一、无虚表、存档即结构体。
///
/// 取值约定：数值型属性一般为 0..100，越大越好。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerCharacter {
    /// 稳定 ID（arena 索引，见 [`crate::world::World`]）
    pub id: PlayerId,
    /// 选手昵称（如 "NiKo"；**展示字段**，引用一律用 id）
    pub name: String,
    /// 年龄（影响发挥波动；跨年 +1）
    pub age: i32,
    /// 角色定位（IGL/AWP/RIFLER...，决定属性画像）
    pub role: Role,
    /// 基础属性（反应/稳定/体能/精力/健康）
    pub base: BaseAttributes,
    /// 技能属性（瞄准/领导/沟通/残局）
    pub skill: SkillAttributes,
    /// 职业属性（抗压/自信/团队/忠诚/士气）
    pub pro: ProAttributes,
    /// 武器熟练度（AK/AWP/手枪/道具/战术）
    pub weapon: WeaponAttributes,
    /// 潜力上限（隐藏；开局随机分配，随年龄曲线驱动成长/衰退）
    pub potential: i32,
    /// 疲劳度（0..100）：比赛消耗、月度恢复；经 ConditionModel 折扣单场实力
    pub fatigue: f64,
    /// 当前伤病（None = 健康）
    pub injury: Option<Injury>,
    /// 职业数据（玩家特有；None = NPC）
    pub career: Option<CareerInfo>,
    /// 所属队伍（**None = 自由身**——FreeAgentTeam 单例的 Rust 替代，见 lib.rs 说明）
    pub team: Option<TeamId>,
    /// 已退役标记（World 退役 = 标记 + 清引用，**arena 不收缩**——
    /// 保持「id = Vec 索引」存档不变量，退役者保留为档案记录）
    pub retired: bool,
}

impl PlayerCharacter {
    /// 是否玩家主角（= Kotlin `is Player`）。
    pub fn is_player(&self) -> bool {
        self.career.is_some()
    }

    /// 是否 NPC 对手。
    pub fn is_npc(&self) -> bool {
        self.career.is_none()
    }

    /// 是否已退役（世界人口更新标记；退役者不参与任何查询）。
    pub fn is_retired(&self) -> bool {
        self.retired
    }

    /// 当前归属队伍（None = 自由身）。
    pub fn team(&self) -> Option<TeamId> {
        self.team
    }

    /// 可变的职业数据（仅玩家；NPC 返回 None）。
    pub fn career_mut(&mut self) -> Option<&mut CareerInfo> {
        self.career.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_vs_npc_discrimination() {
        let npc = PlayerCharacter {
            id: PlayerId(0),
            name: "shadow".into(),
            age: 20,
            role: Role::Awp,
            base: BaseAttributes {
                reaction: 80,
                stability: 80,
                endurance: 80,
                stamina: 80,
                health: 80,
            },
            skill: crate::attributes::SkillAttributes {
                aim: 80,
                leader: 50,
                communication: 50,
                clutch: 70,
            },
            pro: crate::attributes::ProAttributes {
                mentality: 70,
                confidence: 70,
                team_spirit: 60,
                loyalty: 50,
                morale: 60,
            },
            weapon: crate::attributes::WeaponAttributes {
                position: Role::Awp,
                ak: 60,
                awp: 90,
                pistol: 80,
                smoke: 50,
                utility: 50,
            },
            potential: 80,
            fatigue: 0.0,
            injury: None,
            career: None,
            team: None,
            retired: false,
        };
        assert!(npc.is_npc());
        assert!(!npc.is_player());
        assert_eq!(npc.team(), None);
    }
}
