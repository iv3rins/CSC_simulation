//! 选手角色定位（Kotlin `Roles.kt`）。

use serde::{Deserialize, Serialize};

/// 玩家在队伍中的角色定位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    /// 指挥，负责战术决策与团队调度
    Igl,
    /// 狙击手，使用 AWP 等狙击枪械
    Awp,
    /// 步枪手，使用步枪稳定输出
    Rifler,
    /// 突破手，率先进入交战区域打开局面
    Entry,
    /// 辅助，提供道具与火力支援
    Support,
    /// 自由人，独立行动寻找机会
    Lurker,
}

/// 全部角色（供随机采样/遍历；= Kotlin `Role.entries`）。
pub const ALL_ROLES: [Role; 6] = [
    Role::Igl,
    Role::Awp,
    Role::Rifler,
    Role::Entry,
    Role::Support,
    Role::Lurker,
];

/// 随机采样一个角色（= Kotlin `Role.entries.random(rng)`）。
pub fn random_role(rng: &mut csc_util::rng::Xoshiro256StarStar) -> Role {
    ALL_ROLES[rng.next_i32_bound(ALL_ROLES.len() as i32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use csc_util::rng::Xoshiro256StarStar;

    #[test]
    fn serde_names_match_kotlin() {
        assert_eq!(serde_json::to_string(&Role::Igl).unwrap(), r#""IGL""#);
        assert_eq!(serde_json::to_string(&Role::Awp).unwrap(), r#""AWP""#);
        assert_eq!(serde_json::to_string(&Role::Rifler).unwrap(), r#""RIFLER""#);
        assert_eq!(serde_json::to_string(&Role::Lurker).unwrap(), r#""LURKER""#);
    }

    #[test]
    fn random_role_in_range() {
        let mut rng = Xoshiro256StarStar::seed(42);
        for _ in 0..100 {
            let r = random_role(&mut rng);
            assert!(ALL_ROLES.contains(&r));
        }
    }
}
