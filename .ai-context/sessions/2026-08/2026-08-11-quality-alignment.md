# 2026-08-11 GPU 合成器质量对齐：corr 0.001 → 0.668

## 任务
对照 XSynth 参考输出修复 lumino-gpu-synth 的波形质量（爆音/无调子/时间轴错乱）。

## 关键决策与发现
1. **旧参考 `[Zackson_Y]Kentite.wav` 根本不是 right-example.mid 的渲染**（时长 196.671s vs xsynth 真实输出 194.82s）——此前所有对 Kentite 的 corr 对比（0.968 等）都是假象。
2. **用 xsynth render 重新生成参考**：`cargo run -p xsynth-render --release -- <midi> <sf2> -o ref_xsynth_default.wav -s 64000`（默认配置：layers=32、fade_out_killing=true、use_effects=true、attack Exp / decay+release Linear）。
3. **修复致命 bug：块间 GPU 状态错位**（readback 延迟一块 + 状态按数组 index 映射，retain 删除后错位）→ 每两块输出完全重复 → corr≈0。修复：states/out 都读回"本块"（放弃 pipelining，先正确后优化）+ 状态按 voice id 映射（`prev_voice_ids`）。
4. **note_off 语义**：xsynth 一次只释放"最老的一个 note 组"（FIFO + 同 note_id 的 zone voices 一起），原实现释放同 key 全部 voice → 新音符被旧 note_off 提前切掉。
5. **事件排序**：同 tick 事件必须保持 MIDI 原始顺序（轨道序），按 `(sample, channel)` 排序会破坏 note_on/off 配对顺序。
6. **steal 语义**：per-key 层数（默认 32，与参考一致）、杀最安静组（velocity 最小、整 note 组）、立即 ended（fade_out 1ms 曾导致 voice 池爆炸 112871，回退）。
7. **Q 值陷阱**：resonance_to_q 保持 `db_to_amp(db) × FRAC_1_SQRT_2`（与 xsynth SampleSoundfont 一致）；曾误改成"直接传 dB"导致 filter NaN（621k 个 NaN 样本）。
8. **velocity 调制、envelope 曲线、mix 音量平方、pan 等已全部确认与 xsynth 一致**（0-1s corr 0.9995、单音符 0.999996 验证）。

## 当前状态
- GLOBAL corr 0.668（0-1s 0.9995，大部分窗口 0.7-0.9，密集段 0.58-0.77）
- 单音符（97ms/12ms，use_effects=true）corr 0.999996 / 波形 diff <0.0004
- 渲染 81.9s（60s 限时未达标——states 同步读回 + 32 层 + HashMap 状态映射的开销）
- 剩余差异：密集段（2s 起）rms 比参考大 1.2-1.5 倍（生命周期/steal 细节），corr 0.58-0.77

## 待办
- 性能：恢复 readback pipelining（双缓冲时序修正）、状态映射 HashMap 改数组、`max_voices_per_key` 32 vs 用户要求的 4（需要与用户确认验收配置）
- 质量：密集段 +3dB rms 的剩余来源（疑似 steal 触发频率/释放时机的微差）

## 关键文件
- `src/synth/engine.rs`（readback/upload/sync/spawn/release 全部大改）
- `src/midi/parser.rs`（稳定排序）
- `src/synth/voices.rs`（note_id、vel 字段）
- `examples/diag_wav.rs`（通用双 wav 对比工具）、`examples/diag_single_note.rs`（单音符隔离实验）
- 参考：`assets/ref_xsynth_default.wav`、`assets/ref_default.wav`（生成中产物）、`[Zackson_Y]Kentite.orig.wav`（旧参考，被占用未删）
