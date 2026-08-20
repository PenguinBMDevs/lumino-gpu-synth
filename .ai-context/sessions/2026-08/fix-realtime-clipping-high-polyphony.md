# 任务：voice 800-1000 持续爆音——GPU 端深度排查（迭代 5）

## 用户反馈
"voice 达到 800-1000 以上时开始持续爆音。继续检查逻辑。"

## 排查链（逐层深挖，数据驱动）
1. **limiter 块级 gain 调制**（第一嫌疑）→ 复现：diag_crackle2 合成周转场景 jumps 74-76 → **重写 limiter**
2. **lookahead limiter 的 tail 语义错误**（两版迭代）：tail 保存"输出"→ 双重乘 gain + 边界跳变（frame=256 处 d=0.9-1.2 的 dump 实锤）→ tail 改存"输入样本"、gain 按输出时刻应用
3. **无 limiter 时 3 亿垃圾样本**（LUMINO_NO_LIMITER 诊断）→ 逐层排除：samples 上传/扩容清零 → voice_out clear 实验（渲染确实写出巨大）→ SAMPDUMP（shader 内 dump 计算链）
4. **根因 A：env_eval CONCAVE 溢出**——resume 状态 env_t=18176 > duration(1280) → prog=14.2 → `((1-14.2)²)⁴` = 9.2 亿 env → 单样本爆音。修复：**prog clamp 到 [0,1]**（peak 3 亿 → 40791）
5. **根因 B：GrowableBuffer::write 扩容未清零**——write 路径（samples chunk 用）扩容时只复制旧内容、新区域未初始化 → voice 播过样本末尾（SF2 sample_end > rendered length）读到固定垃圾 40788（确定性、单样本尖峰、位置漂移）。修复：**write 扩容清零新区域**（peak 40791 → 7.5，尖峰消失）
6. **根因 C：mix 的 release amp 语义错误**——查 XSynth 源码（apply_channel_effects 后处理 vol³ 覆盖全 channel buffer）→ release tail 必须乘 amp（原实现跳过 → voice release 瞬间跳变）。修复 mix.wgsl
7. **根因 D：trim_exclusive 硬杀**（ended=1 瞬间归零）→ 改 XSynth Kill 语义（1ms fade）
8. **防御修复**：with_max_capacity 初始清零、ensure 全量清零、out_storage/out_readback 清零（wgpu 不保证 0）

## 修复清单
- `src/synth/engine.rs`：apply_limiter 重写（lookahead 256 帧 = 4ms 延迟线 + 超前峰值 gain + soft_knee 兜底）、trim_exclusive fade、out buffer 清零
- `src/gpu/mod.rs`：GrowableBuffer with_max_capacity/ensure/write 全部清零
- `src/gpu/shaders/render.wgsl`：env_eval prog clamp [0,1]（CONCAVE/CONVEX/LERP 防溢出）
- `src/gpu/shaders/mix.wgsl`：release tail 应用 amp（对齐 XSynth 后处理语义）
- 新增 diag：diag_crackle、diag_crackle2、diag_midi_crackle（爆音检测：分级差分 + 块边界 + 位置）

## 最终数据（60 秒 right-example）
| 指标 | 修复前 | 修复后 |
|---|---|---|
| blockjump（块边界跳变） | 1021 | **0** |
| >1.0 差分跳变 | 378 | **24**（音乐瞬态） |
| >0.75 | 567 | 161 |
| 峰值（无 limiter） | 3.07 亿 | 7.5（正常求和） |
| 合成场景 >0.5 跳变 | 74-76 | **0** |
| 确定性 | 漂移（未定义行为） | 两次运行逐位一致 |
| c4c5 bit-identical | - | **0 差异**（baseline 已更新） |
| 性能 | - | 4096 voice 22ms < 32ms 预算 |

## 踩坑
- SAMPDUMP/MIXDUMP 等 shader debug 会影响时序 → 未定义行为位置漂移（40788 出现位置不定）——必须用"确定性数据"（值、位模式）判断
- tail 语义反复（输出 vs 输入样本）——延迟线必须存输入，gain 按输出时刻应用
- env_eval 的 clamp 不能只修 CONCAVE（LERP 同样溢出）

## 遗留
- 真实黑 MIDI 的 >0.5 差分（517 次/60s）为音乐瞬态（密集 note attack/release），XSynth 同样存在
- 20 万+ NPS 的 note < 1 帧物理不可闻（非 bug）
