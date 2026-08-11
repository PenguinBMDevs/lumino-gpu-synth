# 任务：realtime 信息输出对齐 xsynth + 爆音修复

## 文件
- `src/audio/playback.rs`：渲染线程重构（持续渲染 + 节拍 + 背压 + 统计）+ `PlaybackStatsReader`
- `src/synth/engine.rs`：新增 `prewarm_midi_file` 公开 API
- `examples/realtime_midi.rs`：加实时统计线程 + 播放前 prewarm

## 关键决策
### 架构对齐 xsynth BufferedRenderer
- **持续渲染**：渲染线程不再"事件驱动 + try_send 丢块"，改为持续渲染，节拍 = 块实时时长 × 90%（比实时略快，能吸收慢块）。
- **背压**：`samples > last_requested * 110%` 时 sleep（领先 10% 就休息），**绝不丢块**（队列满则等待消费者）。
- **统计**（镜像 xsynth）：`voice_count / samples / last_samples_after_read / average_renderer_load / underruns`，`PlaybackStatsReader` 供 example 每 100ms 打印。

### 爆音根因（两个叠加）
1. **节拍预算算错**：`delay = 1s × block_len / rate × 90%` 用了 `block*channels`(1024) 而非 `block`(512)，预算翻倍成 19.2ms → 渲染线程产能仅 53K/s < 设备消费 96K/s → Buffer 持续负增长、underrun 1606 次。修复为 `block / rate × 0.9`。
2. **realtime 无 pre-warm**：首次遇到大样本要 CPU 重采样 + GPU 上传，单块可达 297ms（profile 显示 samples=297599us/dispatch=43526us）→ 队列瞬间掏空 → 密集段爆音。新增 `prewarm_midi_file`，播放前并行预重采样+上传所有会用样本。

### 修复后指标（right-example.mid 密集段 30s）
- Render load: 1.05 → **0.46**（dispatch 稳定 2-3ms，预算 9.6ms）
- Buffer: -242K（欠账）→ **+320~960（稳定供应）**
- underruns: 269 → **14**（均为启动期，窗口内 0）

## 依赖关系
- `prewarm_midi_file` 复用 offline 渲染的 pre-warm 逻辑（resample_uncached 并行 + write_samples + cache_resampled）。
- 统计计数：渲染线程 `samples += out.len()`，回调 `samples -= data.len()` + underruns++。

## 注意事项
- WASAPI 回调禁止阻塞——underrun 时写静音并计数（不 try_recv 阻塞）。
- 停止/崩溃路径：渲染线程等待队列/节拍时检查 stop_flag，`stop()` 不会死锁。
- 剩余 underrun 均为启动前几块（队列未满），可接受。
