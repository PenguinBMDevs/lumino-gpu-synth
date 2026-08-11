# 任务：修复密集单 key 漏音 + 发声时长不足 + render 进度条

## 文件
- `src/synth/engine.rs`：`spawn_voices` 重构（steal 逻辑从 zone 循环内移到整个 note 级别）、`release_key` 不变、新增 `ProgressBar`
- `src/config.rs`：新增 `SynthConfig.show_progress` 字段
- `examples/render_example.rs`：`show_progress: true`
- 新增 `examples/diag_dense.rs` / `diag_dense2.rs` / `diag_dense3.rs`：密集音符压力测试

## 关键决策
- **漏音根因**：`spawn_voices` 里 XSynth 风格 per-key steal 逻辑跑在 `for zone_id in zone_ids` 循环**内部**，立体声 note 有 2 个 zone，第二个 zone 的 steal 会把第一个 zone 刚 push 的 voice 计入 candidates 并杀掉——自己杀自己 → 密集单 key 高频段漏音/时长被截断。修复：先 build 所有 zone voices，steal/exclusive-class 只在 note 级执行一次。
- **时长不足根因**：同一 bug——voice 被自己 note 的第二个 zone steal 杀（`ended=1` 立即结束），release 尾音消失。修复后 diag_dense3 200 个密集 note 全部发声且完整。
- **stale voice 处理**：steal 计数时先免费释放已 `ended` 的 voice（`stale`），quota 只计 audible voice。
- **进度条**：`ProgressBar` 单行 `\r` 重写，对齐 `max_frames` 渲染地平线；`show_progress` 默认 false 保持库调用安静。
- **对齐 XSynth 语义确认**：`fade_out_killing=false` → XSynth steal 直接 kill（无 fade），与我们的 `ended=1` 一致。

## 依赖关系
- 与 `build_voice`（zone → Voice）、`key_voices` per-key 索引、`voice_id_counter`、`upload_voices`（retain ended）交互。
- `SynthConfig::validate` 未涉及新字段。

## 验证
- `cargo test`：9+6 passed（原有）。clippy lib 仅 1 个预存在 type_complexity 警告（`debug_voices`）。
- diag_dense：key 36/60/88 × 40 notes 全 PASS（无漏音、无短音）。
- diag_dense2：24 key × 30 reps = 720 notes 叠加，silent=0 truncated=0。
- diag_dense3：单 key 200 次超高密度，200/200 发声、零截断。
- 进度条：30s right-example 渲染显示 `[render] [==...] 98%→100%` 正常。

## 注意事项
- 时间轴问题已定位但**未修复**：`ref_xsynth_default.wav`（194.8s）是用**旧版 MIDI**（`right-example.mid.old`）渲染的；新版 `right-example.mid`（427s）第一个音符在 2.0s，参考在 0.47s——0-1s 静音本质是**两个不同文件的时间轴差异**，不是渲染 bug。用户接受该间隔。
- `engine.rs` 已 1865 行（超 400 行限制），拆分是独立重构任务。
