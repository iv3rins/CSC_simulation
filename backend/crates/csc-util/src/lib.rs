//! # csc-util —— 确定性 RNG + 稳定 ID
//!
//! Kotlin `me.iverins.csc.util/`（DeterministicRandom/Signatures）的 Rust 转写（M1）。
//!
//! 核心：**xoshiro256\*\* 确定性随机生成器**——与 Kotlin `DeterministicRandom`
//! 逐位一致（同算法、同种子派生、同拒绝采样），是跨语言可复现性的地基：
//! 同种子 + 同决策序列 → Kotlin 与 Rust 生成**完全相同的随机序列**。
//!
//! 契约：
//! - 状态 = 4 × u64（[`Xoshiro256StarStar::snapshot`]/[`from_state`]），随存档序列化；
//! - 零第三方依赖（不引入 rand crate，算法手写以锁死位级一致性）；
//! - 所有运算使用 wrapping 语义（与 Kotlin Long/Int 补码溢出一致）。

pub mod canonical;
pub mod error;
pub mod id;
pub mod math_utils;
pub mod rng;
pub mod sampling;
pub mod seed_chain;
pub mod signatures;

pub use canonical::canonical_json_bytes;
pub use error::SimError;
pub use id::{PlayerId, TeamId};
pub use math_utils::{round_to_int, sigmoid};
pub use rng::Xoshiro256StarStar;
pub use sampling::{gaussian, pick_index};
pub use seed_chain::{fnv1a64, round_seed};
