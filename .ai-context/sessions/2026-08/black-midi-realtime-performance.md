# 任务：黑乐谱 realtime 性能（GPU 饥饿修复 + 事件流架构）

## 文件
- `src/synth/engine.rs`：全局 voice cap（每块一次）、`prewarm_midi_file` 加 `warm_gpu`、`set_events`/`stream_exhausted`、`StagingBelt` 上传、upload 延迟到 dispatch 合并 submit、dispatch 读回 map/unmap 严格配对
- `src/audio/playback.rs`：`play_events`（事件流模式）、`thread_running`、stats 无锁环形缓冲、underrun 计数修正（只减实际消费）、cushion 节拍
- `src/gpu/mod.rs`：`RENDER_SEGMENTS` 1→16
- `examples/realtime_midi.rs`：block 2048、max_voices 512、`play_events` 替代逐事件注入

## 关键决策
### 卡死根因（用户观察 GPU 0% + CPU 8%）
1. **poll 超时死循环**：`dispatch` poll 设 100ms 超时，黑乐谱密集块偶尔超时 → return Err 但 buffer 已 map 未 unmap → 下块 map 同 buffer → wgpu validation 崩溃（"still mapped" panic）→ 渲染线程死，GPU 0%。修复：**poll 无超时（正确性硬依赖）+ map/unmap 严格配对**（回调触发才 unmap，poll 超时=未 map=安全）。
2. **事件注入通道瓶颈**：每秒 17 万事件走 mpsc channel 逐条 send → 积压。修复：`play_events` 直接把整个事件流 `set_events` 到引擎内部，按 `global_frame` 推进（零 channel 流量）。

### 性能优化
- **全局 voice cap**：黑乐谱 8192+ voice 撑爆 GPU。改为**每块一次**（upload_voices 开头），杀最安静 note 组到 max_voices，O(n) 而非每 note 排序。
- **StagingBelt**：`queue.write_buffer` 每块分配 staging（测 35ms/块）→ 持久 belt 复用。
- **upload 延迟到 dispatch**：4 次 write 合并进 dispatch 的**一次 submit**（wgpu submit 有 ~9ms 固定开销，减少次数）。
- **load 定义修正**：原 load 含 cadence sleep（恒 ≥1）。改为只统计 render_block+resample 实际时间。

### 测量关键数据
- `dispatch: submit=9-92ms, poll=89-416us`——瓶颈是 **submit 同步**（wgpu Vulkan 后端），不是 GPU 计算。
- 黑乐谱 2048 block + 512 voice：load 1.17（真实渲染 44ms/块），**播放流畅 underrun 0**。
- right-example：load 0.24，underrun 0。

## 验证
- 黑乐谱 15s：Buffer 稳定正数（~10K），underrun 窗口全 0，Voice Count 512（cap 生效）。
- right-example 5s：load 0.24，underrun 0。
- 13+6 测试全过。

## 注意事项
- **STATES_SYNC_EVERY 保持 1**：提高会让 voice 从旧状态 resume（音频重放风险），用户禁止损失精度。
- load ≤ 0.05 目标受 wgpu submit 固定开销（~9ms/块）限制，需真正 CPU/GPU 流水线（双 out_storage + 延迟一块 readback）才能达成——未实现（高风险，涉及 bind group 轮换 + offline 渲染兼容）。
- RENDER_SEGMENTS=16 已启用但 GPU 并行度不是当前瓶颈。
