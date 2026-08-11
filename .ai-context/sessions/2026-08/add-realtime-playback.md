# 任务：加入 realtime 管线和公开调用的 API（cpal 实时播放）

## 文件
- `src/audio/playback.rs`：重写 `AudioPlayback`（338 行）——采样率协商、事件/渲染双线程、非阻塞 push 防死锁、补全公开 API
- `src/audio/resample.rs`（新增，136 行）：`LinearResampler` 引擎→设备采样率转换 + 4 单元测试
- `src/audio/mod.rs`：注册 `resample` 模块
- `src/config.rs`：`ChannelMode::channel_count()` 辅助方法
- `src/lib.rs`：顶层 re-export `AudioPlayback`
- `examples/realtime_demo.rs`（新增）：演示全部实时 API 的播放 demo

## 关键决策
- **采样率协商（WASAPI 硬坑）**：设备 `supported_output_configs()` 枚举的采样率是"宽泛承诺"，WASAPI shared mode 下用非默认采样率开流会直接报 `"Stream configuration is not supported in shared mode"`。正确策略：**始终用设备默认配置**（保证能开流），引擎采样率不同则线性重采样。实测 64k 引擎 → 48k 设备播放正常。
- **防死锁**：渲染线程 `sample_tx.try_send()` 非阻塞，队列满就 sleep 跳过（替代原 `send()` 阻塞——那个会让 `stop()` 的 join 永久卡死）。
- **线程结构**：cpal 音频回调（OS 音频线程，只拷数据不阻塞）+ 渲染线程（拥有 GpuSynth，`recv_timeout(5ms)` 批量收事件 + render_block + push）。`AtomicBool stop_flag` + stop channel 双保险。
- **公开 API**：`note_on/note_off/control_change/program_change/pitch_bend/control_change_14bit/damper/all_notes_off/all_sounds_off/reset_controllers/sample_rate/device_sample_rates/stop`。
- **重采样器**：线性插值 + 保留上块末尾样本跨块连续；输出长度 = round(input × ratio)。

## 依赖关系
- `AudioPlayback` 拥有 `GpuSynth`（渲染线程独占），事件经 mpsc channel 注入。
- `MidiEvent` 复用 midi 模块；`ChannelMode` 新增 `channel_count()`。
- cpal 0.18.1：`SampleRate = u32` 类型别名（非 newtype），`with_sample_rate` 返回字段私有的 `SupportedStreamConfig`。

## 验证
- 13+6 测试全过（含 4 个新 resampler 测试：恒等/降采样长度/升采样/跨块连续）。
- clippy lib 仅 1 预存在 type_complexity 警告。
- realtime_demo 实测：48k 引擎（设备匹配）和 64k 引擎（强制重采样）两条路径均正常播放旋律，pitch_bend/damper/all_notes_off 全部生效。

## 注意事项
- `engine.rs` 仍 1865 行（超限，独立重构项）；`playback.rs` 拆分后 338 行合规。
- 引擎 `sample_rate` 建议按 `AudioPlayback::device_sample_rates()` 就近选择，减少重采样损耗；但任意采样率都能工作。
