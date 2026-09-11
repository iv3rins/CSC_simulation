# 多局容量模型与游戏生命周期（Sprint 2，已实现 v2）

## 1. 容量预算公式

单局峰值内存预算：

```
单局峰值 ≈ 活跃 Engine + 最新快照(GameState) + ClientState + pending
          + 保存任务峰值（快照 clone + JSON 流 + gzip 输出）
```

当前实测（128 队）：
- 12 个月快照 JSON ≈ 9.3MB；
- 20 年快照 JSON ≈ 176MB；
- gzip ≈ 快照 1/10（20 年约 16.4MB）；
- 保存任务峰值 ≈ 2×快照 + gzip 输出（流式已避免 3×）。

发布配置：
- `active_games` 默认 4；
- 并发突发（1/5/10 个长档同时保存/读档）允许暂时超过上限，**健康检查排发后台
  收缩，实测 67.8s 从 10 局回落到 4 局**；
- 保存/读档 HTTP 整体超时 300s，内部取快照超时 120s；
- 保存互斥用 RAII 守卫释放：客户端断连也不会把该局永久锁成 409。

## 2. 生命周期状态机

```
Active ──LRU预卸载──▶ Persisted ──按需恢复──▶ Active
  ▲                     │
  └────── 读取 gzip → Engine.restore_with_profile ──────┘
```

- **Active**：Engine 线程 + 内存快照 + ClientState。
- **Persisted**：gzip 写到 `persist_dir/{id}.game.gz`，先写 `.tmp` 再原子 rename
  （写入期间旧文件仍在，恢复请求不会看到「文件消失」）。
- **Evicted/Resume**：`get_or_restore` 自动恢复，`restore_with_profile` 保留新秀分布。
- **租约（lease）**：每个 HTTP 请求在 games 锁内登记 `GameHandle`，Drop 自动释放；
  容量治理只淘汰 `leased == 0 && !advancing && !saving` 的局，因此不会把另一请求
  正在使用的局中途卸载。
- **恢复去重**：同一 game_id 只允许一个 `restore_from_disk` 任务；并发请求看到
  `restoring` 标记后短暂等待，避免两个任务同时消费同一个磁盘包（早期压测 404 根因）。
- **恢复中的局不淘汰**：restore 插入 map 与消费磁盘包之间也在 `restoring` 集合中。

## 3. 并发修复记录（容量压测暴露并修复）

| 症状 | 根因 | 修复 |
|---|---|---|
| 并发保存 500「模拟线程已退出」 | 请求拿到 Arc 后、开始操作前，另一请求按 LRU 把该局卸载 | 请求租约 `GameHandle` + 取租约失败自动重试 |
| 并发读档 404「游戏不存在」 | 两个请求同时恢复同一局，第二个把磁盘包读到一半时文件被第一个删除 | `restoring` 去重集合 |
| 恢复后立即 404（文件短暂消失） | 先摘 map 后写盘，`persist` 重写 tmp 期间最终文件不存在 | 先原子写盘、再摘 map；恢复后保留陈旧磁盘包，下次淘汰覆盖 |
| 断连后该局永久 409 | `saving` 原子位靠 handler 正常路径释放，future 被 drop 不执行 | `SaveGuard` RAII 释放 |

## 4. 实测：10 × 20 年 128 队并发压测

> 工具：`scripts/capacity_stress.mjs`；结果存档：`scripts/data/capacity_stress_10x20y.json`。
> 流程：建 10 局（Auto）→ 逐局推进 240 个月 → 10 局并发 `/save/gzip` →
> 10 局并发 `/load/gzip` → 并发 `/view` → 等待后台收缩。

| 指标 | 结果 |
|---|---|
| 总墙钟 | 351.6s |
| 单局 20 年推进 | P50 26.6s / P95 26.9s |
| 并发保存（10 个 20 年档） | P50 58.0s / P95 58.3s / max 58.3s |
| 并发读档（10 个 20 年档） | P50 2.6s / P95 2.7s / max 2.7s |
| 并发 `/view` | P95 5ms |
| gzip 总字节 | 163.8MB（均值 16.4MB/档；JSON 均值 176.0MB） |
| CRC32 | 10/10 通过 |
| 活跃数回落 | 10 → 4，耗时 67.8s，磁盘保留 6 局 |
| 错误 | 0（无 404/409/500/超时） |

解读：
- 保存是 CPU 密集重操作（快照 clone + JSON 流式序列化 + gzip），10 并发 P50 58s；
  单局保存约 12.6s，未出现互相死锁，全部在 300s 超时内完成。
- 读档与轻量 `/view` 在并发下仍保持秒级/毫秒级，说明模拟线程与快照互斥设计生效。
- 后台收缩期间健康检查保持毫秒级响应（`spawn_rebalance` 幂等排发）。

## 5. 验收口径

- [x] 1/5/10 个 20 年档并存：`/view` P95 5ms，无 404/409/500。
- [x] 容量超限按 LRU 卸载，不删除；恢复后状态一致（单测 `capacity_model_six_games_keeps_four_active`、
      `concurrent_restore_of_same_game_never_404s`、`busy_burst_recovers_to_capacity_ceiling`）。
- [x] 保存任务期间单局 409，其他局不受影响；断连后 `SaveGuard` 自动释放互斥。
- [x] 存档失败率 0%（本压测 10/10 CRC 通过）。

## 6. 后续建议

- 后台收缩可加节流（当前健康检查触发即排发一次，`rebalancing` 位防止重复堆积）。
- 如需更激进的并发保存吞吐，可把「快照 clone + gzip」拆到专用压缩 worker + 限流
  （产品当前无此需求，58s/10 档仍在 300s 预算内）。
- 容量上限随档龄分桶：1/5 年档并发保存可在未来用同一脚本补测。
