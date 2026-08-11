# 任务：修复实时音频欠载（GPU 饥饿/浪费）+ 日志阻塞

## 文件
- `src/synth/engine.rs`：
  - `upload_voices` 去掉每块无条件 `render_bg_dirty = true`（原来每块重建 2 个 bind group）
  - `dispatch` 的 poll 加 100ms 超时 + readback recv 加超时（防 GPU 卡死无限等待）
- `src/audio/playback.rs`：stats 无锁化（Mutex<VecDeque> → AtomicU64 环形缓冲 `STATS_RING=128`）
- `examples/realtime_midi.rs`：block_size 512→2048；stats 打印加 stdout flush（print! 缓冲导致"瞬时打印很多后卡住"）

## 关键决策
### 欠账根因（用户：1000+ voice 时 Buffer 飞 -10W 回升不来）
1. **每块同步 poll 等 GPU**：`dispatch` 里 `poll(Wait, timeout:None)` 每块阻塞等当前块完成，CPU/GPU 完全串行。512 block 时每块固定开销（poll/map/encoder/submit/write）占比高，1000+ voice 的 GPU 执行 + CPU 组装串行 > 9.6ms 预算 → 欠账累积。
2. **每块重建 bind groups**：`upload_voices` 末尾无条件 dirty，dispatch 每块 `rebuild_bind_groups()` + `rebuild_mix_bind_group()`。bind group 只依赖 buffer 而非内容，应在 buffer 增长时（write 返回 true）才重建。
3. **block 太小**：512 frames @ 48k = 10.67ms/块，GPU dispatch 密集段 4-5ms 已占 50%，偶发峰值（缓存 miss/样本上传）即超预算且不补偿。

### 修复
- **block_size 512 → 2048**：预算 9.6ms → 38.4ms，固定开销摊薄 4 倍。实测 load 1.05 → **0.25**（dispatch ~5-8ms 仅占 25%），Buffer 稳定正数，underruns 密集段为 0。
- **去每块 bind group 重建**：写 buffer 只在增长时返回 true 触发 dirty。
- **stats 无锁化**：渲染线程实时禁锁（用户硬性要求），用 AtomicU64 环形缓冲发布 render load。
- **日志实时化**：`print!` 不 flush 时 `\r` 行累积到缓冲满一次性吐出（表现为瞬时大量打印后卡住），加 `stdout().flush()`。

### 剩余（评估后未做，风险高）
- dispatch 完整流水线（双份 out_storage_buf + 延迟一块读回）可让 CPU/GPU 完全重叠，但需改动 `out_storage_buf` 单缓冲架构且影响离线渲染块对齐。当前 load 0.25 余量充足，暂不需要。

## 验证
- right-example.mid 60s（含多次高潮段）：load 0.26，underruns 9（启动期），Buffer 短暂 -768 立即回正。
- C4-C5 全曲：load 0.21，underruns 2。
- 13+6 测试全过；clippy 仅预存在 diag 示例警告。

## 注意事项
- **实时渲染线程内禁止锁/长等待**：stats 已无锁；dispatch poll 加 100ms 超时兜底。
- block_size 会重编译 shader（BLOCK 常量），改 config 需重新初始化引擎。
- 2048 block 音频延迟 = 42.7ms，实时播放队列 16 块缓冲下无感。
