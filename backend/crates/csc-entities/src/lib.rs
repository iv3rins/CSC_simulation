//! # csc-entities —— 实体系统（arena + ID 引用）
//!
//! Kotlin `me.iverins.csc.entities/`（19 文件 / 1546 行）的 Rust 转写（M2）。
//!
//! 核心重设计（对应 docs/09 B3/C1 + 转写蓝本 §4.1）：
//!
//! | Kotlin | Rust | 说明 |
//! |---|---|---|
//! | 继承：`PlayerCharacter` ← `Player`/`NPC` | **组合**：单一 `PlayerCharacter` + `career: Option<CareerInfo>` | 模拟路径统一，无虚表 |
//! | 对象图：`Team.roster: List<PlayerCharacter>`（引用共享） | **ID arena**：`Team.roster_ids: Vec<PlayerId>` + `World.players: Vec<PlayerCharacter>` | 引用→索引，存档=arena 快照 |
//! | `Team.with*` 返回新 Team（不可变快照） | `Team` 可变实体 + `World` 原子操作维护双端不变量 | 单写点防不一致 |
//! | `FreeAgentTeam.instance` 全局单例 | **`Option<TeamId>`（None = 自由身）** | 单例彻底消除（C1） |
//! | `CareerInfo.team: Team`（引用） | `CareerInfo` 不含 team——统一到实体 `team` 字段 | 单一事实来源 |
//! | `EntityEngine`（name/sig → 实体 map） | `World`（arena + 查询方法） | 引擎编排职责留到 M8 core |
//!
//! 依赖方向：`csc-domain`（值对象，含 `VrsEntry`）+ `csc-util`（RNG/ID）→ 本 crate；
//! 被 `csc-simulation`/`csc-tournaments` 等消费。VrsMapper（standings → 实体）
//! 依赖 `csc-domain::VrsEntry`（值对象已下沉，不反向依赖排名引擎）。

pub mod attributes;
pub mod baseline;
pub mod career;
pub mod character;
pub mod chemistry;
pub mod generator;
pub mod injury;
pub mod mapper;
pub mod mark;
pub mod narrative;
pub mod power;
pub mod role;
pub mod role_profile;
pub mod team;
pub mod transfer_contact;
pub mod world;

pub use attributes::{BaseAttributes, ProAttributes, SkillAttributes, WeaponAttributes};
pub use baseline::{AgeBand, RatingBaseline, RatingDist, RatingProfile, RoleBaseline};
pub use career::{
    CareerInfo, CareerMemory, CareerMemoryKind, Honours, IndividualHonour, IndividualHonourType,
    PlayerFinance, PlayerStatus, Sponsorship, TeamHonour,
};
pub use character::PlayerCharacter;
pub use chemistry::TeamChemistry;
pub use generator::{GeneratedPlayer, RandomPlayerGenerator};
pub use injury::{Injury, InjuryKind, InjurySeverity};
pub use mapper::VrsMapper;
pub use mark::{CareerMark, CareerMarkType, CareerMarks};
pub use narrative::{NarrativeProgress, Promise, PromiseStatus};
pub use power::PowerCalculator;
pub use role::{ALL_ROLES, Role};
pub use role_profile::{Attr, RoleProfile, RoleProfiles};
pub use team::Team;
pub use transfer_contact::TeamContact;
pub use world::{World, WorldError};
