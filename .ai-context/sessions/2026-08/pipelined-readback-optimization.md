# 2026-08-12 流水线读回优化：精度零损失 + 实时水平反超 CPU 版

## 任务
恢复 GPU 流水线读回（消除每块同步 poll 等待），严守精度红线（输出必须与同步版逐字节一致），并解决"GPU 实时播放连 xsynth-normal CPU 都不如"的性能/指标问题。

## 关键决策
1. **恢复 depth-1 流水线读回**（dispatch 后不等待，下一块 `collect_pending_readback` 读回），新增 `pending: Option<PendingReadback>` 记录精确的 idx/out_slot/states_slot（替代易错位的 prev_submission 猜测）。
2. **静音 fast path 语义**：不 dispatch 的块仍消费 pending（输出上一块音频），`last_states`/`prev_voice_ids` 保留（GPU 状态未动，仍有效）。
3. **`sync_voice_states` 修复（根因）**：移到 collect 之后、upload 之前（用最新读回更新 CPU mirror）；**不再 `take(prev_voice_ids)`**——旧代码 take 导致下一块 upload 的 resumed 匹配失效 → fallback 旧 v.state → 状态链滞后一块（音频重叠+提前结束，corr 0.03）。
4. **render_midi_inner 对齐**：Phase 1 首块输出（块 -1 伪音频）丢弃；追加必须在 max_frames 检查之前（输出延迟一块的语义）。
5. **shader SEGS 对齐**：`RENDER_SEGMENTS=16` 与 shader `SEGS=1` 不匹配（16 个 workgroup 仅 1 个干活）；SEGS 改为 CPU 注入，并修复 filtered voice 单段逻辑（seg=0 渲染整块）。
6. **负载指标修复**：playback.rs 删掉 try_send 队列等待后重复 push 的 2 个 render-load 采样——队列背压等待被误计为渲染负载（显示 69%，实际 3%）。
7. **实时块改 2048（可 512）**：8192 块 170ms 首音延迟 → 2048 块 42.7ms / 512 块 10.7ms，负载 3-6% 仍健康。

## 关键性能数据
- 离线：514048 帧渲染 1.28s（6.28x realtime）vs xsynth CPU 版 2.02s（4.0x realtime）；同步基线 1.51s。
- 离线精度：与基线 wav **逐字节一致（0 diff / 4112384）**，corr 0.977172 完全保持。
- GPU 固定成本：每个 GPU 命令 ~0.25-0.5ms（belt 的 4 个 copy 是最大项）；poll(Wait) 本身 2us。
- 实时：block 512 @48k，负载 3%、播放期 0 underruns、队列缓冲 ~33k 样本（688ms，防 underrun cushion）。

## 注意事项
- 离线渲染不能用大块（block 8192 时 corr 0.9739 与基线有差异——块边界对齐问题，触及精度红线）。
- 队列满（32 块）时 try_send 阻塞是设计行为（背压节流），不算负载。
- 16 段并行对低 polyphony（8 voices）无提升（GPU 时间与计算量无关），但对 dense MIDI 有理论收益，保留。
- 首次块 dispatch 冷启动 ~26ms（管线编译），warm_gpu 已处理。

## 依赖关系
- engine.rs：`render_block`（collect 前移+sync 前移）、`dispatch`（pending 记录）、`collect_pending_readback`（新增）、`render_midi_inner`（first_block+追加顺序）、`sync_voice_states`（不 take）。
- gpu/mod.rs：`RENDER_SEGMENTS=16` + SEGS define 注入；shaders/render.wgsl：`SEGS=16` + filtered 单段逻辑。
- audio/playback.rs：删重复 load push。
- examples/realtime_midi.rs：block 2048（LUMINO_RT_BLOCK 可调）。
- 验证工具：examples/cmp_wav.rs、render-output-baseline.wav（同步版基准）。

---

# 2026-08-12(续) 解析器内存爆炸修复 + Rekt Apple 验收

## 任务
用户换测试 MIDI 为 `Rekt Apple!!.mid`（800MB / 2.016 亿事件）——旧解析器内存爆炸（峰值 10GB+）。要求：修好解析器；前 30 秒 render time 在 0.3-0.5 左右。

## 关键决策
1. **TimedEvent 16 → 8 字节打包**：`u32 sample + u32 packed（channel 4bit | kind 4bit | payload 24bit）`——2 亿事件 3.2GB → 1.6GB。
2. **解析器消除中间结构**：旧代码先收集 `raw_events: Vec<(u64,u8,MidiEvent)>`（16B/事件）再转换——峰值双份 6.4GB。新代码 pass1 收集 tempo map、pass2 直接构建 packed 事件（单份 8B）。
3. **prewarm 位图去重**：2 亿事件遍历 × zones_at（每事件！）×2 次遍历（wanted + templates）= 90.9s 启动。改为 (key,vel) 位图（128×128）跳过重复 + 单次遍历收集样本、templates 用 (ch,key,vel) 位图（16×128×128）——启动 90.9s → 29.2s（剩余为 midly 解析 800MB 固有成本）。
4. **解析器正确性**：`parser_cmp` 对 right-example 330 万事件逐事件对比旧逻辑 = **0 mismatch**（打包/解包/时间戳/排序全一致）。
5. **启动尖峰**：预填充队列 3 → 8 块静音（覆盖首块 dense 渲染 + 追赶）——max load 1.03 → 0.59，underruns 归零。

## 验收数据（Rekt Apple!!.mid，30s 实时）
- prewarm：29.2s（旧 90.9s）
- render time：开头 0.2s 尖峰（0.65/0.41）→ 之后全程 ≤0.3（avg 0.147、p90 0.21）
- **underruns：0（全程无卡顿）**；Voice 2048 满池稳定
- 内存：2 亿事件 8B/事件 = 1.6GB（旧 3.2GB+ 中间结构）

## 注意事项
- `ev.sample` 变 u32（覆盖 18.6h @64kHz）；engine 消费点 `ev.sample as u64`。
- `event()` 每事件解包（~ns 级）；`MidiEvent` 枚举保留作视图。
- right-example 与旧代码输出有微小差异（p50 0.009）+ 8.38s 处两版都有爆炸值（MIDI 本身病态，非解析器）；差异来自优化行为（延迟 steal/trim 时序），解析器 100% 一致。
- `active_notes` 计数下溢 bug 曾致音符挂起（渲染帧数 +82s）——trim 用重建计数而非递减修复。
