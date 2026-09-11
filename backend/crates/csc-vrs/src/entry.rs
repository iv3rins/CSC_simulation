//! 一条 VRS 全球排名记录（Kotlin `VrsEntry.kt` 转写；对应 standings 表的一行）。
//!
//! 架构说明：`VrsEntry` 是**纯值对象**，已下沉到 `csc-domain`（跨系统共享值对象），
//! 本模块仅为兼容 `csc_vrs::entry::VrsEntry` / `crate::entry::VrsEntry` 引用而 re-export。
//! 新代码请直接使用 `csc_domain::VrsEntry`。

pub use csc_domain::VrsEntry;
