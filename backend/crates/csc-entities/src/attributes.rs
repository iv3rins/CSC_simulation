//! 选手能力属性分组（Kotlin `PlayerAttributes.kt`）。
//!
//! 取值约定：数值型属性一般为 0..100，越大越好。
//! 这些属性组被 [`crate::character::PlayerCharacter`] 承载，用于对战模拟。

use serde::{Deserialize, Serialize};

use crate::role::Role;

/// 基础属性：选手的身体 / 状态基础，随年龄与训练成长。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseAttributes {
    /// 反应时间 (0..100)：越快越能应对突发交火
    pub reaction: i32,
    /// 稳定性 (0..100)：越低发挥波动越大（影响单场 σ）
    pub stability: i32,
    /// 体能 (0..100)：影响长时间比赛 / 加时的续航
    pub endurance: i32,
    /// 精力 (0..100)：影响训练量与每日可投入时间
    pub stamina: i32,
    /// 健康 (0..100)：过低易伤病，影响生涯
    pub health: i32,
}

/// 技能属性：选手的技术能力，与对局表现直接相关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillAttributes {
    /// 瞄准能力 (0..100)：枪法核心，全角色通用权重最高
    pub aim: i32,
    /// 领导力 (0..100)：IGL 核心属性，影响战术执行
    pub leader: i32,
    /// 沟通 (0..100)：信息交流质量，影响团队磨合
    pub communication: i32,
    /// 残局能力 (0..100)：1vN 残局处理，关键分决定者
    pub clutch: i32,
}

/// 心理属性：选手的职业态度与心理素质。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProAttributes {
    /// 抗压能力 (0..100)：大场面/逆风时的稳定性
    pub mentality: i32,
    /// 自信 (0..100)：影响神经刀类选手的爆发上限
    pub confidence: i32,
    /// 团队配合 (0..100)：影响队伍磨合系数
    pub team_spirit: i32,
    /// 忠诚度 (0..100)：影响转会意愿与合同忠诚
    pub loyalty: i32,
    /// 士气 (0..100)：连败/连胜时波动，影响整体发挥
    pub morale: i32,
}

/// 武器熟练度：选手在不同枪械与道具上的专精（生涯模拟中与 Role 强相关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponAttributes {
    /// 当前主打位置（可随战术调整，默认与 role 一致）
    pub position: Role,
    /// AK-47 熟练度 (0..100)：步枪手核心
    pub ak: i32,
    /// AWP 熟练度 (0..100)：狙击手核心
    pub awp: i32,
    /// 手枪熟练度 (0..100)：经济局/手枪局关键
    pub pistol: i32,
    /// 道具使用（烟雾/闪/火）熟练度 (0..100)：辅助核心
    pub smoke: i32,
    /// 战术配合 / 地图理解 (0..100)：指挥与自由人核心
    pub utility: i32,
}
