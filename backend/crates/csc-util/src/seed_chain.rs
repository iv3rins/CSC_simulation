//! 稳定 seed/hash helper —— LIVE 引擎的确定性铁律地基。
//!
//! 约束（与 `csc-simulation::replay` 的 `fnv1a64` 同源，但本模块属 csc-util，不依赖
//! 上层 crate）：同一组输入（world_seed / 比赛标识 / 回合编号 / 决策序列 / 版本）在
//! 任何进程、任何版本、任何语言下都必须派生出**完全相同**的 seed 值。
//!
//! 编码规则：所有字段以**固定宽度 + 长度前缀**写入字节缓冲，避免歧义（字符串长度
//! 前缀防止 "ab"+"c" 与 "a"+"bc" 碰撞），再用 FNV-1a 64 折叠为 u64。**不使用** Rust
//! 默认 `Hash`（其 `RandomState` 每次运行随机化，破坏可复现性）。

/// FNV-1a 64（跨平台稳定）。
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// LIVE 单个回合的确定性 seed。
///
/// 输入语义（方案 §3.2）：
/// - `world_seed`：世界种子（存档级，保证同一存档同一比赛同一回合序列可重现）；
/// - `match_id` / `map_id`：比赛/地图标识（赛事编排层生成，不用随机 UUID）；
/// - `round_number`：**下一回合**编号（比赛开始 = 1，随 `simulate_next_round` 递增）；
/// - `decision_seq`：已提交决策的单调计数（决策进入 seed 链，不同决策 → 不同 seed）；
/// - `simulation_version`：模拟器版本号（版本升级后 seed 链变化，旧回放仍可读）。
pub fn round_seed(
    world_seed: u64,
    match_id: &str,
    map_id: &str,
    round_number: i32,
    decision_seq: u32,
    simulation_version: u32,
) -> u64 {
    let mut out = Vec::with_capacity(64);
    // 域标签：防止与其它 hash 用途混淆。
    push_str(&mut out, "csc/live/round/v1");
    push_u64(&mut out, world_seed);
    push_str(&mut out, match_id);
    push_str(&mut out, map_id);
    push_i32(&mut out, round_number);
    push_u32(&mut out, decision_seq);
    push_u32(&mut out, simulation_version);
    fnv1a64(&out)
}

fn push_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// 长度前缀字符串：`len(u64 LE) + bytes`，杜绝相邻字符串的拼接歧义。
fn push_str(out: &mut Vec<u8>, s: &str) {
    push_u64(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_known_vector() {
        // 与 replay.rs 的 fnv1a64 一致（已知向量）。
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn round_seed_is_deterministic() {
        let a = round_seed(42, "m1", "map_1", 3, 0, 1);
        let b = round_seed(42, "m1", "map_1", 3, 0, 1);
        assert_eq!(a, b, "同输入必须同 seed");
    }

    #[test]
    fn round_seed_depends_on_all_inputs() {
        let base = round_seed(42, "m1", "map_1", 3, 0, 1);
        assert_ne!(base, round_seed(43, "m1", "map_1", 3, 0, 1), "world_seed");
        assert_ne!(base, round_seed(42, "m2", "map_1", 3, 0, 1), "match_id");
        assert_ne!(base, round_seed(42, "m1", "map_2", 3, 0, 1), "map_id");
        assert_ne!(base, round_seed(42, "m1", "map_1", 4, 0, 1), "round_number");
        assert_ne!(base, round_seed(42, "m1", "map_1", 3, 1, 1), "decision_seq");
        assert_ne!(
            base,
            round_seed(42, "m1", "map_1", 3, 0, 2),
            "simulation_version"
        );
    }

    #[test]
    fn length_prefix_prevents_ambiguity() {
        // "a"+"bc" 与 "ab"+"c" 若用裸拼接会碰撞；长度前缀保证不碰撞。
        let a = seed_of_pair("a", "bc");
        let b = seed_of_pair("ab", "c");
        assert_ne!(a, b, "长度前缀必须消除字符串拼接歧义");
    }

    fn seed_of_pair(x: &str, y: &str) -> u64 {
        let mut out = Vec::new();
        push_str(&mut out, x);
        push_str(&mut out, y);
        fnv1a64(&out)
    }
}
