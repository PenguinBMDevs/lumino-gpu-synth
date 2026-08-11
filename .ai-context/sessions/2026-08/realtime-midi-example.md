# 任务：realtime example 直接播放用户指定 MIDI 文件

## 文件
- `examples/realtime_midi.rs`（新增，105 行）：播放用户指定 MIDI 的实时 example，可选 `[seconds]` 截断参数
- `src/audio/playback.rs`：新增 `engine_rate` 字段 + `engine_sample_rate()` 访问器

## 关键决策
- **实时调度器**：`MidiFile::load(path, sample_rate)` 得到 sample-accurate 事件流（`TimedEvent{sample, channel, event}`，sample 以引擎采样率为单位）。播放时用墙钟推进：`elapsed_frames = t0.elapsed() × engine_rate`，把 `sample <= elapsed_frames` 的事件逐个 `send_event` 发出，sleep(2ms) 轮询。这样事件按真实时间播放，多 channel 事件（含 CC/PC/PB）原样转发。
- **引擎采样率暴露**：调度器必须用引擎采样率（非设备采样率，重采样时两者不同），给 `AudioPlayback` 补了 `engine_rate` 字段和 `engine_sample_rate()`。
- **结尾清理**：循环结束后对所有 16 channel 发 `all_notes_off`，再等 600ms release 尾后 `stop()`。
- **可选 seconds 参数**：方便测试长文件（如 right-example.mid 426s 只需跑 8s 验证）。

## 依赖关系
- 复用 `MidiFile::load`（midi parser）、`AudioPlayback::send_event`、`engine_sample_rate()`。
- `SynthError::Config` 用于 usage 错误。

## 验证
- `cargo run --release --example realtime_midi -- assets/C4-C5.mid`：426 事件、6.5s 完整播放 ✅
- `-- assets/right-example.mid 8`：330 万事件文件前 8s，10971 事件按时间轴发送，无卡顿 ✅
- 13+6 测试全过；clippy 仅预存在 diag 示例 warning。

## 注意事项
- 引擎固定 48kHz + test.sf2（bank 0 preset 0）；换音色需改 example。
- `realtime_demo.rs`（内置旋律+API 演示）保留，两个 example 并存。
