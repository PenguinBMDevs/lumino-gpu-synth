# 任务：修复超高瞬时发音数时实时播放严重爆音（数字削波）—— 迭代 2：limiter 方案

## 文件
- `src/synth/engine.rs`：新增 `limiter_gain` 状态 + `apply_limiter()`（块级峰值限制器，接入 readback 与 fast-path 两条输出路径）
- `src/gpu/shaders/mix.wgsl`：soft_clip 已移除，仅保留注释说明（mix 输出原始累加值，f32 无损）
- `examples/diag_peak.rs` / `diag_peakshape.rs` / `diag_wavpeak.rs` / `diag_wavloc.rs` / `diag_bitcmp.rs`（新增）：削波回归验证工具链

## 迭代 1 失败复盘（用户：全程爆音）
第一次修复在 mix.wgsl 加 soft clip（knee=1.0，渐近 1.05），峰值验证通过（710→1.05）但**听感更糟**：
- 无记忆非线性数学无解：输入 1~700 只能输出 1.0~1.05 空间 → 必然平顶方波化
- 实测（diag_peakshape）：256 voice 时 53%、4096 voice 时 88% 的样本处于峰值的 90% 以上（正弦仅 3.6%）→ "全程爆音"
- **自我批判**：只验证了"峰值被压住"（假指标），没验证"波形保真"（真目标）

## 迭代 2：CPU 块级峰值限制器（最终方案）
- **标量增益缩放整块**：波形保真（不改变形状），峰值精确压到 0.98
- **Attack 立即生效**：gain 由本块峰值决定（readback 时数据在手），整块从第一个样本就缩放 → 无攻击滞后窗口
- **Release 指数恢复**（τ=50ms）：音量回归不 pumping、块边界无 click
- **正常块完全透明**：峰值 <0.98 时 target=1.0 → bit-identical 保持（c4c5_t2.mid 与 baseline 1028096 样本逐位一致）
- 应用位置：`readback()` + fast-path（`voices.is_empty()` 重放 last_out）两条输出路径

## 踩坑记录
1. **soft clip 在 shader 先执行会先方波化**：即使后续 CPU limiter 把 1.05 缩放到 0.98，平顶形状无法恢复 → soft clip 必须整体移除
2. **块内指数 attack 有 1ms 滞后窗口**：gain 从上一块终值（1.0）指数下降期间，超载块开头直接输出原始巨大值（实测 -4.7e9 泄漏）→ attack 必须整块恒定 = 本块 target
3. **块间 gain 跳变**：超载块（gain 2e-10）与安静块（gain 1.0）交界 → 由 release 平滑吸收；超载 onset 处的跳变被信号本身的开场瞬态掩盖

## 验证（全部数据闭环）
| 场景 | 修复前 | 修复后 |
|---|---|---|
| 64~12288 voice 峰值 | 10~710 | **0.980**（over1.0=0） |
| 方波化（over90pctOfPeak） | 53%~88% | **0.1%**（动态正常 p/r=3.8） |
| right-example.mid 427s 全曲 | peak=1.9e12 / RMS=4.3e8 | **peak=0.980 / RMS=0.239 / over=0** |
| c4c5_t2.mid vs baseline | — | **bit-identical**（1028096 样本 0 差异） |
| cargo test | — | 13 + 6 doc 全过 |

## 注意事项
- 超载块的音量被压低（RMS 0.24）是**保真优先**的取舍：波形无失真、无爆音，release 后音量回归
- limiter 状态在 engine 实例内（limiter_gain 初始 1.0）；`render_midi_inner` 重置 last_out 等字段时无需重置 limiter（跨文件渲染延续合理）
- 若用户后续希望"超载时更响"，方向是 voice 数感知归一化（1/√N，保持 RMS 恒定），但会改变与 XSynth 参考的对比，需单独评估
- NaN/Inf 防护：峰值扫描跳过非有限值，防止病态 voice 毒化 limiter 成静音
